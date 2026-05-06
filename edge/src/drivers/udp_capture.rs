//! UDP packet capture driver — captures raw UDP data and sends frames to a channel.
//!
//! Each [`UdpCapture`] binds to a local address and optionally joins a multicast
//! group. Received packets are timestamped and sent as [`UdpFrame`] to a
//! crossbeam channel, which feeds into the shared MCAP writer.
//!
//! Timestamps use milliseconds since UNIX epoch (same clock domain as camera
//! frames), enabling synchronization in the MCAP file.

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

/// Maximum single UDP datagram size.
const MAX_UDP_PAYLOAD: usize = 65535;

/// Receive buffer size.
const RECV_BUF_SIZE: usize = MAX_UDP_PAYLOAD + 64;

// ── Configuration ────────────────────────────────────────────────────────── //

/// Configuration for UDP capture (constructed from YAML config).
#[derive(Debug, Clone)]
pub struct UdpCaptureConfig {
    /// Name / label for this capture stream.
    pub name: String,
    /// Local bind address (e.g. "0.0.0.0:5000").
    pub bind_addr: String,
    /// Optional multicast group to join.
    pub multicast_group: Option<String>,
    /// Network interface name (for multicast join).
    pub interface: Option<String>,
}

// ── UDP Frame ────────────────────────────────────────────────────────────── //

/// A captured UDP frame, written into the MCAP file alongside camera frames.
#[derive(Debug, Clone)]
pub struct UdpFrame {
    pub stream_name: String,
    /// Milliseconds since UNIX epoch when the packet was received.
    pub timestamp_ms: u64,
    /// Raw UDP payload bytes.
    pub data: Vec<u8>,
}

// ── UDP Capture Worker ───────────────────────────────────────────────────── //

pub struct UdpCapture {
    config: UdpCaptureConfig,
    socket: UdpSocket,
}

impl UdpCapture {
    /// Bind to the configured address and optionally join multicast.
    pub fn open(config: UdpCaptureConfig) -> Result<Self> {
        let socket = UdpSocket::bind(&config.bind_addr)
            .with_context(|| format!("Failed to bind UDP socket to {}", config.bind_addr))?;

        // Enable receiving broadcast packets.
        socket.set_broadcast(true)?;

        // Set receive buffer to 8 MB to reduce kernel drops.
        set_recv_buffer(&socket, 8 * 1024 * 1024);

        // Join multicast group if configured.
        if let Some(ref group) = config.multicast_group {
            let group_addr: std::net::Ipv4Addr = group
                .parse()
                .with_context(|| format!("Invalid multicast group: {}", group))?;
            let iface = std::net::Ipv4Addr::UNSPECIFIED;
            socket
                .join_multicast_v4(&group_addr, &iface)
                .with_context(|| format!("Failed to join multicast {}", group))?;
            info!(name = %config.name, group = %group, "Joined multicast group");
        }

        // Non-blocking with a short timeout so we can check the stop flag.
        socket.set_read_timeout(Some(std::time::Duration::from_millis(100)))?;

        info!(
            name = %config.name,
            bind = %config.bind_addr,
            "UDP capture socket opened"
        );

        Ok(Self { config, socket })
    }

    /// Run the capture loop, sending [`UdpFrame`]s to the channel.
    ///
    /// Blocks until `stop` is set to true. Returns the total packet count.
    pub fn run_to_channel(
        &self,
        tx: crossbeam_channel::Sender<UdpFrame>,
        stop: Arc<AtomicBool>,
    ) -> Result<u64> {
        let mut recv_buf = vec![0u8; RECV_BUF_SIZE];
        let mut packet_count: u64 = 0;

        while !stop.load(Ordering::Relaxed) {
            let n = match self.socket.recv_from(&mut recv_buf) {
                Ok((n, _addr)) => n,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => {
                    warn!(name = %self.config.name, error = %e, "UDP recv error");
                    continue;
                }
            };

            if n == 0 {
                continue;
            }

            let timestamp_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            let frame = UdpFrame {
                stream_name: self.config.name.clone(),
                timestamp_ms,
                data: recv_buf[..n].to_vec(),
            };

            if tx.try_send(frame).is_err() {
                debug!(name = %self.config.name, "Channel full, dropping UDP packet");
            }

            packet_count += 1;
        }

        info!(
            name = %self.config.name,
            packets = packet_count,
            "UDP capture stopped"
        );
        Ok(packet_count)
    }

    pub fn config(&self) -> &UdpCaptureConfig {
        &self.config
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────── //

/// Best-effort attempt to increase the socket receive buffer.
fn set_recv_buffer(socket: &UdpSocket, size: usize) {
    use std::os::unix::io::AsRawFd;
    let fd = socket.as_raw_fd();
    let size_val = size as libc::c_int;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVBUF,
            &size_val as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        warn!("Failed to set SO_RCVBUF to {} bytes", size);
    }
}
