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
use recorder::{
    mcap_writer::McapWriter,
    session::{SensorInfo, SessionManager},
};

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

    /// Stop after this many seconds (omit to run until Ctrl-C).
    #[arg(short, long)]
    duration: Option<f64>,

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

    let session = SessionManager::new(output_dir, tags);
    session.create_dir()?;

    let mcap_path = session.session_dir().join("recording.mcap");

    info!(session_id = %session.session_id, path = %mcap_path.display(), "Session started");

    // ── Open cameras ─────────────────────────────────────────────────────── //

    let mut camera_handles = Vec::new();
    let mut sensor_infos: Vec<SensorInfo> = Vec::new();

    // Bounded channel: at most (8 × camera count) frames queued.
    let channel_cap = cfg.cameras.len().max(1) * 8;
    let (tx, rx) = crossbeam_channel::bounded(channel_cap);

    let stop = Arc::new(AtomicBool::new(false));

    // ── MCAP writer ───────────────────────────────────────────────────────── //

    let mut writer = McapWriter::new(&mcap_path)?;

    for cam_cfg in cfg.cameras {
        match UsbCamera::open(cam_cfg.clone()) {
            Ok(cam) => {
                writer.register_camera(&cam_cfg.name)?;
                sensor_infos.push(SensorInfo {
                    name: cam.config().name.clone(),
                    device_id: cam.config().device_id,
                    width: cam.actual_width(),
                    height: cam.actual_height(),
                    configured_fps: cam.config().fps,
                    actual_fps: cam.actual_fps(),
                });

                let tx2 = tx.clone();
                let stop2 = Arc::clone(&stop);
                let handle = std::thread::Builder::new()
                    .name(format!("cam-{}", cam.config().name))
                    .spawn(move || cam.run(tx2, stop2))?;
                camera_handles.push(handle);
            }
            Err(e) => {
                warn!("Skipping camera '{}': {:#}", cam_cfg.name, e);
            }
        }
    }

    // Drop the main-thread sender: when all camera threads exit and drop their
    // senders, `rx.recv()` will return `Disconnected`.
    drop(tx);

    if camera_handles.is_empty() {
        anyhow::bail!("No cameras could be opened. Exiting.");
    }

    info!(cameras = camera_handles.len(), "Recording started — press Ctrl-C to stop");

    // ── Signal handling ───────────────────────────────────────────────────── //

    let stop_signal = Arc::clone(&stop);
    ctrlc::set_handler(move || {
        info!("Signal received — stopping capture…");
        stop_signal.store(true, Ordering::Relaxed);
    })?;

    // ── Writer loop ───────────────────────────────────────────────────────── //

    let t_start = Instant::now();
    let mut total_frames: u64 = 0;

    loop {
        // Duration limit check
        if let Some(dur) = args.duration {
            if t_start.elapsed().as_secs_f64() >= dur {
                info!("Duration limit reached ({dur:.1}s)");
                stop.store(true, Ordering::Relaxed);
                break;
            }
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => {
                writer.write_frame(&frame)?;
                total_frames += 1;
            }
            Err(RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // All camera threads exited and dropped their senders.
                break;
            }
        }
    }

    // ── Teardown ──────────────────────────────────────────────────────────── //

    // Wait for all camera threads to finish.
    for handle in camera_handles {
        let _ = handle.join();
    }

    // Drain any frames still in the channel (cameras may have batched a few
    // frames before seeing the stop flag).
    while let Ok(frame) = rx.try_recv() {
        writer.write_frame(&frame)?;
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

    session.finalize(&mcap_path, total_frames, total_frame_bytes, sensor_infos)?;

    Ok(())
}
