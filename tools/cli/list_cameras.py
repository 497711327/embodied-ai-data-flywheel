"""
List all USB/V4L2 camera devices available on this machine.

Usage:
    python tools/cli/list_cameras.py
    python tools/cli/list_cameras.py --max-index 10

Output shows device index, resolution, and FPS so you can copy the
correct device_id values into edge/configs/cameras.yaml.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

try:
    import cv2
except ImportError:
    print("ERROR: opencv-python-headless is not installed.")
    print("       Run: pip install opencv-python-headless")
    sys.exit(1)


def probe_camera(index: int) -> dict | None:
    """
    Try to open /dev/videoN and read one frame.
    Returns a dict with device info, or None if the device is unavailable.
    """
    cap = cv2.VideoCapture(index, cv2.CAP_V4L2)
    if not cap.isOpened():
        cap.release()
        return None

    ret, frame = cap.read()
    if not ret or frame is None:
        cap.release()
        return None

    info = {
        "device_id": index,
        "device_path": f"/dev/video{index}",
        "width": int(cap.get(cv2.CAP_PROP_FRAME_WIDTH)),
        "height": int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT)),
        "fps": cap.get(cv2.CAP_PROP_FPS),
        "backend": cap.getBackendName(),
    }
    cap.release()
    return info


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--max-index",
        type=int,
        default=8,
        help="Probe /dev/video0 through /dev/video<N> (default: 8)",
    )
    args = parser.parse_args()

    print(f"Probing /dev/video0 … /dev/video{args.max_index} …\n")

    found: list[dict] = []
    for i in range(args.max_index + 1):
        info = probe_camera(i)
        if info:
            found.append(info)

    if not found:
        print("No camera devices found.")
        print()
        print("Possible reasons (WSL/Linux):")
        print("  • No USB camera connected.")
        print("  • Camera not forwarded to WSL (see docs/wsl_camera_capture.md).")
        print("  • Missing udev permissions — try: sudo usermod -aG video $USER")
        sys.exit(1)

    print(f"Found {len(found)} camera(s):\n")
    header = f"{'Index':>5}  {'Device':<15}  {'Resolution':<14}  {'FPS':>6}  {'Backend'}"
    print(header)
    print("─" * len(header))
    for d in found:
        print(
            f"{d['device_id']:>5}  {d['device_path']:<15}  "
            f"{d['width']}x{d['height']:<8}  {d['fps']:>6.1f}  {d['backend']}"
        )

    print()
    print("Copy device_id values into edge/configs/cameras.yaml.")
    print()
    print("Example snippet:")
    for d in found:
        print(f"  - name: camera_X")
        print(f"    device_id: {d['device_id']}")
        print(f"    width: {d['width']}")
        print(f"    height: {d['height']}")
        print(f"    fps: {int(d['fps']) if d['fps'] > 0 else 30}")
        print()


if __name__ == "__main__":
    main()
