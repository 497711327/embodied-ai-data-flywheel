#!/usr/bin/env bash
set -euo pipefail

# Uninstall PTP systemd services created by install_ptp_services.sh.
#
# Default behavior:
# - stop/disable services
# - remove service unit files
# - keep generated ptp4l config under /etc/linuxptp
#
# Optional:
# - --purge-config: also remove /etc/linuxptp/edge-ptp4l-*.conf
# - --purge-linuxptp: also uninstall linuxptp package
#
# Usage:
#   ./sync/uninstall_ptp_services.sh
#   ./sync/uninstall_ptp_services.sh --purge-config
#   ./sync/uninstall_ptp_services.sh --purge-config --purge-linuxptp

PURGE_CONFIG=0
PURGE_LINUXPTP=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --purge-config)
      PURGE_CONFIG=1
      shift
      ;;
    --purge-linuxptp)
      PURGE_LINUXPTP=1
      shift
      ;;
    -h|--help)
      cat <<'EOF'
Usage:
  uninstall_ptp_services.sh [--purge-config] [--purge-linuxptp]

Options:
  --purge-config     Remove generated ptp4l config files in /etc/linuxptp
  --purge-linuxptp   Uninstall linuxptp package (apt remove)
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

echo "[1/5] Stopping services..."
sudo systemctl stop edge-phc2sys.service || true
sudo systemctl stop edge-ptp4l.service || true

echo "[2/5] Disabling services..."
sudo systemctl disable edge-phc2sys.service edge-ptp4l.service || true

echo "[3/5] Removing service unit files..."
sudo rm -f /etc/systemd/system/edge-phc2sys.service
sudo rm -f /etc/systemd/system/edge-ptp4l.service

echo "[4/5] Reloading systemd..."
sudo systemctl daemon-reload
sudo systemctl reset-failed || true

if [[ "$PURGE_CONFIG" -eq 1 ]]; then
  echo "[5/5] Removing generated ptp4l config files..."
  sudo rm -f /etc/linuxptp/edge-ptp4l-*.conf
else
  echo "[5/5] Keeping ptp4l config files in /etc/linuxptp"
fi

if [[ "$PURGE_LINUXPTP" -eq 1 ]]; then
  echo "Removing linuxptp package..."
  sudo apt-get remove -y linuxptp
fi

echo
echo "Uninstall complete."
if [[ "$PURGE_CONFIG" -eq 0 ]]; then
  echo "Config files kept: /etc/linuxptp/edge-ptp4l-*.conf"
fi
