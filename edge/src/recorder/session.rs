//! Session lifecycle: directory creation, UUID, manifest JSON.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

// ── Types ─────────────────────────────────────────────────────────────────── //

#[derive(Debug, Serialize)]
pub struct SensorInfo {
    pub name: String,
    pub device_id: u32,
    pub width: u32,
    pub height: u32,
    pub configured_fps: u32,
    pub actual_fps: f64,
}

#[derive(Debug, Serialize)]
pub struct SessionManifest {
    pub session_id: String,
    pub started_at: String,
    pub ended_at: String,
    pub hostname: String,
    pub git_commit: Option<String>,
    pub mcap_path: String,
    pub total_frames: u64,
    pub total_frame_bytes: usize,
    pub sensors: Vec<SensorInfo>,
    pub tags: HashMap<String, String>,
    /// Clock domain used for all frame timestamps (e.g. `"CLOCK_TAI/PTP"`).
    pub clock_source: String,
    /// Whether `ptp4l` was detected running at session start.
    pub ptp_synced: bool,
    /// CLOCK_MONOTONIC → CLOCK_TAI offset applied to V4L2 hardware timestamps (ns).
    pub mono_to_tai_offset_ns: i64,
}

// ── SessionManager ────────────────────────────────────────────────────────── //

pub struct SessionManager {
    pub session_id: Uuid,
    pub started_at: chrono::DateTime<Utc>,
    session_dir: PathBuf,
    tags: HashMap<String, String>,
}

impl SessionManager {
    /// Create a new session rooted under `output_dir`.
    ///
    /// Directory layout: `<output_dir>/YYYYMMDD/<uuid>/`
    pub fn new(output_dir: PathBuf, tags: HashMap<String, String>) -> Self {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let date = now.format("%Y%m%d").to_string();
        let dir = output_dir.join(date).join(id.to_string());
        Self {
            session_id: id,
            started_at: now,
            session_dir: dir,
            tags,
        }
    }

    /// Absolute path to this session's directory (create it before use).
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// Ensure the session directory exists.
    pub fn create_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.session_dir).with_context(|| {
            format!("Cannot create session dir: {}", self.session_dir.display())
        })
    }

    /// Write `session.manifest.json` into the session directory.
    pub fn finalize(
        &self,
        mcap_path: &Path,
        total_frames: u64,
        total_frame_bytes: usize,
        sensors: Vec<SensorInfo>,
        clock_source: &str,
        ptp_synced: bool,
        mono_to_tai_offset_ns: i64,
    ) -> Result<()> {
        let ended_at = Utc::now();

        let manifest = SessionManifest {
            session_id: self.session_id.to_string(),
            started_at: self.started_at.to_rfc3339(),
            ended_at: ended_at.to_rfc3339(),
            hostname: hostname(),
            git_commit: git_commit(),
            mcap_path: mcap_path
                .canonicalize()
                .unwrap_or_else(|_| mcap_path.to_path_buf())
                .display()
                .to_string(),
            total_frames,
            total_frame_bytes,
            sensors,
            tags: self.tags.clone(),
            clock_source: clock_source.to_string(),
            ptp_synced,
            mono_to_tai_offset_ns,
        };

        let manifest_path = self.session_dir.join("session.manifest.json");
        let json = serde_json::to_string_pretty(&manifest).context("serialise manifest")?;
        std::fs::write(&manifest_path, json)
            .with_context(|| format!("write manifest: {}", manifest_path.display()))?;

        tracing::info!(path = %manifest_path.display(), "Manifest written");
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────── //

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn git_commit() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}
