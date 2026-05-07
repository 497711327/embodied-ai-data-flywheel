# PTP Sync Scripts for Edge Capture

This folder contains operational scripts to set up and verify PTP time sync on the Ubuntu capture node.

## Files

- `install_ptp_services.sh`: install `linuxptp`, generate config, install systemd services.
- `start_ptp.sh`: start/restart PTP services.
- `stop_ptp.sh`: stop PTP services.
- `uninstall_ptp_services.sh`: remove PTP systemd services (optional config/package cleanup).
- `ptp_status.sh`: show service status, recent logs, and kernel clock values.
- `verify_capture_clock.sh`: verify latest capture manifest is marked as `CLOCK_TAI/PTP`.

## Architecture

Two services are installed:

1. `edge-ptp4l.service`
- Sync NIC PHC (hardware clock) to upstream PTP Grandmaster.

2. `edge-phc2sys.service`
- Sync system `CLOCK_TAI` from NIC PHC.

Your capture program then timestamps camera/UDP data in `CLOCK_TAI` and records metadata in `session.manifest.json`.

If the selected interface has no PTP Hardware Clock (common for USB Ethernet adapters), the installer falls back to software timestamping:

- `ptp4l` runs in software mode
- `edge-phc2sys.service` becomes a no-op placeholder
- precision is lower than a true PHC-backed NIC

## 1) Install

Run once on the capture machine:

```bash
cd /home/teto/workspace/embodied-ai-data-flywheel/edge
./sync/install_ptp_services.sh --iface enx0826ae341be5
```

Optional parameters:

```bash
./sync/install_ptp_services.sh \
  --iface enx0826ae341be5 \
  --domain 0 \
  --transport UDPv4 \
  --tai-offset -37
```

Notes:
- `--transport` can be `UDPv4` or `L2`.
- `--tai-offset` default `-37` matches current TAI-UTC leap-second offset assumption in code.
- On interfaces without PHC, install will automatically switch to software timestamping.

## 2) Start

```bash
./sync/start_ptp.sh
```

## 3) Check Runtime Health

```bash
./sync/ptp_status.sh
```

You want to see both services active and `ptp4l` / `phc2sys` logs without large unstable offsets.

## 4) Verify Capture Metadata

After recording a session:

```bash
./sync/verify_capture_clock.sh
```

This checks the newest `session.manifest.json` under:

- `/home/teto/workspace/embodied-ai-data-flywheel/data/raw`

Or specify one manifest explicitly:

```bash
./sync/verify_capture_clock.sh --manifest /path/to/session.manifest.json
```

Expected good result:

- `ptp_synced: true`
- `clock_source` contains `CLOCK_TAI/PTP`

## 5) Stop

```bash
./sync/stop_ptp.sh
```

## 6) Uninstall

Remove only systemd services (keep config files):

```bash
./sync/uninstall_ptp_services.sh
```

Remove services and generated `/etc/linuxptp/edge-ptp4l-*.conf`:

```bash
./sync/uninstall_ptp_services.sh --purge-config
```

Also uninstall `linuxptp` package:

```bash
./sync/uninstall_ptp_services.sh --purge-config --purge-linuxptp
```

## Troubleshooting

1. Interface has no hardware timestamp support
- Check `ethtool -T <iface>` output.
- If hardware timestamps are unavailable, precision will be worse.
- USB Ethernet adapters often fall into this category.

2. Services are running but not locking
- Confirm correct network interface connected to domain controller / GM.
- Check PTP domain and transport mode match GM settings.

3. Capture still shows `CLOCK_TAI/NTP`
- Verify both services are active.
- Record a new session after PTP is stable, then run `verify_capture_clock.sh`.
