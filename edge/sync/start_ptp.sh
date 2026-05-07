#!/usr/bin/env bash
set -euo pipefail

# Start or restart PTP synchronization services.

sudo systemctl restart edge-ptp4l.service
sudo systemctl restart edge-phc2sys.service

echo "Started services: edge-ptp4l, edge-phc2sys"
echo
sudo systemctl --no-pager --full status edge-ptp4l.service edge-phc2sys.service || true
