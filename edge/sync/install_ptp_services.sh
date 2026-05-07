#!/usr/bin/env bash
set -euo pipefail

# Install and configure linuxptp services for a capture node.
# This script creates two systemd services:
#   1) edge-ptp4l.service   (NIC PHC <- PTP GM)
#   2) edge-phc2sys.service (CLOCK_TAI <- NIC PHC)
#
# Usage:
#   ./sync/install_ptp_services.sh --iface enx0826ae341be5
#   ./sync/install_ptp_services.sh --iface eth0 --domain 0 --transport UDPv4 --tai-offset -37

IFACE=""
DOMAIN="0"
TRANSPORT="UDPv4"
TAI_OFFSET="-37"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --iface)
      IFACE="${2:-}"
      shift 2
      ;;
    --domain)
      DOMAIN="${2:-}"
      shift 2
      ;;
    --transport)
      TRANSPORT="${2:-}"
      shift 2
      ;;
    --tai-offset)
      TAI_OFFSET="${2:-}"
      shift 2
      ;;
    -h|--help)
      cat <<'EOF'
Usage:
  install_ptp_services.sh --iface <net_if>
Options:
  --iface <name>       Network interface connected to GM (required)
  --domain <n>         PTP domainNumber (default: 0)
  --transport <mode>   UDPv4 or L2 (default: UDPv4)
  --tai-offset <sec>   phc2sys -O value (default: -37)
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -z "$IFACE" ]]; then
  echo "ERROR: --iface is required" >&2
  exit 1
fi

if ! ip link show "$IFACE" >/dev/null 2>&1; then
  echo "ERROR: interface not found: $IFACE" >&2
  exit 1
fi

if [[ "$TRANSPORT" != "UDPv4" && "$TRANSPORT" != "L2" ]]; then
  echo "ERROR: --transport must be UDPv4 or L2" >&2
  exit 1
fi

echo "[1/5] Installing dependencies (linuxptp, ethtool)..."
sudo apt-get update
sudo apt-get install -y linuxptp ethtool

echo "[2/5] Checking timestamping capabilities on $IFACE..."
ethtool -T "$IFACE" || true

echo "[3/5] Writing ptp4l config..."
PTP4L_CONF="/etc/linuxptp/edge-ptp4l-${IFACE}.conf"

sudo mkdir -p /etc/linuxptp
sudo tee "$PTP4L_CONF" >/dev/null <<EOF
[global]
# Local node should lock to upstream GM.
slaveOnly               1
domainNumber            ${DOMAIN}
network_transport       ${TRANSPORT}
time_stamping           hardware

# Conservative defaults for switched Ethernet.
delay_mechanism         E2E
announceReceiptTimeout  3
syncReceiptTimeout      3
logging_level           6

[${IFACE}]
EOF

echo "[4/5] Installing systemd services..."

sudo tee /etc/systemd/system/edge-ptp4l.service >/dev/null <<EOF
[Unit]
Description=Edge PTP sync (ptp4l)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/sbin/ptp4l -f ${PTP4L_CONF} -i ${IFACE} -m -s
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
EOF

sudo tee /etc/systemd/system/edge-phc2sys.service >/dev/null <<EOF
[Unit]
Description=Edge PTP sync (phc2sys -> CLOCK_TAI)
After=edge-ptp4l.service
Requires=edge-ptp4l.service

[Service]
Type=simple
ExecStart=/usr/sbin/phc2sys -s ${IFACE} -c CLOCK_TAI -O ${TAI_OFFSET} -m
Restart=always
RestartSec=2

[Install]
WantedBy=multi-user.target
EOF

echo "[5/5] Enabling services..."
sudo systemctl daemon-reload
sudo systemctl enable edge-ptp4l.service edge-phc2sys.service

echo
echo "Install complete. Next steps:"
echo "  ./sync/start_ptp.sh"
echo "  ./sync/ptp_status.sh"
echo "  ./sync/verify_capture_clock.sh"
echo "  # remove later: ./sync/uninstall_ptp_services.sh --purge-config"
echo
echo "Services:"
echo "  edge-ptp4l.service"
echo "  edge-phc2sys.service"
