//! UDP packet capture driver — captures raw UDP data and sends frames to a channel.
//!
//! Each [`UdpCapture`] binds to a local address and optionally joins a multicast
//! group. Received packets are timestamped and sent as [`UdpFrame`] to a
//! crossbeam channel, which feeds into the shared MCAP writer.
//!
//! Timestamps use milliseconds since UNIX epoch (same clock domain as camera
//! frames), enabling synchronization in the MCAP file.

use std::io;
use std::net::UdpSocket;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use nix::time::{ClockId, clock_gettime};
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
    /// Nanoseconds since UNIX epoch in CLOCK_TAI domain.
    pub timestamp_ns: u64,
    /// Raw UDP payload bytes.
    pub data: Vec<u8>,
}

// ── UDP Capture Worker ───────────────────────────────────────────────────── //

pub struct UdpCapture {
    config: UdpCaptureConfig,
    socket: UdpSocket,
    realtime_to_tai_offset_ns: i64,
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

        // Ask kernel to attach nanosecond RX timestamps via ancillary data.
        set_socket_timestampns(&socket);

        let realtime_to_tai_offset_ns = measure_realtime_to_tai_offset_ns();

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
            rt_to_tai_offset_ns = realtime_to_tai_offset_ns,
            "UDP capture socket opened"
        );

        Ok(Self {
            config,
            socket,
            realtime_to_tai_offset_ns,
        })
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
            let (n, recv_rt_ns) = match recv_with_timestamp_ns(&self.socket, &mut recv_buf) {
                Ok(tuple) => tuple,
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

            // Kernel RX timestamps are CLOCK_REALTIME. Convert to the same
            // CLOCK_TAI domain used by camera frames for cross-sensor alignment.
            let timestamp_ns = realtime_ns_to_tai_ns(recv_rt_ns, self.realtime_to_tai_offset_ns);

            let frame = UdpFrame {
                stream_name: self.config.name.clone(),
                timestamp_ns,
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

/// Best-effort attempt to enable kernel receive timestamps (SO_TIMESTAMPNS).
fn set_socket_timestampns(socket: &UdpSocket) {
    let fd = socket.as_raw_fd();
    let on: libc::c_int = 1;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TIMESTAMPNS,
            &on as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        warn!(error = %io::Error::last_os_error(), "Failed to enable SO_TIMESTAMPNS");
    }
}

/// Measure CLOCK_TAI - CLOCK_REALTIME offset in nanoseconds.
fn measure_realtime_to_tai_offset_ns() -> i64 {
    let rt = clock_gettime(ClockId::CLOCK_REALTIME);
    let tai = clock_gettime(ClockId::CLOCK_TAI);

    match (rt, tai) {
        (Ok(rt), Ok(tai)) => {
            let rt_ns = rt.tv_sec() as i64 * 1_000_000_000 + rt.tv_nsec() as i64;
            let tai_ns = tai.tv_sec() as i64 * 1_000_000_000 + tai.tv_nsec() as i64;
            tai_ns - rt_ns
        }
        _ => {
            warn!("CLOCK_TAI unavailable while calibrating UDP timestamps; falling back to +37s");
            37_000_000_000
        }
    }
}

fn realtime_ns_to_tai_ns(realtime_ns: u64, offset_ns: i64) -> u64 {
    let tai = realtime_ns as i128 + offset_ns as i128;
    if tai <= 0 {
        0
    } else {
        tai as u64
    }
}

fn align_cmsg_len(value: usize) -> usize {
    let align = std::mem::size_of::<usize>();
    (value + align - 1) & !(align - 1)
}

/// Receive one UDP packet and return (payload_size, kernel_realtime_ns).
fn recv_with_timestamp_ns(socket: &UdpSocket, recv_buf: &mut [u8]) -> io::Result<(usize, u64)> {
    let fd = socket.as_raw_fd();

    let mut iov = libc::iovec {
        iov_base: recv_buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: recv_buf.len(),
    };

    // Enough for one cmsghdr + one timespec.
    let mut control_buf = [0u8; 128];

    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control_buf.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = control_buf.len();

    let n = unsafe { libc::recvmsg(fd, &mut msg, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut timestamp_ns = 0u64;
    let control_len = msg.msg_controllen as usize;
    let cmsg_hdr_len = std::mem::size_of::<libc::cmsghdr>();
    let cmsg_data_offset = align_cmsg_len(cmsg_hdr_len);

    let mut offset = 0usize;
    while offset + cmsg_hdr_len <= control_len {
        let cmsg_ptr = unsafe { control_buf.as_ptr().add(offset) as *const libc::cmsghdr };
        let cmsg = unsafe { &*cmsg_ptr };
        let cmsg_len = cmsg.cmsg_len as usize;

        if cmsg_len < cmsg_hdr_len || offset + cmsg_len > control_len {
            break;
        }

        if cmsg.cmsg_level == libc::SOL_SOCKET && cmsg.cmsg_type == libc::SO_TIMESTAMPNS {
            let required = cmsg_data_offset + std::mem::size_of::<libc::timespec>();
            if cmsg_len >= required {
                let ts_ptr = unsafe {
                    (cmsg_ptr as *const u8).add(cmsg_data_offset) as *const libc::timespec
                };
                let ts = unsafe { *ts_ptr };
                if ts.tv_sec >= 0 && ts.tv_nsec >= 0 {
                    timestamp_ns = ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64;
                }
                break;
            }
        }

        offset += align_cmsg_len(cmsg_len);
    }

    if timestamp_ns == 0 {
        timestamp_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
    }

    Ok((n as usize, timestamp_ns))
}

/// Best-effort attempt to increase the socket receive buffer.
fn set_recv_buffer(socket: &UdpSocket, size: usize) {
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
