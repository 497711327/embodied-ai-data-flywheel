# Embodied Data Stack (ROS2 Humble)

> Enterprise‑grade multi‑sensor data collection & closed‑loop dataset infrastructure for robotics / autonomous driving / embodied intelligence.

---

# Overview

**Embodied Data Stack** is a ROS2 Humble‑based data infrastructure platform designed for:

* Multi‑sensor data acquisition
* Time synchronization
* TF management
* Recording pipeline (rosbag2)
* Dataset export (KITTI / nuScenes / custom)
* Data quality monitoring
* Closed‑loop dataset iteration

Target users:

* Robotics engineers
* Autonomous driving engineers
* Data infrastructure engineers
* Embodied AI platform developers

---

# Architecture

```
Sensor Drivers
      ↓
Sensor Sync Layer
      ↓
TF Manager
      ↓
Recording Engine (rosbag2)
      ↓
Data Quality Monitor
      ↓
Dataset Exporter
      ↓
Training Pipeline Interface
```

Platform supports:

* Cameras
* LiDAR
* Radar
* IMU
* CAN bus
* GNSS

---

# Repository Structure

```
embodied_data_stack/
│
├── docker/
├── docs/
├── config/
├── scripts/
├── tools/
│
├── ros2_ws/
│   ├── src/
│   │
│   │   ├── platform_msgs/
│   │   ├── sensor_drivers/
│   │   ├── sensor_sync/
│   │   ├── tf_manager/
│   │   ├── recording/
│   │   ├── data_monitor/
│   │   ├── dataset_exporter/
│   │   ├── visualization/
│   │   ├── calibration/
│   │   └── bringup/
│   │
│   ├── build/
│   ├── install/
│   └── log/
│
└── README.md
```

---

# Supported Sensors

| Sensor | Status    |
| ------ | --------- |
| Camera | Supported |
| LiDAR  | Supported |
| Radar  | Supported |
| IMU    | Supported |
| CAN    | Supported |
| GNSS   | Optional  |

---

# Coordinate Frames (TF Tree)

```
map
 └── odom
      └── base_link
           ├── camera_front
           ├── lidar_top
           ├── radar_front
           └── imu_link
```

---

# Topic Naming Convention

Format:

```
/sensor_type/location/data_type
```

Examples:

```
/camera/front/image_raw
/lidar/top/points
/radar/front/object_list
/imu/data
/can/raw
```

---

# QoS Strategy

QoS configuration stored in:

```
config/qos.yaml
```

Example:

```
camera:
  reliability: best_effort
  depth: 5

lidar:
  reliability: reliable
  depth: 10

imu:
  reliability: best_effort
  depth: 50
```

---

# Quick Start

## 1. Create workspace

```
mkdir -p ros2_ws/src
cd ros2_ws
colcon build
```

## 2. Source environment

```
source install/setup.bash
```

## 3. Launch platform

```
ros2 launch bringup system.launch.py
```

---

# Recording Pipeline

Recommended recording topics:

```
/camera/*
/lidar/*
/radar/*
/imu/*
/tf
/tf_static
```

Start recording:

```
bash scripts/record.sh
```

---

# Dataset Export

Supported formats:

* KITTI
* nuScenes
* Custom training format

Example:

```
ros2 run dataset_exporter kitti_exporter
```

Output:

```
image/
velodyne/
calib/
label/
```

---

# Data Quality Monitoring

Monitors:

* Topic frequency
* Delay jitter
* Packet drop rate
* Timestamp drift

Tools:

```
tools/topic_monitor.py
```

---

# Docker Support

Build image:

```
docker build -t embodied_data_stack .
```

Run container:

```
docker compose up
```

---

# Development Roadmap

## Phase 1 — Sensor Integration

* camera driver
* lidar driver
* imu driver
* radar driver

## Phase 2 — Synchronization Framework

* ApproximateTime sync
* ExactTime sync
* timestamp alignment

## Phase 3 — Recording Infrastructure

* rosbag2 recorder node
* trigger recording
* segmented recording

## Phase 4 — Dataset Export Layer

* KITTI exporter
* nuScenes exporter

## Phase 5 — Closed‑Loop Pipeline

* dataset validator
* auto filtering
* active learning interface

---

# Recommended Development Workflow

```
feature/sensor_driver_camera
feature/tf_tree_config
feature/rosbag_pipeline
feature/dataset_exporter
```

---

# Contribution Guidelines

1. Create feature branch
2. Follow topic naming convention
3. Add launch file if new node introduced
4. Add config entry if sensor added
5. Submit merge request

---

# Long‑Term Vision

This platform will evolve into:

* Embodied intelligence dataset engine
* Autonomous driving data infrastructure
* Simulation‑training bridge platform
* Active learning closed‑loop system

---

# License

Apache‑2.0 (recommended)

---

# Maintainers

Embodied Data Infrastructure Team
