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
phoenix     One-shot export for Phoenix replay (cameraXX.mp4 + .mudp).
phoenix-batch Batch Phoenix export for all MCAP files in a folder.

Note
----
Phoenix video export uses ffmpeg strict mode (H264 + ASS subtitle + matroska container).
Install ffmpeg before running `phoenix` / `phoenix-batch`.

Usage
-----
    # Print recording summary
    python tools/cli/parse_mcap.py info data/raw/20260429/<uuid>/recording.mcap

    # Extract frames for all cameras (default output dir = ./frames/<name>/)
    python tools/cli/parse_mcap.py frames recording.mcap [--camera front] [--out ./frames]

    # Re-encode to MP4 (one per camera)
    python tools/cli/parse_mcap.py video recording.mcap [--fps 30] [--out ./video]

    # Re-encode with Phoenix-compatible naming (camera01.mp4, camera02.mp4...)
    python tools/cli/parse_mcap.py video recording.mcap --phoenix-naming --out ./video

    # Inspect UDP packets
    python tools/cli/parse_mcap.py udp recording.mcap [--stream radar_front] [--limit 20]

    # Extract UDP payloads as .mudp files (one per stream)
    python tools/cli/parse_mcap.py udp recording.mcap --out ./mudp

    # One-shot Phoenix export (video + mudp)
    python tools/cli/parse_mcap.py phoenix recording.mcap --out ./phoenix_bundle

    # Batch Phoenix export for all MCAP files under a directory
    python tools/cli/parse_mcap.py phoenix-batch data/raw/20260507 --out ./phoenix_batch
"""

from __future__ import annotations

import argparse
import base64
import json
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import BinaryIO, Iterator

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


def _ass_time_from_ms(ms: int) -> str:
    """Convert milliseconds to ASS timestamp format H:MM:SS.CS."""
    if ms < 0:
        ms = 0
    cs = (ms % 1000) // 10
    total_s = ms // 1000
    s = total_s % 60
    total_m = total_s // 60
    m = total_m % 60
    h = total_m // 60
    return f"{h}:{m:02d}:{s:02d}.{cs:02d}"


def _build_ass_lines(frame_ts_ns: list[int]) -> list[str]:
    """Build ASS dialogue lines carrying Phoenix-style UTC text per frame.

    ASS Start/End times are relative to the first frame so that the matroska
    container reports the correct video duration.  The subtitle Text field
    carries the absolute TAI millisecond timestamp so Phoenix can still read
    wall-clock time from the subtitle stream.
    """
    if not frame_ts_ns:
        return []

    t0_ns = frame_ts_ns[0]  # recording-relative zero
    lines: list[str] = []
    # Use frame-to-frame delta for subtitle duration; clamp to at least 1 ms.
    default_dur_ms = 33

    for i, ts_ns in enumerate(frame_ts_ns):
        start_ms = (ts_ns - t0_ns) // 1_000_000   # relative, for ASS timing
        abs_us = ts_ns // 1_000                    # absolute TAI microseconds - must match MUDP cantime unit
        if i + 1 < len(frame_ts_ns):
            next_ms = (frame_ts_ns[i + 1] - t0_ns) // 1_000_000
            dur_ms = max(1, next_ms - start_ms)
        else:
            dur_ms = default_dur_ms
        end_ms = start_ms + dur_ms

        # Phoenix sync: Get_cantime() returns uint64_t timestamps (µs) from the
        # MUDP header and passes it as time_stamp to the video decoder, which
        # then looks for the subtitle frame whose UTC value matches.  Both must
        # therefore use the same unit: microseconds.
        lines.append(
            f"Dialogue: 0,{_ass_time_from_ms(start_ms)},{_ass_time_from_ms(end_ms)},"
            f"Default,,0,0,0,UTC:{abs_us}"
        )

    return lines


def _write_ass_file(path: Path, frame_ts_ns: list[int]) -> None:
    header = [
        "[Script Info]",
        "ScriptType: v4.00+",
        "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
        "Style: Default,Arial,20,&H00FFFFFF,&H00FFFFFF,&H00000000,&H64000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1",
        "",
        "[Events]",
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Text",
    ]
    lines = header + _build_ass_lines(frame_ts_ns)
    # Write UTF-8 BOM to mirror Phoenix subtitle packet behavior.
    content = "\ufeff" + "\n".join(lines) + "\n"
    path.write_text(content, encoding="utf-8")


def _encode_phoenix_h264(
    out_path: Path,
    frames_dir: Path,
    ass_path: Path,
    fps: float,
) -> None:
    ffmpeg_bin = shutil.which("ffmpeg")
    if not ffmpeg_bin:
        print("ERROR: ffmpeg not found. Install ffmpeg for Phoenix strict export.")
        sys.exit(1)

    cmd = [
        ffmpeg_bin,
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-framerate",
        f"{fps}",
        "-i",
        str(frames_dir / "%06d.jpg"),
        "-i",
        str(ass_path),
        "-map",
        "0:v:0",
        "-map",
        "1:s:0",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-g",
        "10",
        "-bf",
        "0",
        "-pix_fmt",
        "yuv420p",
        "-c:s",
        "ass",
        "-f",
        "matroska",
        str(out_path),
    ]

    ret = subprocess.run(cmd, check=False)
    if ret.returncode != 0:
        print(f"ERROR: ffmpeg failed for {out_path}")
        sys.exit(1)


def _cmd_video_phoenix_strict(
    mcap_path: Path,
    out_dir: Path,
    camera_filter: str | None,
    fps: float,
) -> None:
    """Export Phoenix-compatible videos: H264 + ASS subtitle in matroska container."""
    out_dir.mkdir(parents=True, exist_ok=True)

    # Gather raw JPEG payloads from MCAP by camera.
    by_camera: dict[str, list[tuple[int, bytes]]] = {}
    for name, ts_ns, jpeg in _iter_image_messages(mcap_path, camera_filter):
        by_camera.setdefault(name, []).append((ts_ns, jpeg))

    if not by_camera:
        print("No image messages found (check --camera filter).")
        sys.exit(1)

    camera_order = {name: idx for idx, name in enumerate(sorted(by_camera.keys()))}

    for name in sorted(by_camera.keys(), key=lambda n: camera_order[n]):
        frames = by_camera[name]
        out_name = f"_camera{camera_order[name] + 1:02d}.mp4"
        out_path = out_dir / out_name

        with tempfile.TemporaryDirectory(prefix="phoenix_vid_") as tmp:
            tmp_dir = Path(tmp)
            frames_dir = tmp_dir / "frames"
            frames_dir.mkdir(parents=True, exist_ok=True)

            ts_ns_list: list[int] = []
            for idx, (ts_ns, jpeg) in enumerate(frames):
                (frames_dir / f"{idx:06d}.jpg").write_bytes(jpeg)
                ts_ns_list.append(ts_ns)

            ass_path = tmp_dir / "timestamps.ass"
            _write_ass_file(ass_path, ts_ns_list)
            _encode_phoenix_h264(out_path, frames_dir, ass_path, fps)

        dur_s = (frames[-1][0] - frames[0][0]) / 1e9 if len(frames) > 1 else 0.0
        actual_fps = (len(frames) / dur_s) if dur_s > 0 else 0.0
        print(f"  {name}: {len(frames)} frames, {dur_s:.1f}s, {actual_fps:.1f} fps -> {out_path}")

    print("\nPhoenix mapping:")
    for name, idx in sorted(camera_order.items(), key=lambda item: item[1]):
        print(f"  {name} -> _camera{idx + 1:02d}.mp4")


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
    mcap_path = Path(args.mcap)
    if not mcap_path.exists():
        print(f"ERROR: file not found: {mcap_path}")
        sys.exit(1)

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    camera_filter: str | None = args.camera
    fps: float = args.fps
    phoenix_naming: bool = args.phoenix_naming

    # Phoenix replay requires H264 + subtitle stream in matroska container.
    # Keep this strict mode on for one-shot Phoenix export.
    if getattr(args, "phoenix_strict", False):
        _cmd_video_phoenix_strict(mcap_path, out_dir, camera_filter, fps)
        return

    cv2 = _require_cv2()

    # Per-camera VideoWriter objects, created lazily on first frame.
    writers: dict[str, cv2.VideoWriter] = {}
    frame_counts: dict[str, int] = {}
    start_times: dict[str, int] = {}
    end_times: dict[str, int] = {}
    output_names: dict[str, str] = {}
    camera_order: dict[str, int] = {}

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
            if name not in camera_order:
                camera_order[name] = len(camera_order)

            if phoenix_naming:
                out_name = f"_camera{camera_order[name] + 1:02d}.mp4"
            else:
                out_name = f"{name}.mp4"

            output_names[name] = out_name
            out_path = str(out_dir / out_name)
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
        out_path = out_dir / output_names.get(name, f"{name}.mp4")
        print(f"  {name}: {n} frames, {dur_s:.1f}s, {actual_fps:.1f} fps → {out_path}")

    if phoenix_naming and writers:
        print("\nPhoenix mapping:")
        for name, idx in sorted(camera_order.items(), key=lambda item: item[1]):
            print(f"  {name} -> {output_names[name]}")

    if not writers:
        print("No image messages found (check --camera filter).")
        sys.exit(1)


def cmd_udp(args: argparse.Namespace) -> None:
    """Print UDP packets and optionally export them as .mudp files."""
    mcap_path = Path(args.mcap)
    if not mcap_path.exists():
        print(f"ERROR: file not found: {mcap_path}")
        sys.exit(1)

    stream_filter: str | None = args.stream
    out_root = Path(args.out) if args.out else None
    if out_root:
        out_root.mkdir(parents=True, exist_ok=True)

    mudp_files: dict[str, BinaryIO] = {}

    counts: dict[str, int] = {}
    total_bytes = 0
    total_packets = 0
    max_packets = args.limit
    hex_bytes = max(0, args.hex_bytes)

    try:
        for name, ts_ns, payload in _iter_udp_messages(mcap_path, stream_filter):
            idx = counts.get(name, 0)
            counts[name] = idx + 1
            total_packets += 1
            total_bytes += len(payload)

            if out_root:
                fh = mudp_files.get(name)
                if fh is None:
                    out_path = out_root / f"{name}.mudp"
                    fh = open(out_path, "wb")
                    mudp_files[name] = fh

                # MudpLogHeader_T: first 8 bytes are timestamp(us), little-endian.
                # Then append raw UDP bytes; packet headers remain unchanged.
                ts_us = ts_ns // 1_000
                fh.write(struct.pack("<Q", ts_us))
                fh.write(payload)

            if max_packets is None or total_packets <= max_packets:
                preview = payload[:hex_bytes].hex() if hex_bytes > 0 else ""
                line = f"[{total_packets:06d}] stream={name} ts_ns={ts_ns} size={len(payload)}"
                if preview:
                    suffix = "..." if len(payload) > hex_bytes else ""
                    line += f" hex={preview}{suffix}"
                print(line)
    finally:
        for fh in mudp_files.values():
            fh.close()

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
        print(f"  mudp files written to: {out_root}")


def cmd_phoenix(args: argparse.Namespace) -> None:
    """One-shot export for Phoenix replay assets."""
    mcap_path = Path(args.mcap)
    if not mcap_path.exists():
        print(f"ERROR: file not found: {mcap_path}")
        sys.exit(1)

    out_root = Path(args.out)
    out_root.mkdir(parents=True, exist_ok=True)

    video_out = out_root / "video"
    mudp_out = out_root / "udp"

    print("[1/2] Exporting video for Phoenix replay...")
    cmd_video(argparse.Namespace(
        mcap=str(mcap_path),
        camera=args.camera,
        fps=args.fps,
        out=str(video_out),
        phoenix_naming=True,
        phoenix_strict=True,
    ))

    print("\n[2/2] Exporting UDP as .mudp (timestamp in microseconds)...")
    cmd_udp(argparse.Namespace(
        mcap=str(mcap_path),
        stream=args.stream,
        limit=0,
        hex_bytes=0,
        out=str(mudp_out),
    ))

    print("\nPhoenix bundle ready:")
    print(f"  video: {video_out}")
    print(f"  udp:   {mudp_out}")


def cmd_phoenix_batch(args: argparse.Namespace) -> None:
    """Batch export Phoenix replay assets for all MCAP files in a folder."""
    in_root = Path(args.input)
    if not in_root.exists() or not in_root.is_dir():
        print(f"ERROR: input directory not found: {in_root}")
        sys.exit(1)

    out_root = Path(args.out)
    out_root.mkdir(parents=True, exist_ok=True)

    pattern = args.pattern
    recursive = args.recursive
    iterator = in_root.rglob(pattern) if recursive else in_root.glob(pattern)
    mcap_files = sorted(p for p in iterator if p.is_file())

    if not mcap_files:
        print(f"No files matched pattern '{pattern}' under {in_root}")
        sys.exit(1)

    print(f"Found {len(mcap_files)} MCAP files. Starting batch export...")

    success = 0
    failed = 0

    for idx, mcap_path in enumerate(mcap_files, start=1):
        rel_parent = mcap_path.parent.relative_to(in_root)
        bundle_dir = out_root / rel_parent / mcap_path.stem

        print("\n" + "=" * 72)
        print(f"[{idx}/{len(mcap_files)}] {mcap_path}")
        print(f"Output -> {bundle_dir}")

        try:
            cmd_phoenix(argparse.Namespace(
                mcap=str(mcap_path),
                camera=args.camera,
                stream=args.stream,
                fps=args.fps,
                out=str(bundle_dir),
            ))
            success += 1
        except SystemExit:
            failed += 1
            print(f"[WARN] Failed to export: {mcap_path}", file=sys.stderr)
            if not args.continue_on_error:
                break
        except Exception as e:
            failed += 1
            print(f"[WARN] Failed to export: {mcap_path} ({e})", file=sys.stderr)
            if not args.continue_on_error:
                break

    print("\n" + "=" * 72)
    print("Batch export finished:")
    print(f"  success: {success}")
    print(f"  failed:  {failed}")
    print(f"  output:  {out_root}")

    if failed > 0 and not args.continue_on_error:
        sys.exit(1)


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
    pv.add_argument("--phoenix-naming", action="store_true",
                    help="Use Phoenix-style output names (_camera01.mp4, _camera02.mp4...) for folder replay")
    pv.add_argument("--phoenix-strict", action="store_true",
                    help="Use Phoenix replay compatible video export (H264 + ASS subtitle stream in matroska)")

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
                    help="Optional output directory for .mudp files (one per stream)")

    # phoenix
    pp = sub.add_parser("phoenix", help="One-shot Phoenix export (cameraXX.mp4 + .mudp, requires ffmpeg)")
    pp.add_argument("mcap", help="Path to .mcap file")
    pp.add_argument("--camera", metavar="NAME",
                    help="Only export this camera to video (default: all)")
    pp.add_argument("--stream", metavar="NAME",
                    help="Only export this UDP stream to mudp (default: all)")
    pp.add_argument("--fps", type=float, default=30.0,
                    help="Output video FPS (default: 30)")
    pp.add_argument("--out", default="./phoenix_bundle",
                    help="Output bundle directory (default: ./phoenix_bundle)")

    # phoenix-batch
    pb = sub.add_parser("phoenix-batch", help="Batch Phoenix export for all MCAP files in a folder (requires ffmpeg)")
    pb.add_argument("input", help="Input directory containing .mcap files")
    pb.add_argument("--camera", metavar="NAME",
                    help="Only export this camera to video (default: all)")
    pb.add_argument("--stream", metavar="NAME",
                    help="Only export this UDP stream to mudp (default: all)")
    pb.add_argument("--fps", type=float, default=30.0,
                    help="Output video FPS (default: 30)")
    pb.add_argument("--pattern", default="*.mcap",
                    help="Glob pattern for input files (default: *.mcap)")
    pb.add_argument("--recursive", action="store_true", default=True,
                    help="Recursively search input directory (default: on)")
    pb.add_argument("--no-recursive", dest="recursive", action="store_false",
                    help="Only search top-level input directory")
    pb.add_argument("--out", default="./phoenix_batch",
                    help="Output root directory (default: ./phoenix_batch)")
    pb.add_argument("--continue-on-error", action="store_true", default=True,
                    help="Continue processing other files if one export fails (default: on)")
    pb.add_argument("--stop-on-error", dest="continue_on_error", action="store_false",
                    help="Stop batch immediately on first error")

    return p


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()

    dispatch = {
        "info":   cmd_info,
        "frames": cmd_frames,
        "video":  cmd_video,
        "udp":    cmd_udp,
        "phoenix": cmd_phoenix,
        "phoenix-batch": cmd_phoenix_batch,
    }
    dispatch[args.cmd](args)


if __name__ == "__main__":
    main()
