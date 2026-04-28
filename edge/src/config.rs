//! YAML config types and loader.

use std::{collections::HashMap, path::PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

// ── Top-level config ─────────────────────────────────────────────────────── //

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub session: SessionConfig,
    pub cameras: Vec<CameraConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub struct SessionConfig {
    /// Root directory under which per-session subdirectories are created.
    pub output_dir: Option<PathBuf>,
    /// Arbitrary key-value metadata burned into the session manifest.
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

// ── Per-camera config ────────────────────────────────────────────────────── //

#[derive(Debug, Clone, Deserialize)]
pub struct CameraConfig {
    /// Logical name used in MCAP topic paths: `/camera/<name>/compressed`
    pub name: String,
    /// V4L2 device index (0 → /dev/video0)
    pub device_id: u32,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// JPEG quality 1-100, used only when re-encoding YUYV frames.
    #[serde(default = "default_jpeg_quality")]
    pub jpeg_quality: u8,
    /// Number of V4L2 mmap buffers to pre-allocate.
    #[serde(default = "default_buffer_size")]
    pub buffer_size: u32,
}

fn default_width() -> u32 { 1280 }
fn default_height() -> u32 { 720 }
fn default_fps() -> u32 { 30 }
fn default_jpeg_quality() -> u8 { 90 }
fn default_buffer_size() -> u32 { 4 }

// ── Loader ───────────────────────────────────────────────────────────────── //

pub fn load(path: &std::path::Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read config: {}", path.display()))?;
    serde_yaml::from_str(&text)
        .with_context(|| format!("Cannot parse config: {}", path.display()))
}
