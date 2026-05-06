//! MCAP writer — one file per recording session, one channel per camera.
//!
//! Schema: `foxglove.CompressedImage` (JSON encoding), compatible with
//! Foxglove Studio out of the box.
//!
//! Thread model: single-writer, NOT Send. All calls must come from the same
//! thread (the writer loop in main.rs).

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    fs::File,
    io::BufWriter,
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result};
use tracing::debug;

use crate::drivers::usb_camera::CameraFrame;
use crate::drivers::udp_capture::UdpFrame;

// ── foxglove.CompressedImage JSON schema ─────────────────────────────────── //

/// Minimal JSON Schema describing `foxglove.CompressedImage`.
/// Foxglove Studio uses this to decode the topic automatically.
const COMPRESSED_IMAGE_SCHEMA: &[u8] = br#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "foxglove.CompressedImage",
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec":  { "type": "integer" },
        "nsec": { "type": "integer" }
      }
    },
    "frame_id": { "type": "string" },
    "data":     { "type": "string", "contentEncoding": "base64" },
    "format":   { "type": "string" }
  }
}"#;

/// JSON Schema for raw UDP packets stored in MCAP.
const RAW_UDP_SCHEMA: &[u8] = br#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "RawUdpPacket",
  "type": "object",
  "properties": {
    "timestamp": {
      "type": "object",
      "properties": {
        "sec":  { "type": "integer" },
        "nsec": { "type": "integer" }
      }
    },
    "stream_name": { "type": "string" },
    "data":        { "type": "string", "contentEncoding": "base64" }
  }
}"#;

// ── Writer ───────────────────────────────────────────────────────────────── //

pub struct McapWriter {
    writer: mcap::Writer<'static, BufWriter<File>>,
    /// camera_name → Arc<Channel>, cached so we only register once.
    channels: HashMap<String, Arc<mcap::Channel<'static>>>,
    /// Per-channel sequence counter.
    sequences: HashMap<String, u32>,
    /// Running total of raw frame byte sizes (not MCAP overhead).
    total_frame_bytes: usize,
}

impl McapWriter {
    /// Create an MCAP file at `path`. Parent directory must already exist.
    pub fn new(path: &Path) -> Result<Self> {
        let file = File::create(path)
            .with_context(|| format!("Cannot create MCAP: {}", path.display()))?;
        let writer = mcap::Writer::new(BufWriter::new(file))
            .context("MCAP writer init")?;

        Ok(Self {
            writer,
            channels: HashMap::new(),
            sequences: HashMap::new(),
            total_frame_bytes: 0,
        })
    }

    /// Register a camera channel. Must be called before the first frame.
    /// Calling again with the same name is a no-op.
    ///
    /// `clock_source` is written into the MCAP channel metadata so that
    /// downstream readers know the time domain (e.g. `"CLOCK_TAI/PTP"`).
    pub fn register_camera(&mut self, name: &str, clock_source: &str) -> Result<()> {
        if self.channels.contains_key(name) {
            return Ok(());
        }

        let schema = Arc::new(mcap::Schema {
            name: "foxglove.CompressedImage".to_string(),
            encoding: "jsonschema".to_string(),
            data: Cow::Borrowed(COMPRESSED_IMAGE_SCHEMA),
        });

        let mut metadata = BTreeMap::new();
        metadata.insert("clock_source".to_string(), clock_source.to_string());

        let channel = Arc::new(mcap::Channel {
            topic: format!("/camera/{}/compressed", name),
            message_encoding: "json".to_string(),
            metadata,
            schema: Some(schema),
        });

        self.channels.insert(name.to_string(), channel);
        self.sequences.insert(name.to_string(), 0);
        debug!(camera = %name, "MCAP channel registered");
        Ok(())
    }

    /// Register a UDP stream channel. Must be called before the first UDP frame.
    pub fn register_udp_stream(&mut self, name: &str) -> Result<()> {
        let key = format!("udp_{}", name);
        if self.channels.contains_key(&key) {
            return Ok(());
        }

        let schema = Arc::new(mcap::Schema {
            name: "RawUdpPacket".to_string(),
            encoding: "jsonschema".to_string(),
            data: Cow::Borrowed(RAW_UDP_SCHEMA),
        });

        let mut metadata = BTreeMap::new();
        metadata.insert("source".to_string(), "udp_capture".to_string());

        let channel = Arc::new(mcap::Channel {
            topic: format!("/udp/{}/raw", name),
            message_encoding: "json".to_string(),
            metadata,
            schema: Some(schema),
        });

        self.channels.insert(key.clone(), channel);
        self.sequences.insert(key, 0);
        debug!(udp_stream = %name, "MCAP UDP channel registered");
        Ok(())
    }

    /// Write a UDP frame into the MCAP file.
    /// Timestamp (ms) is stored in the first 8 bytes of the stream for
    /// synchronization with camera frames.
    pub fn write_udp_frame(&mut self, frame: &UdpFrame) -> Result<()> {
        let key = format!("udp_{}", frame.stream_name);
        let channel = match self.channels.get(&key) {
            Some(c) => Arc::clone(c),
            None => {
                self.register_udp_stream(&frame.stream_name)?;
                Arc::clone(self.channels.get(&key).unwrap())
            }
        };

        let seq = self.sequences.get_mut(&key).unwrap();
        let cur_seq = *seq;
        *seq = seq.wrapping_add(1);

        // Convert ms to ns for MCAP log_time.
        let timestamp_ns = frame.timestamp_ms * 1_000_000;
        let sec = timestamp_ns / 1_000_000_000;
        let nsec = timestamp_ns % 1_000_000_000;

        let b64 = base64_encode(&frame.data);

        let payload = serde_json::json!({
            "timestamp": { "sec": sec, "nsec": nsec },
            "stream_name": frame.stream_name,
            "data": b64
        });
        let payload_bytes = serde_json::to_vec(&payload).context("JSON encode UDP")?;

        self.writer
            .write(&mcap::Message {
                channel,
                sequence: cur_seq,
                log_time: timestamp_ns,
                publish_time: timestamp_ns,
                data: Cow::Owned(payload_bytes),
            })
            .context("MCAP write UDP")?;

        self.total_frame_bytes += frame.data.len();
        Ok(())
    }

    /// Encode `frame` as a `foxglove.CompressedImage` JSON message and write
    /// it to the MCAP file.
    pub fn write_frame(&mut self, frame: &CameraFrame) -> Result<()> {
        let channel = match self.channels.get(&frame.camera_name) {
            Some(c) => Arc::clone(c),
            None => {
                // Auto-register on first sight (shouldn't happen in normal use)
                self.register_camera(&frame.camera_name, "unknown")?;
                Arc::clone(self.channels.get(&frame.camera_name).unwrap())
            }
        };

        let seq = self.sequences.get_mut(&frame.camera_name).unwrap();
        let cur_seq = *seq;
        *seq = seq.wrapping_add(1);

        let sec = frame.timestamp_ns / 1_000_000_000;
        let nsec = frame.timestamp_ns % 1_000_000_000;

        // Encode image bytes as base64 (required by the JSON schema).
        let b64 = base64_encode(&frame.data);

        let payload = serde_json::json!({
            "timestamp": { "sec": sec, "nsec": nsec },
            "frame_id":  frame.camera_name,
            "data":      b64,
            "format":    frame.encoding
        });
        let payload_bytes = serde_json::to_vec(&payload).context("JSON encode")?;

        self.writer
            .write(&mcap::Message {
                channel,
                sequence: cur_seq,
                log_time: frame.timestamp_ns,
                publish_time: frame.timestamp_ns,
                data: Cow::Owned(payload_bytes),
            })
            .context("MCAP write")?;

        self.total_frame_bytes += frame.data.len();
        Ok(())
    }

    /// Total raw image bytes written (excludes MCAP framing overhead).
    pub fn total_frame_bytes(&self) -> usize {
        self.total_frame_bytes
    }

    /// Finalise the MCAP file (writes chunk index, summary, magic footer).
    pub fn close(mut self) -> Result<()> {
        self.writer.finish().context("MCAP finish")?;
        Ok(())
    }
}

// ── Base64 encoder (no external dep) ─────────────────────────────────────── //

const B64_CHARS: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        let combined = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_CHARS[(combined >> 18) & 0x3f] as char);
        out.push(B64_CHARS[(combined >> 12) & 0x3f] as char);
        if chunk.len() > 1 {
            out.push(B64_CHARS[(combined >> 6) & 0x3f] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_CHARS[combined & 0x3f] as char);
        } else {
            out.push('=');
        }
    }
    out
}
