# edge/ — 车端 / 机端采集运行时

负责在车 / 机器人本地完成"传感器接入 → 时间同步 → 录制 → 触发 → 缓冲 → 上行 → OTA"。
不强绑定 ROS2：可以用纯 C++/Rust 实现，也可以用 ROS2 / Cyclone DDS / Zenoh，按硬件平台选。
本仓库的 [`ros2_ws/`](../ros2_ws) 是其中一种实现示例（笔记本/开发板上跑 ROS2 Humble）。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `agents/` | 设备侧主进程（Vehicle Agent / Robot Agent），生命周期、配置拉取、上行调度、心跳 |
| `drivers/` | 传感器抽象层：camera / lidar / radar / imu / gnss / can / 力矩 / 触觉，统一 `SensorFrame` 接口 |
| `recorder/` | 高性能录制器，建议 MCAP；分段、压缩（zstd）、按 topic 分流、按事件触发分包 |
| `triggers/` | 事件触发录制：硬刹、TOR、规划失败、感知不一致、长尾场景规则、模型不确定度 |
| `sync/` | 时间同步：PTP/IEEE-1588、PPS、硬件触发、driver 内部 sync queue |
| `buffer/` | Store-and-forward 环形缓冲；断网续传、磁盘配额、优先级队列 |
| `privacy/` | 端侧脱敏：人脸 / 车牌 / 私人地址模糊；地理围栏屏蔽 |
| `ota/` | 配置和模型 OTA：分组发布、灰度、回滚、签名校验 |
| `configs/` | 设备配置（车型、传感器布局、外参版本、录制策略） |

## 工业级实践要点

1. **录制格式选 MCAP**：自描述 schema、自带索引、跨语言支持好，比 rosbag2 更适合大规模数据湖。
2. **Trigger 优先于全量录制**：长尾场景靠规则 + 模型不确定度抓取，而不是 7×24 全录。
3. **数据上行不走 ROS topic**：用独立上行进程读 MCAP 分段，gRPC + 分块 + 断点续传，避免影响实时链路。
4. **配置驱动**：传感器布局 / topic / 外参 / 触发规则全部配置化，配置带版本和 hash，写入每段数据的 metadata。
5. **隐私从端侧开始**：合规要求高的场景（欧盟 GDPR、车队人脸）必须在出车前完成脱敏。
6. **可观测**：每条 session 在端侧就生成 trace ID，向云端 OTel collector 汇报采集质量指标。

## 与其它层的契约

* 上行包：MCAP 分段文件 + `session.manifest.json` + `quality.preflight.json`，约定见 `ingestion/schemas/`。
* 下行任务：`feedback/triggers/` 生成的"重采集任务"通过 OTA 通道下发到 `agents/`。
