//! `capture` binary — multi-camera USB capture agent.
//!
//! Usage:
//!   capture --config edge/configs/cameras.yaml --output data/raw [--duration 60]
//!           [--tag location=parking_lot] [--tag weather=sunny]
//!
//! Each camera runs in its own thread (blocking V4L2 stream.next()).
//! A single writer thread receives CameraFrames via a crossbeam channel and
//! writes them to one MCAP file per session.

mod config;
mod drivers;
mod recorder;
mod time;

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use clap::Parser;
use crossbeam_channel::RecvTimeoutError;
use tracing::{info, warn};

use drivers::usb_camera::UsbCamera;
use drivers::udp_capture::{UdpCapture, UdpCaptureConfig, UdpFrame};
use recorder::{
    mcap_writer::McapWriter,
    session::{SensorInfo, SessionManager},
};
use time::ClockCalibration;

// ── Unified message for the writer channel ───────────────────────────────── //

use drivers::usb_camera::CameraFrame;

/// Messages sent from capture threads (camera or UDP) to the writer loop.
enum CaptureMessage {
    Camera(CameraFrame),
    Udp(UdpFrame),
}

// ── CLI ───────────────────────────────────────────────────────────────────── //

/// Multi-camera USB capture → MCAP recording agent.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Path to cameras.yaml config file.
    #[arg(short, long, default_value = "edge/configs/cameras.yaml")]
    config: PathBuf,

    /// Root output directory (overrides config session.output_dir).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Stop after this many seconds (overrides config session.duration).
    #[arg(short, long)]
    duration: Option<f64>,

    /// Split MCAP every N seconds (overrides config session.segment_duration).
    #[arg(short, long)]
    segment_duration: Option<f64>,

    /// Extra metadata tags burned into the session manifest (key=value).
    #[arg(short, long = "tag", value_name = "KEY=VALUE")]
    tags: Vec<String>,
}

// ── Entry point ───────────────────────────────────────────────────────────── //

fn main() -> Result<()> {
    // Structured logging: RUST_LOG=debug for verbose output.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();

    // ── Clock calibration (PTP / CLOCK_TAI) ──────────────────────────────── //
    // Must happen before opening cameras so every thread shares the same
    // MONO→TAI reference epoch.
    let calibration = std::sync::Arc::new(ClockCalibration::measure());

    // ── Load config ─────────────────────────────────────────────────────── //

    let cfg = config::load(&args.config)?;

    // Merge CLI tags on top of YAML tags (CLI wins on collision).
    let mut tags = cfg.session.tags.clone();
    for kv in &args.tags {
        let (k, v) = kv.split_once('=').unwrap_or((kv.as_str(), ""));
        tags.insert(k.to_string(), v.to_string());
    }

    // ── Session directory ────────────────────────────────────────────────── //

    let output_dir = args
        .output
        .or(cfg.session.output_dir.clone())
        .unwrap_or_else(|| PathBuf::from("data/raw"));

    // Resolve recording duration: CLI > YAML > infinite (0).
    let duration_secs = args.duration.unwrap_or(cfg.session.duration);
    let duration_limit = if duration_secs > 0.0 { Some(duration_secs) } else { None };

    // Resolve segment duration: CLI > YAML > no splitting (0).
    let segment_secs = args.segment_duration.unwrap_or(cfg.session.segment_duration);
    let segment_limit = if segment_secs > 0.0 { Some(segment_secs) } else { None };

    let session = SessionManager::new(output_dir, tags);
    session.create_dir()?;

    let mcap_path = if segment_limit.is_some() {
        session.session_dir().join("recording_0000.mcap")
    } else {
        session.session_dir().join("recording.mcap")
    };

    info!(session_id = %session.session_id, path = %mcap_path.display(), "Session started");

    // ── Open cameras ─────────────────────────────────────────────────────── //

    let mut camera_handles = Vec::new();
    let mut aux_handles: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let mut sensor_infos: Vec<SensorInfo> = Vec::new();

    // Bounded channel: at most (8 × (camera + udp stream) count) messages queued.
    let total_sources = cfg.cameras.len().max(1) + cfg.udp_streams.len();
    let channel_cap = total_sources * 8;
    let (tx, rx) = crossbeam_channel::bounded::<CaptureMessage>(channel_cap);

    let stop = Arc::new(AtomicBool::new(false));

    // ── MCAP writer ───────────────────────────────────────────────────────── //

    let mut writer = McapWriter::new(&mcap_path)?;

    for cam_cfg in &cfg.cameras {
        match UsbCamera::open(cam_cfg.clone()) {
            Ok(cam) => {
                writer.register_camera(&cam_cfg.name, calibration.clock_label)?;
                sensor_infos.push(SensorInfo {
                    name: cam.config().name.clone(),
                    device_id: cam.config().device_id,
                    width: cam.actual_width(),
                    height: cam.actual_height(),
                    configured_fps: cam.config().fps,
                    actual_fps: cam.actual_fps(),
                });

                // Camera thread sends CameraFrame wrapped in CaptureMessage.
                let (cam_tx, cam_rx) = crossbeam_channel::bounded::<CameraFrame>(8);
                let tx2 = tx.clone();
                let stop2 = Arc::clone(&stop);
                let cal2  = Arc::clone(&calibration);

                // Spawm camera capture thread.
                let cam_name = cam.config().name.clone();
                let cam_handle = std::thread::Builder::new()
                    .name(format!("cam-{}", cam_name))
                    .spawn(move || cam.run(cam_tx, stop2, cal2))?;
                camera_handles.push(cam_handle);

                // Bridge thread: forward CameraFrame → CaptureMessage::Camera.
                let stop3 = Arc::clone(&stop);
                let bridge_handle = std::thread::Builder::new()
                    .name(format!("bridge-cam-{}", cam_name))
                    .spawn(move || {
                        while !stop3.load(Ordering::Relaxed) {
                            match cam_rx.recv_timeout(Duration::from_millis(100)) {
                                Ok(frame) => {
                                    let _ = tx2.try_send(CaptureMessage::Camera(frame));
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                            }
                        }
                    })?;
                aux_handles.push(bridge_handle);
            }
            Err(e) => {
                warn!("Skipping camera '{}': {:#}", cam_cfg.name, e);
            }
        }
    }

    // ── Open UDP streams ──────────────────────────────────────────────────── //

    let mut udp_handles = Vec::new();

    for udp_cfg in &cfg.udp_streams {
        let capture_cfg = UdpCaptureConfig {
            name: udp_cfg.name.clone(),
            bind_addr: format!("{}:{}", udp_cfg.bind_ip, udp_cfg.port),
            multicast_group: udp_cfg.multicast_group.clone(),
            interface: None,
        };

        match UdpCapture::open(capture_cfg) {
            Ok(capture) => {
                writer.register_udp_stream(&udp_cfg.name)?;

                let tx2 = tx.clone();
                let stop2 = Arc::clone(&stop);
                let stream_name = udp_cfg.name.clone();

                let handle = std::thread::Builder::new()
                    .name(format!("udp-{}", stream_name))
                    .spawn(move || {
                        // Create a local channel for the capture, then forward.
                        let (udp_tx, udp_rx) = crossbeam_channel::bounded::<UdpFrame>(64);

                        // Spawn inner capture thread.
                        let stop3 = Arc::clone(&stop2);
                        let inner = std::thread::Builder::new()
                            .name(format!("udp-rx-{}", stream_name))
                            .spawn(move || capture.run_to_channel(udp_tx, stop3))
                            .expect("spawn udp rx thread");

                        // Forward loop.
                        while !stop2.load(Ordering::Relaxed) {
                            match udp_rx.recv_timeout(Duration::from_millis(100)) {
                                Ok(frame) => {
                                    let _ = tx2.try_send(CaptureMessage::Udp(frame));
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        let _ = inner.join();
                    })?;
                udp_handles.push(handle);

                info!(name = %udp_cfg.name, port = udp_cfg.port, "UDP stream capture started");
            }
            Err(e) => {
                warn!("Skipping UDP stream '{}': {:#}", udp_cfg.name, e);
            }
        }
    }

    // Drop the main-thread sender: when all capture threads exit and drop their
    // senders, `rx.recv()` will return `Disconnected`.
    drop(tx);

    if camera_handles.is_empty() && udp_handles.is_empty() {
        anyhow::bail!("No cameras or UDP streams could be opened. Exiting.");
    }

    info!(
        cameras = camera_handles.len(),
        udp_streams = udp_handles.len(),
        "Recording started — press Ctrl-C to stop"
    );

    // ── Signal handling ───────────────────────────────────────────────────── //

    let stop_signal = Arc::clone(&stop);
    ctrlc::set_handler(move || {
        info!("Signal received — stopping capture…");
        stop_signal.store(true, Ordering::Relaxed);
    })?;

    // ── Writer loop ───────────────────────────────────────────────────────── //

    let t_start = Instant::now();
    let mut total_frames: u64 = 0;
    let mut segment_index: u32 = 0;
    let mut segment_start = Instant::now();

    loop {
        // Duration limit check
        if let Some(dur) = duration_limit {
            if t_start.elapsed().as_secs_f64() >= dur {
                info!("Duration limit reached ({dur:.1}s)");
                stop.store(true, Ordering::Relaxed);
                break;
            }
        }

        // Segment splitting: close current MCAP and open a new one.
        if let Some(seg_dur) = segment_limit {
            if segment_start.elapsed().as_secs_f64() >= seg_dur {
                writer.close()?;
                segment_index += 1;
                let new_path = session
                    .session_dir()
                    .join(format!("recording_{:04}.mcap", segment_index));
                writer = McapWriter::new(&new_path)?;
                // Re-register channels in the new segment file.
                for cam_cfg in &cfg.cameras {
                    let _ = writer.register_camera(&cam_cfg.name, calibration.clock_label);
                }
                for udp_cfg in &cfg.udp_streams {
                    let _ = writer.register_udp_stream(&udp_cfg.name);
                }
                info!(segment = segment_index, path = %new_path.display(), "New MCAP segment");
                segment_start = Instant::now();
            }
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(msg) => {
                match msg {
                    CaptureMessage::Camera(frame) => {
                        writer.write_frame(&frame)?;
                    }
                    CaptureMessage::Udp(frame) => {
                        writer.write_udp_frame(&frame)?;
                    }
                }
                total_frames += 1;
            }
            Err(RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // All capture threads exited and dropped their senders.
                break;
            }
        }
    }

    // ── Teardown ──────────────────────────────────────────────────────────── //

    // Wait for all camera threads to finish.
    for handle in camera_handles {
        let _ = handle.join();
    }

    // Wait for all auxiliary (bridge) threads to finish.
    for handle in aux_handles {
        let _ = handle.join();
    }

    // Wait for all UDP threads to finish.
    for handle in udp_handles {
        let _ = handle.join();
    }

    // Drain any messages still in the channel.
    while let Ok(msg) = rx.try_recv() {
        match msg {
            CaptureMessage::Camera(frame) => {
                writer.write_frame(&frame)?;
            }
            CaptureMessage::Udp(frame) => {
                writer.write_udp_frame(&frame)?;
            }
        }
        total_frames += 1;
    }

    let elapsed = t_start.elapsed();
    let fps_avg = if elapsed.as_secs_f64() > 0.0 {
        total_frames as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };

    info!(
        total_frames,
        elapsed_s  = format!("{:.1}", elapsed.as_secs_f64()),
        fps_avg    = format!("{fps_avg:.1}"),
        "Capture finished"
    );

    let total_frame_bytes = writer.total_frame_bytes();
    writer.close()?;

    session.finalize(
        &mcap_path,
        total_frames,
        total_frame_bytes,
        sensor_infos,
        calibration.clock_label,
        calibration.ptp_synced,
        calibration.mono_to_tai_offset_ns,
    )?;

    Ok(())
}
