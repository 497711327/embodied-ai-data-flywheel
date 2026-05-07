#!/usr/bin/env bash
set -euo pipefail

# Stop PTP synchronization services.

sudo systemctl stop edge-phc2sys.service || true
sudo systemctl stop edge-ptp4l.service || true

echo "Stopped services: edge-ptp4l, edge-phc2sys"
