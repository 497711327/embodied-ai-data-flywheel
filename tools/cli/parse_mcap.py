"""
Parse an MCAP recording and extract camera video data.

The MCAP files produced by edge-capture use the foxglove.CompressedImage
JSON schema on topics  /camera/<name>/compressed

Modes
-----
info        Print session summary: topics, frame counts, time range, clock source.
frames      Dump every frame as a JPEG file.
video       Re-encode each camera channel into an MP4 (requires opencv).
udp         Print or extract raw UDP packets from /udp/<name>/raw topics.

Usage
-----
    # Print recording summary
    python tools/cli/parse_mcap.py info data/raw/20260429/<uuid>/recording.mcap

    # Extract frames for all cameras (default output dir = ./frames/<name>/)
    python tools/cli/parse_mcap.py frames recording.mcap [--camera front] [--out ./frames]

    # Re-encode to MP4 (one per camera)
    python tools/cli/parse_mcap.py video recording.mcap [--fps 30] [--out ./video]

    # Inspect UDP packets
    python tools/cli/parse_mcap.py udp recording.mcap [--stream radar_front] [--limit 20]

    # Extract raw UDP payloads as .bin files
    python tools/cli/parse_mcap.py udp recording.mcap --out ./udp_packets
"""

from __future__ import annotations

import argparse
import base64
import json
import sys
from pathlib import Path
from typing import Iterator

# ── dependency checks ────────────────────────────────────────────────────────

try:
    from mcap.reader import make_reader
except ImportError:
    print("ERROR: mcap package not installed.  Run: pip install mcap")
    sys.exit(1)


def _require_cv2():
    try:
        import cv2
        return cv2
    except ImportError:
        print("ERROR: opencv-python-headless not installed.")
        print("       Run: pip install opencv-python-headless")
        sys.exit(1)


# ── MCAP helpers ─────────────────────────────────────────────────────────────

def _open_reader(mcap_path: Path):
    """Return an mcap Reader.  Works with both mcap 0.x and 1.x."""
    fh = open(mcap_path, "rb")
    return make_reader(fh), fh


def _iter_image_messages(
    mcap_path: Path,
    camera_filter: str | None = None,
) -> Iterator[tuple[str, int, bytes]]:
    """
    Yield (camera_name, timestamp_ns, jpeg_bytes) for every
    /camera/<name>/compressed message in the MCAP.

    Parameters
    ----------
    camera_filter:  if given, only yield messages from that camera name.
    """
    reader, fh = _open_reader(mcap_path)
    try:
        for schema, channel, message in reader.iter_messages():
            topic: str = channel.topic  # e.g. "/camera/front/compressed"
            if not topic.startswith("/camera/") or not topic.endswith("/compressed"):
                continue

            # Extract camera name from topic path.
            parts = topic.split("/")  # ['', 'camera', 'front', 'compressed']
            if len(parts) < 4:
                continue
            name = parts[2]

            if camera_filter and name != camera_filter:
                continue

            try:
                payload = json.loads(message.data)
                b64 = payload["data"]
                jpeg = base64.b64decode(b64)
            except (KeyError, ValueError, Exception) as e:
                print(f"  [WARN] Failed to decode frame on {topic}: {e}", file=sys.stderr)
                continue

            yield name, message.log_time, jpeg
    finally:
        fh.close()


def _iter_udp_messages(
    mcap_path: Path,
    stream_filter: str | None = None,
) -> Iterator[tuple[str, int, bytes]]:
    """
    Yield (stream_name, timestamp_ns, payload_bytes) for every
    /udp/<name>/raw message in the MCAP.

    Parameters
    ----------
    stream_filter: if given, only yield messages from that UDP stream name.
    """
    reader, fh = _open_reader(mcap_path)
    try:
        for schema, channel, message in reader.iter_messages():
            topic: str = channel.topic  # e.g. "/udp/radar_front/raw"
            if not topic.startswith("/udp/") or not topic.endswith("/raw"):
                continue

            parts = topic.split("/")  # ['', 'udp', 'radar_front', 'raw']
            if len(parts) < 4:
                continue
            name = parts[2]

            if stream_filter and name != stream_filter:
                continue

            try:
                payload = json.loads(message.data)
                b64 = payload["data"]
                raw = base64.b64decode(b64)
            except (KeyError, ValueError, Exception) as e:
                print(f"  [WARN] Failed to decode UDP packet on {topic}: {e}", file=sys.stderr)
                continue

            yield name, message.log_time, raw
    finally:
        fh.close()


# ── sub-commands ─────────────────────────────────────────────────────────────

def cmd_info(args: argparse.Namespace) -> None:
    """Print a human-readable summary of an MCAP file."""
    mcap_path = Path(args.mcap)
    if not mcap_path.exists():
        print(f"ERROR: file not found: {mcap_path}")
        sys.exit(1)

    reader, fh = _open_reader(mcap_path)
    try:
        summary = reader.get_summary()
    finally:
        fh.close()

    # ── manifest (sibling JSON) ──────────────────────────────────────────────
    manifest_path = mcap_path.parent / "session.manifest.json"
    manifest: dict = {}
    if manifest_path.exists():
        with open(manifest_path) as f:
            manifest = json.load(f)

    print("=" * 60)
    print(f"File  : {mcap_path}")
    print(f"Size  : {mcap_path.stat().st_size / 1e6:.1f} MB")
    print()

    if manifest:
        print(f"Session ID   : {manifest.get('session_id', 'n/a')}")
        print(f"Host         : {manifest.get('hostname', 'n/a')}")
        print(f"Git commit   : {manifest.get('git_commit', 'n/a')}")
        print(f"Started      : {manifest.get('started_at', 'n/a')}")
        print(f"Ended        : {manifest.get('ended_at', 'n/a')}")
        print(f"Clock source : {manifest.get('clock_source', 'n/a')}")
        print(f"PTP synced   : {manifest.get('ptp_synced', 'n/a')}")
        if "mono_to_tai_offset_ns" in manifest:
            offset_s = manifest["mono_to_tai_offset_ns"] / 1e9
            print(f"TAI offset   : {offset_s:.3f} s  "
                  f"(≈ TAI − UTC leap-second difference + monotonic offset)")
        total_frames = manifest.get("total_frames", 0)
        total_mb = manifest.get("total_frame_bytes", 0) / 1e6
        print(f"Total frames : {total_frames}")
        print(f"Image bytes  : {total_mb:.1f} MB")
        tags = manifest.get("tags", {})
        if tags:
            print(f"Tags         : {tags}")
        print()

        sensors = manifest.get("sensors", [])
        if sensors:
            print("Sensors:")
            for s in sensors:
                print(f"  {s['name']:20s}  /dev/video{s['device_id']}  "
                      f"{s['width']}×{s['height']}  "
                      f"{s['configured_fps']}fps (actual {s['actual_fps']:.1f})")
            print()

    # ── channel stats ────────────────────────────────────────────────────────
    if summary and summary.statistics:
        stats = summary.statistics
        start_ns = stats.message_start_time
        end_ns   = stats.message_end_time
        duration_s = (end_ns - start_ns) / 1e9 if end_ns > start_ns else 0.0

        print(f"Duration     : {duration_s:.2f} s")
        print(f"Time range   : {start_ns} → {end_ns} ns (CLOCK_TAI)")
        print()

        print("Channels:")
        ch_stats = stats.channel_message_counts if stats.channel_message_counts else {}

    if summary and summary.channels:
        for ch_id, ch in summary.channels.items():
            meta = dict(ch.metadata) if ch.metadata else {}
            clock_src = meta.get("clock_source", "unknown")
            count = ""
            if summary.statistics and summary.statistics.channel_message_counts:
                n = summary.statistics.channel_message_counts.get(ch_id, 0)
                count = f"  {n} msgs"
            print(f"  {ch.topic:<40s}  clock={clock_src}{count}")
    print("=" * 60)


def cmd_frames(args: argparse.Namespace) -> None:
    """Extract frames from MCAP and save as JPEG files."""
    mcap_path = Path(args.mcap)
    if not mcap_path.exists():
        print(f"ERROR: file not found: {mcap_path}")
        sys.exit(1)

    out_root = Path(args.out)
    camera_filter: str | None = args.camera

    # Collect frames per camera first (for progress reporting).
    counts: dict[str, int] = {}

    for name, ts_ns, jpeg in _iter_image_messages(mcap_path, camera_filter):
        cam_dir = out_root / name
        cam_dir.mkdir(parents=True, exist_ok=True)

        idx = counts.get(name, 0)
        out_path = cam_dir / f"{ts_ns:020d}_{idx:06d}.jpg"
        out_path.write_bytes(jpeg)
        counts[name] = idx + 1

        if idx % 30 == 0:
            print(f"  {name}: {idx + 1} frames → {cam_dir}", end="\r")

    print()
    if not counts:
        print("No image messages found (check --camera filter).")
        sys.exit(1)

    print("\nExtracted:")
    for name, n in sorted(counts.items()):
        cam_dir = out_root / name
        print(f"  {name}: {n} frames → {cam_dir}/")


def cmd_video(args: argparse.Namespace) -> None:
    """Re-encode camera streams to MP4."""
    cv2 = _require_cv2()

    mcap_path = Path(args.mcap)
    if not mcap_path.exists():
        print(f"ERROR: file not found: {mcap_path}")
        sys.exit(1)

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    camera_filter: str | None = args.camera
    fps: float = args.fps

    # Per-camera VideoWriter objects, created lazily on first frame.
    writers: dict[str, cv2.VideoWriter] = {}
    frame_counts: dict[str, int] = {}
    start_times: dict[str, int] = {}
    end_times: dict[str, int] = {}

    import numpy as np

    for name, ts_ns, jpeg in _iter_image_messages(mcap_path, camera_filter):
        # Decode JPEG bytes → numpy array
        arr = np.frombuffer(jpeg, dtype=np.uint8)
        frame = cv2.imdecode(arr, cv2.IMREAD_COLOR)
        if frame is None:
            print(f"  [WARN] Could not decode JPEG for {name} at {ts_ns}", file=sys.stderr)
            continue

        h, w = frame.shape[:2]

        if name not in writers:
            out_path = str(out_dir / f"{name}.mp4")
            fourcc = cv2.VideoWriter_fourcc(*"mp4v")
            writer = cv2.VideoWriter(out_path, fourcc, fps, (w, h))
            if not writer.isOpened():
                print(f"ERROR: cannot open VideoWriter for {out_path}")
                sys.exit(1)
            writers[name] = writer
            start_times[name] = ts_ns
            frame_counts[name] = 0
            print(f"  {name}: {w}×{h} → {out_path}")

        writers[name].write(frame)
        frame_counts[name] += 1
        end_times[name] = ts_ns

        idx = frame_counts[name]
        if idx % 30 == 0:
            print(f"  {name}: {idx} frames encoded", end="\r")

    print()

    for name, writer in writers.items():
        writer.release()
        n = frame_counts[name]
        dur_s = (end_times[name] - start_times[name]) / 1e9
        actual_fps = n / dur_s if dur_s > 0 else 0.0
        out_path = out_dir / f"{name}.mp4"
        print(f"  {name}: {n} frames, {dur_s:.1f}s, {actual_fps:.1f} fps → {out_path}")

    if not writers:
        print("No image messages found (check --camera filter).")
        sys.exit(1)


def cmd_udp(args: argparse.Namespace) -> None:
    """Print and optionally extract raw UDP payloads from MCAP."""
    mcap_path = Path(args.mcap)
    if not mcap_path.exists():
        print(f"ERROR: file not found: {mcap_path}")
        sys.exit(1)

    stream_filter: str | None = args.stream
    out_root = Path(args.out) if args.out else None
    if out_root:
        out_root.mkdir(parents=True, exist_ok=True)

    counts: dict[str, int] = {}
    total_bytes = 0
    total_packets = 0
    max_packets = args.limit
    hex_bytes = max(0, args.hex_bytes)

    for name, ts_ns, payload in _iter_udp_messages(mcap_path, stream_filter):
        idx = counts.get(name, 0)
        counts[name] = idx + 1
        total_packets += 1
        total_bytes += len(payload)

        if out_root:
            stream_dir = out_root / name
            stream_dir.mkdir(parents=True, exist_ok=True)
            out_path = stream_dir / f"{ts_ns:020d}_{idx:06d}.bin"
            out_path.write_bytes(payload)

        if max_packets is None or total_packets <= max_packets:
            preview = payload[:hex_bytes].hex() if hex_bytes > 0 else ""
            line = f"[{total_packets:06d}] stream={name} ts_ns={ts_ns} size={len(payload)}"
            if preview:
                suffix = "..." if len(payload) > hex_bytes else ""
                line += f" hex={preview}{suffix}"
            print(line)

    if not counts:
        print("No UDP messages found (check --stream filter).")
        sys.exit(1)

    print()
    print("UDP summary:")
    for name, count in sorted(counts.items()):
        print(f"  {name}: {count} packets")
    print(f"  total: {total_packets} packets, {total_bytes} bytes")

    if max_packets is not None and total_packets > max_packets:
        print(f"  note: only first {max_packets} packets were printed")

    if out_root:
        print(f"  raw payloads written to: {out_root}")


# ── CLI ───────────────────────────────────────────────────────────────────────

def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = p.add_subparsers(dest="cmd", required=True)

    # info
    pi = sub.add_parser("info", help="Print recording summary")
    pi.add_argument("mcap", help="Path to .mcap file")

    # frames
    pf = sub.add_parser("frames", help="Extract frames as JPEG files")
    pf.add_argument("mcap", help="Path to .mcap file")
    pf.add_argument("--camera", metavar="NAME",
                    help="Only extract this camera (default: all)")
    pf.add_argument("--out", default="./frames",
                    help="Output root directory (default: ./frames)")

    # video
    pv = sub.add_parser("video", help="Re-encode to MP4")
    pv.add_argument("mcap", help="Path to .mcap file")
    pv.add_argument("--camera", metavar="NAME",
                    help="Only encode this camera (default: all)")
    pv.add_argument("--fps", type=float, default=30.0,
                    help="Output video FPS (default: 30)")
    pv.add_argument("--out", default="./video",
                    help="Output directory (default: ./video)")

    # udp
    pu = sub.add_parser("udp", help="Print or extract raw UDP packets")
    pu.add_argument("mcap", help="Path to .mcap file")
    pu.add_argument("--stream", metavar="NAME",
                    help="Only inspect this UDP stream (default: all)")
    pu.add_argument("--limit", type=int, default=None,
                    help="Only print the first N packets (default: all)")
    pu.add_argument("--hex-bytes", type=int, default=16,
                    help="Hex preview bytes per packet line (default: 16)")
    pu.add_argument("--out", default=None,
                    help="Optional output directory for raw .bin payloads")

    return p


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    dispatch = {
        "info":   cmd_info,
        "frames": cmd_frames,
        "video":  cmd_video,
        "udp":    cmd_udp,
    }
    dispatch[args.cmd](args)


if __name__ == "__main__":
    main()
