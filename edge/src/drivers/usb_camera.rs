//! V4L2 USB camera driver.
//!
//! Each [`UsbCamera`] is opened once, then [`UsbCamera::run`] is called in a
//! dedicated `std::thread`. Frames land in a bounded crossbeam channel shared
//! with the writer thread.
//!
//! Format negotiation order: MJPG (camera produces JPEG, zero re-encode) →
//! YUYV (soft-encode to JPEG via the `image` crate).

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};
use v4l::{
    buffer::Type,
    io::{mmap::Stream, traits::CaptureStream},
    video::Capture,
    Device, FourCC,
};

use crate::{config::CameraConfig, time::ClockCalibration};

// ── Public frame type ────────────────────────────────────────────────────── //

/// A captured, JPEG-encoded camera frame.
#[derive(Debug)]
pub struct CameraFrame {
    pub camera_name: String,
    pub device_id: u32,
    /// CLOCK_TAI nanoseconds at the moment the frame was **captured by the
    /// kernel driver** (V4L2 buffer metadata timestamp, converted from
    /// CLOCK_MONOTONIC via [`ClockCalibration`]).
    ///
    /// This is set at frame-capture time, not at userspace dequeue time,
    /// eliminating USB-transfer and scheduling jitter.
    pub timestamp_ns: u64,
    pub frame_index: u64,
    pub width: u32,
    pub height: u32,
    /// Always `"jpeg"` — callers can assume JPEG bytes.
    pub encoding: &'static str,
    pub data: Vec<u8>,
}

// ── Camera handle ────────────────────────────────────────────────────────── //

pub struct UsbCamera {
    config: CameraConfig,
    device: Device,
    actual_width: u32,
    actual_height: u32,
    actual_fps: f64,
    is_mjpg: bool,
}

impl UsbCamera {
    /// Open `/dev/video<device_id>`, negotiate format and FPS.
    pub fn open(config: CameraConfig) -> Result<Self> {
        let dev = Device::new(config.device_id as usize).with_context(|| {
            format!(
                "Cannot open /dev/video{}. \
                 Check 'ls /dev/video*' and usbipd-win attachment on WSL.",
                config.device_id
            )
        })?;

        // ── Format negotiation ──────────────────────────────────────────── //

        // Start from current format so we only override what we care about.
        let mut fmt = dev.format().context("read current format")?;
        fmt.width = config.width;
        fmt.height = config.height;
        fmt.fourcc = FourCC::new(b"MJPG");

        let (fmt, is_mjpg) = match dev.set_format(&fmt) {
            Ok(f) if f.fourcc == FourCC::new(b"MJPG") => {
                info!(
                    camera = %config.name,
                    "Format: MJPG  {}×{}",
                    f.width,
                    f.height
                );
                (f, true)
            }
            _ => {
                // Try YUYV fallback
                fmt.fourcc = FourCC::new(b"YUYV");
                let f = dev.set_format(&fmt).with_context(|| {
                    format!(
                        "Camera '{}' supports neither MJPG nor YUYV",
                        config.name
                    )
                })?;
                info!(
                    camera = %config.name,
                    "Format: YUYV  {}×{}  (will re-encode to JPEG)",
                    f.width,
                    f.height
                );
                (f, false)
            }
        };

        // ── Frame-rate ──────────────────────────────────────────────────── //

        let actual_fps = match dev.params() {
            Ok(mut p) => {
                p.interval = v4l::Fraction::new(1, config.fps);
                let _ = dev.set_params(&p); // ignore if camera refuses
                dev.params()
                    .map(|p| {
                        if p.interval.numerator > 0 {
                            p.interval.denominator as f64
                                / p.interval.numerator as f64
                        } else {
                            config.fps as f64
                        }
                    })
                    .unwrap_or(config.fps as f64)
            }
            Err(_) => config.fps as f64,
        };

        info!(
            camera = %config.name,
            device  = config.device_id,
            width   = fmt.width,
            height  = fmt.height,
            fps     = actual_fps,
            "Camera ready"
        );

        Ok(Self {
            actual_width: fmt.width,
            actual_height: fmt.height,
            actual_fps,
            is_mjpg,
            device: dev,
            config,
        })
    }

    // ── Accessors (for session manifest) ─────────────────────────────────── //

    pub fn config(&self) -> &CameraConfig { &self.config }
    pub fn actual_width(&self) -> u32 { self.actual_width }
    pub fn actual_height(&self) -> u32 { self.actual_height }
    pub fn actual_fps(&self) -> f64 { self.actual_fps }

    // ── Capture loop (blocks until stop is set) ──────────────────────────── //

    /// Runs the V4L2 capture loop in the calling thread.
    ///
    /// Sends [`CameraFrame`]s via `tx`. Returns total captured frame count.
    /// Frames are silently dropped when the channel is full (back-pressure from
    /// a slow writer) — a warning is logged per dropped frame.
    ///
    /// `calibration` converts V4L2 CLOCK_MONOTONIC hardware timestamps to
    /// CLOCK_TAI. Pass the same `Arc<ClockCalibration>` to every camera so
    /// all streams share a single reference epoch.
    pub fn run(
        self,
        tx: crossbeam_channel::Sender<CameraFrame>,
        stop: Arc<AtomicBool>,
        calibration: Arc<ClockCalibration>,
    ) -> Result<u64> {
        let mut stream =
            Stream::with_buffers(&self.device, Type::VideoCapture, self.config.buffer_size)
                .context("create mmap stream")?;

        let name = self.config.name.clone();
        let device_id = self.config.device_id;
        let width = self.actual_width;
        let height = self.actual_height;
        let quality = self.config.jpeg_quality;
        let is_mjpg = self.is_mjpg;

        let mut frame_index: u64 = 0;
        let mut dropped: u64 = 0;

        while !stop.load(Ordering::Relaxed) {
            // stream.next() blocks until a frame is ready (typically < 33 ms).
            let (buf, meta) = match stream.next() {
                Ok(v) => v,
                Err(e) => {
                    warn!(camera = %name, "Frame read error: {e}");
                    continue;
                }
            };

            // ── Hardware timestamp (kernel driver, CLOCK_MONOTONIC) ────────
            // V4L2 sets this at the moment the frame DMA completes — before
            // the buffer is dequeued to userspace.  This eliminates USB
            // transfer latency and thread-scheduling jitter that would affect
            // a userspace SystemTime::now() call.
            let hw_sec  = meta.timestamp.sec  as u64;
            let hw_usec = meta.timestamp.usec as u64;
            let hw_mono_ns = hw_sec * 1_000_000_000 + hw_usec * 1_000;

            // Convert CLOCK_MONOTONIC → CLOCK_TAI using the pre-measured
            // offset.  All cameras share the same CalibrationCalibration so
            // their timestamps are directly comparable.
            let timestamp_ns = calibration.mono_to_tai(hw_mono_ns);

            let data: Vec<u8> = if is_mjpg {
                buf.to_vec() // already JPEG
            } else {
                yuyv_to_jpeg(buf, width, height, quality)
            };

            let frame = CameraFrame {
                camera_name: name.clone(),
                device_id,
                timestamp_ns,
                frame_index,
                width,
                height,
                encoding: "jpeg",
                data,
            };

            match tx.try_send(frame) {
                Ok(()) => {}
                Err(_) => {
                    dropped += 1;
                    debug!(camera = %name, dropped, "Channel full — frame dropped");
                }
            }

            frame_index += 1;
        }

        info!(camera = %name, frames = frame_index, dropped, "Capture stopped");
        Ok(frame_index)
    }
}

// ── YUYV → JPEG helper ───────────────────────────────────────────────────── //

/// Convert a YUYV 4:2:2 buffer to JPEG bytes using the `image` crate.
///
/// YUYV layout: `[Y0 U0 Y1 V0]` per 2 pixels (4 bytes).
fn yuyv_to_jpeg(yuyv: &[u8], width: u32, height: u32, quality: u8) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut rgb = vec![0u8; pixel_count * 3];

    for i in 0..(pixel_count / 2) {
        let y0 = yuyv[i * 4] as f32;
        let u = yuyv[i * 4 + 1] as f32 - 128.0;
        let y1 = yuyv[i * 4 + 2] as f32;
        let v = yuyv[i * 4 + 3] as f32 - 128.0;

        let cvt = |y: f32| -> (u8, u8, u8) {
            let r = (y + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g = (y - 0.344_136 * u - 0.714_136 * v).clamp(0.0, 255.0) as u8;
            let b = (y + 1.772 * u).clamp(0.0, 255.0) as u8;
            (r, g, b)
        };

        let (r0, g0, b0) = cvt(y0);
        let (r1, g1, b1) = cvt(y1);

        rgb[i * 6] = r0;
        rgb[i * 6 + 1] = g0;
        rgb[i * 6 + 2] = b0;
        rgb[i * 6 + 3] = r1;
        rgb[i * 6 + 4] = g1;
        rgb[i * 6 + 5] = b1;
    }

    use image::{codecs::jpeg::JpegEncoder, ImageEncoder};
    let mut jpeg = Vec::new();
    if let Err(e) = JpegEncoder::new_with_quality(&mut jpeg, quality).write_image(
        &rgb,
        width,
        height,
        image::ExtendedColorType::Rgb8,
    ) {
        warn!("YUYV→JPEG encode failed: {e}");
    }
    jpeg
}
