# ingestion/ — 上行接入层

把 fleet 上行的数据安全、可追溯地接入数据湖。
原则：**只接收、只校验、只登记，不做业务转换**。所有重处理放在 `processing/`。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `gateway/` | 接入网关：gRPC / HTTPS，预签名 URL 上传到对象存储；签名校验、限流、设备鉴权（mTLS / JWT） |
| `streaming/` | Kafka / Pulsar 流：每个 topic 对应一类事件（session.opened、chunk.uploaded、quality.report …） |
| `schemas/` | Schema Registry 源：Avro / Protobuf / JSON Schema；CI 强制向后兼容 |
| `validators/` | 入湖前校验：schema 合法、时间戳单调、必填字段、传感器配置 hash 在白名单 |
| `dedup/` | 幂等：按 `(device_id, session_id, chunk_id)` 去重，处理网络重传 |
| `catalog/` | Session Catalog：注册一段采集的元数据（车/机器人 ID、地点、天气、配置、外参、git commit） |

## 工业级实践要点

1. **Schema First**：所有跨服务事件先在 `schemas/` 落 Proto/Avro，CI 用 Buf / `schema-registry-cli` 校验兼容性，禁止破坏性变更。
2. **数据先落对象存储再发事件**：Gateway 拿到分块后写 S3，再向 Kafka 发 `chunk.uploaded` 事件，下游订阅事件做后续处理（CDC 风格）。
3. **幂等关键**：fleet 网络抖动是常态；按 chunk hash 去重，重复上传不应导致重复入湖。
4. **明确入湖契约**：`validators/` 不通过的数据进入 `quarantine/` 而非丢弃，方便排查。
5. **多租户**：device → fleet → org 三级租户，所有接入接口带 `tenant_id`，下游存储和权限按租户隔离。
6. **观测**：上行成功率、字节速率、端到云延迟、Schema 校验失败率必须有 SLO 看板。

## 与其它层的契约

* 写入：对象存储 raw bucket（路径见 `storage/raw/`）。
* 发事件：`session.*`、`chunk.*`、`quality.*` 到 Kafka，topic schema 在 `schemas/`。
* 不直接调下游服务，下游通过订阅事件解耦。
