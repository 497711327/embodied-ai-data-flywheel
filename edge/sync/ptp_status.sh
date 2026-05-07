#!/usr/bin/env bash
set -euo pipefail

# Show quick health of PTP stack and recent logs.

echo "== systemd status =="
sudo systemctl --no-pager --full status edge-ptp4l.service edge-phc2sys.service || true

echo
echo "== recent ptp4l logs =="
sudo journalctl -u edge-ptp4l.service -n 40 --no-pager || true

echo
echo "== recent phc2sys logs =="
sudo journalctl -u edge-phc2sys.service -n 40 --no-pager || true

echo
echo "== kernel clocks =="
python3 - <<'PY'
import time
print(f"CLOCK_REALTIME: {time.time():.6f}")
if hasattr(time, "CLOCK_TAI"):
    print(f"CLOCK_TAI:      {time.clock_gettime(time.CLOCK_TAI):.6f}")
else:
    print("CLOCK_TAI:      unavailable in this Python build")
print(f"CLOCK_MONOTONIC:{time.clock_gettime(time.CLOCK_MONOTONIC):.6f}")
PY
