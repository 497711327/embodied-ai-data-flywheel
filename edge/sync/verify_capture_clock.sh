#!/usr/bin/env bash
set -euo pipefail

# Verify whether recent edge-capture sessions are marked as PTP-synced.
#
# Usage:
#   ./sync/verify_capture_clock.sh
#   ./sync/verify_capture_clock.sh --manifest /path/to/session.manifest.json
#   ./sync/verify_capture_clock.sh --root /path/to/data/raw

MANIFEST=""
RAW_ROOT="/home/teto/workspace/embodied-ai-data-flywheel/data/raw"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest)
      MANIFEST="${2:-}"
      shift 2
      ;;
    --root)
      RAW_ROOT="${2:-}"
      shift 2
      ;;
    -h|--help)
      cat <<'EOF'
Usage:
  verify_capture_clock.sh [--manifest <session.manifest.json>] [--root <data_raw_dir>]

Options:
  --manifest <file>   Check a specific manifest file
  --root <dir>        Root directory for auto-discovery (default: /home/teto/workspace/embodied-ai-data-flywheel/data/raw)
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -n "$MANIFEST" ]]; then
  if [[ ! -f "$MANIFEST" ]]; then
    echo "ERROR: manifest not found: $MANIFEST" >&2
    exit 1
  fi
else
  MANIFEST="$(find "$RAW_ROOT" -type f -name session.manifest.json -printf '%T@ %p\n' 2>/dev/null | sort -nr | head -n1 | cut -d' ' -f2-)"
  if [[ -z "$MANIFEST" ]]; then
    echo "ERROR: no session.manifest.json found under: $RAW_ROOT" >&2
    exit 1
  fi
fi

python3 - "$MANIFEST" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
obj = json.loads(manifest_path.read_text())

session_id = obj.get("session_id", "n/a")
clock_source = obj.get("clock_source", "n/a")
ptp_synced = obj.get("ptp_synced", False)
offset = obj.get("mono_to_tai_offset_ns", None)
started = obj.get("started_at", "n/a")
ended = obj.get("ended_at", "n/a")

print(f"manifest     : {manifest_path}")
print(f"session_id   : {session_id}")
print(f"started_at   : {started}")
print(f"ended_at     : {ended}")
print(f"clock_source : {clock_source}")
print(f"ptp_synced   : {ptp_synced}")
if offset is not None:
    print(f"mono_to_tai_offset_ns: {offset}")

if ptp_synced and "PTP" in str(clock_source):
    print("RESULT: PASS (capture timestamps are in CLOCK_TAI/PTP)")
    sys.exit(0)

print("RESULT: WARN (capture not marked as PTP-synced; likely CLOCK_TAI/NTP)")
sys.exit(2)
PY
