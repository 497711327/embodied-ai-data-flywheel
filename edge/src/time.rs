//! PTP-aware clock calibration.
//!
//! # Design
//!
//! V4L2 buffer metadata timestamps are in **CLOCK_MONOTONIC** (set by the
//! kernel driver at frame-capture time, before the buffer is dequeued to
//! userspace). This is far more accurate than calling `SystemTime::now()`
//! after the `stream.next()` call, because it eliminates:
//!
//! * USB transfer latency
//! * Kernel→userspace dequeue latency
//! * Thread-scheduling jitter between cameras
//!
//! **CLOCK_TAI** (International Atomic Time) is the target time domain:
//! * Continuous — no leap-second jumps (unlike CLOCK_REALTIME)
//! * Disciplined by `ptp4l + phc2sys` when PTP hardware is present, or by
//!   `chronyd`/`ntpd` otherwise
//! * MCAP / Foxglove Studio uses nanosecond timestamps; TAI is the standard
//!   time base for robotic datasets (e.g. ROS 2's `builtin_interfaces/Time`)
//!
//! ## Conversion formula
//!
//! ```text
//! tai_ns = hw_mono_ns + mono_to_tai_offset_ns
//! ```
//!
//! The offset is measured once at startup by sampling both clocks back-to-back.
//! It stays constant between NTP/PTP steps (typically < 1 µs/s drift).
//!
//! ## PTP setup (Linux)
//!
//! ```bash
//! apt install linuxptp
//! # Sync PHC (NIC hardware clock) to grandmaster:
//! ptp4l -i eth0 -m -s
//! # Feed CLOCK_TAI from PHC (offset = current TAI-UTC = 37 s since 2017):
//! phc2sys -s eth0 -c CLOCK_TAI -O -37 -m
//! ```
//!
//! Without PTP, CLOCK_TAI is still NTP-disciplined via the kernel's
//! `adjtimex` TAI offset (accuracy ~1–10 ms, adequate for labelling
//! but not for sub-frame multi-camera alignment).

use nix::time::{clock_gettime, ClockId};
use tracing::{info, warn};

// ── Public type ───────────────────────────────────────────────────────────── //

/// Offset between CLOCK_MONOTONIC (V4L2 hardware timestamps) and CLOCK_TAI.
///
/// Measured once at startup. Immutable after construction — share via `Arc`.
#[derive(Debug, Clone)]
pub struct ClockCalibration {
    /// Add this to a CLOCK_MONOTONIC nanosecond value to get CLOCK_TAI ns.
    ///
    /// `tai_ns = hw_mono_ns + mono_to_tai_offset_ns`
    pub mono_to_tai_offset_ns: i64,

    /// `true` if `ptp4l` was detected running on this host.
    pub ptp_synced: bool,

    /// Human-readable label written to MCAP channel metadata and session
    /// manifest so downstream tools know the clock provenance.
    pub clock_label: &'static str,
}

impl ClockCalibration {
    /// Sample both CLOCK_MONOTONIC and CLOCK_TAI back-to-back, compute the
    /// offset, and check for `ptp4l`.
    ///
    /// Falls back to `CLOCK_REALTIME + 37 s` if `CLOCK_TAI` is unavailable
    /// (kernel < 3.10, very unlikely on any modern system).
    pub fn measure() -> Self {
        // Sample CLOCK_MONOTONIC first so the offset accounts for the brief
        // gap between the two clock_gettime calls (< 1 µs in practice).
        let mono = clock_gettime(ClockId::CLOCK_MONOTONIC)
            .expect("CLOCK_MONOTONIC unavailable — this should never happen");
        let mono_ns = mono.tv_sec() as i64 * 1_000_000_000 + mono.tv_nsec() as i64;

        match clock_gettime(ClockId::CLOCK_TAI) {
            Ok(tai) => {
                let tai_ns = tai.tv_sec() as i64 * 1_000_000_000 + tai.tv_nsec() as i64;
                let offset = tai_ns - mono_ns;

                // Sanity: TAI must be > 2020-01-01 00:00:37 TAI.
                // If it's near zero the kernel TAI offset was never set.
                let sane = tai.tv_sec() > 1_577_836_837_i64;
                let ptp_running = ptp4l_running();
                let ptp_synced = sane && ptp_running;

                let clock_label = if ptp_synced {
                    "CLOCK_TAI/PTP"
                } else {
                    "CLOCK_TAI/NTP"
                };

                if ptp_synced {
                    info!(
                        mono_to_tai_offset_s = offset / 1_000_000_000,
                        "PTP detected — all timestamps in CLOCK_TAI (sub-µs accuracy)"
                    );
                } else {
                    warn!(
                        mono_to_tai_offset_s = offset / 1_000_000_000,
                        ptp4l_found = ptp_running,
                        tai_sane    = sane,
                        "ptp4l not active — timestamps in CLOCK_TAI (NTP-disciplined, ~1–10 ms). \
                         Install linuxptp and run `ptp4l -i <iface> -s` + \
                         `phc2sys -s <iface> -c CLOCK_TAI -O -37` for sub-µs alignment."
                    );
                }

                Self { mono_to_tai_offset_ns: offset, ptp_synced, clock_label }
            }

            Err(_) => {
                // Kernel < 3.10: CLOCK_TAI unavailable. Fall back to
                // CLOCK_REALTIME + approximate TAI-UTC offset (37 s since 2017).
                warn!(
                    "CLOCK_TAI unavailable — falling back to CLOCK_REALTIME + 37 s. \
                     Upgrade kernel to ≥ 3.10 for accurate TAI."
                );
                let rt = clock_gettime(ClockId::CLOCK_REALTIME)
                    .expect("CLOCK_REALTIME unavailable");
                let rt_ns = rt.tv_sec() as i64 * 1_000_000_000 + rt.tv_nsec() as i64;
                // TAI ≈ REALTIME + 37 s  →  offset = (RT - MONO) + 37 s
                let offset = (rt_ns - mono_ns) + 37_000_000_000_i64;
                Self {
                    mono_to_tai_offset_ns: offset,
                    ptp_synced: false,
                    clock_label: "CLOCK_REALTIME+37s/approx",
                }
            }
        }
    }

    /// Convert a V4L2 `CLOCK_MONOTONIC` hardware timestamp (nanoseconds) to
    /// `CLOCK_TAI` nanoseconds.
    ///
    /// # Arguments
    /// * `mono_ns` — `meta.timestamp.sec() * 1e9 + meta.timestamp.usec() * 1e3`
    #[inline]
    pub fn mono_to_tai(&self, mono_ns: u64) -> u64 {
        (mono_ns as i64 + self.mono_to_tai_offset_ns) as u64
    }

    /// Sample a fresh CLOCK_TAI timestamp (for non-V4L2 events such as
    /// session start/end or dropped-frame markers).
    #[inline]
    pub fn now_tai_ns(&self) -> u64 {
        match clock_gettime(ClockId::CLOCK_TAI) {
            Ok(t) => (t.tv_sec() as u64) * 1_000_000_000 + t.tv_nsec() as u64,
            Err(_) => {
                // Fallback: CLOCK_MONOTONIC + offset
                let m = clock_gettime(ClockId::CLOCK_MONOTONIC).unwrap();
                let mono_ns = m.tv_sec() as u64 * 1_000_000_000 + m.tv_nsec() as u64;
                self.mono_to_tai(mono_ns)
            }
        }
    }
}

// ── ptp4l detection ───────────────────────────────────────────────────────── //

/// Check whether `ptp4l` appears to be running by looking for its PID/socket
/// files (as written by `linuxptp` defaults).
fn ptp4l_running() -> bool {
    let paths = [
        "/var/run/ptp4l.pid",
        "/run/ptp4l.pid",
        "/var/run/ptp4l",
        "/run/ptp4l",
    ];
    paths
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}
