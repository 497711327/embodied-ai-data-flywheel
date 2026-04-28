# Embodied AI Data Flywheel

工业级数据飞轮，面向自动驾驶 / 具身智能（VLA、世界模型、操作策略）。
目标：让"采集 → 入湖 → 处理 → 标注 → 训练 → 推理 → 反馈 → 再采集"全链路可复现、可观测、可回滚。

## 顶层结构

```text
data-flywheel/
├── edge/         # 车端 / 机端采集运行时（C++ / Rust / Python）
├── ingestion/    # 上行接入：网关、Kafka/Pulsar、Schema Registry
├── storage/      # 数据湖：对象存储 + Iceberg/Delta + 元数据/血缘
├── processing/   # ETL & 数据挖掘：Spark/Ray、解码、增强、Corner Case 挖掘
├── labeling/     # 标注平台对接 + 预标注 + 一致性 QA
├── training/     # 训练管线：实验、模型、评测、模型注册表
├── inference/    # 推理服务：Triton/TensorRT、影子/灰度、监控
├── feedback/     # 主动学习闭环：失败挖掘、重采集、优先级
├── tools/        # 可视化 & 调试：Foxglove、rerun、Dashboards、CLI
├── infra/        # Docker / K8s / Terraform / Argo / Observability
└── docs/         # 架构、数据契约、Runbook、ADR
```

## 整体数据流

```text
                ┌──────────────────────────────────────────┐
                │           Fleet (车 / 机器人)             │
                │  edge/ : 驱动 + recorder(MCAP) + buffer  │
                │           triggers + privacy + OTA        │
                └──────────────┬───────────────────────────┘
                               │ 分块上传 (HTTPS/gRPC, 断点续传)
                               ▼
   ┌──────────────────────────────────────────────────────────┐
   │ ingestion/  Gateway → Kafka/Pulsar → Schema Registry      │
   │             校验 / 去重 / Session Catalog                 │
   └──────────────┬───────────────────────────────────────────┘
                  ▼
   ┌──────────────────────────────────────────────────────────┐
   │ storage/    Raw(S3/MinIO + MCAP) + Iceberg/Delta Lake     │
   │             Metadata DB + DataHub Lineage + Retention     │
   └──────────────┬───────────────────────────────────────────┘
                  ▼
   ┌──────────────────────────────────────────────────────────┐
   │ processing/ Spark/Ray ETL → 解码 → Embedding → Mining     │
   │             Active Sampling → DVC/lakeFS dataset versions │
   └─────┬────────────────────────────────────┬───────────────┘
         ▼                                    ▼
   ┌────────────┐                     ┌──────────────────────┐
   │ labeling/  │                     │ training/            │
   │ Pre-label  │  ←──── ontology ───▶│ Hydra + Argo/Kubeflow│
   │ Adapters   │                     │ MLflow / W&B 注册表   │
   │ QA         │                     │ 离线评测 + Replay     │
   └─────┬──────┘                     └──────────┬───────────┘
         │                                       ▼
         │                            ┌──────────────────────┐
         │                            │ inference/           │
         │                            │ Triton / TensorRT    │
         │                            │ Shadow → Canary → GA │
         │                            └──────────┬───────────┘
         │                                       ▼
         │                            ┌──────────────────────┐
         └────────────────────────────│ feedback/            │
                                      │ 失败挖掘 / 不一致     │
                                      │ 重采集触发 / 优先级   │
                                      └──────────┬───────────┘
                                                 │ 下发采集任务
                                                 ▼
                                          回到 edge/ Fleet
```

横向支撑：

* `tools/`     —— Foxglove / rerun.io / Grafana / Streamlit / 数据飞轮 CLI
* `infra/`     —— Docker、K8s（Helm）、Argo Workflows/Events、Terraform、OTel/Prom/Loki/Tempo
* `docs/`      —— 架构、数据契约（Avro/Proto/JSON Schema）、Runbook、ADR

## 关键设计约束

1. **Raw immutable**：原始包（MCAP / Parquet 等）写入对象存储后不可改，所有派生数据通过 lineage 追溯。
2. **Schema first**：所有跨服务消息先在 `ingestion/schemas/` 定义，再写代码；Schema Registry 强制兼容性。
3. **Session 是一等公民**：每段数据都属于一个 `session_id`，绑定车/机器人 ID、传感器配置 hash、标定版本、git commit、地图版本、天气、场景标签。
4. **数据湖分层**：raw → bronze（解码）→ silver（对齐+特征）→ gold（训练就绪）。
5. **数据集即代码**：dataset version 用 lakeFS / DVC 管理，由 query + git commit 唯一确定。
6. **训练-推理-反馈三位一体**：每个上线模型必须能在 shadow 模式回放历史 session，失败样本自动进入 `feedback/`。
7. **可观测**：traceID 从车端事件穿到训练 job、推理请求和反馈任务（OpenTelemetry）。
8. **隐私合规**：人脸 / 车牌 / 私人地址在 edge 或 ingestion 阶段脱敏，原始数据访问通过 IAM + 审计日志。

## 各层入口

* [edge/](edge/README.md) —— 车端 / 机端采集
* [ingestion/](ingestion/README.md) —— 上行接入
* [storage/](storage/README.md) —— 数据湖
* [processing/](processing/README.md) —— ETL 与数据挖掘
* [labeling/](labeling/README.md) —— 标注
* [training/](training/README.md) —— 训练
* [inference/](inference/README.md) —— 推理
* [feedback/](feedback/README.md) —— 主动学习闭环
* [tools/](tools/README.md) —— 可视化与调试
* [infra/](infra/README.md) —— 基础设施
* [docs/](docs/README.md) —— 架构与文档

## 推荐技术栈（选型示例，不强绑定）

| 层 | 工业级常见选型 |
| --- | --- |
| edge runtime | C++ / Rust / Python，DDS 中间件可选 Cyclone DDS、Zenoh、eCAL |
| 录制格式 | MCAP（推荐）、Parquet |
| 上行 | gRPC + 分块 + 断点续传 + 预签名 URL |
| 流 | Kafka / Pulsar + Schema Registry（Avro / Protobuf） |
| 对象存储 | S3 / GCS / OSS / MinIO |
| 数据湖表 | Apache Iceberg / Delta Lake / Hudi |
| 元数据 | Postgres + DataHub / OpenMetadata |
| 计算 | Ray / Spark / Flink，调度 Argo Workflows |
| 数据集版本 | lakeFS / DVC |
| 标注 | CVAT / Label Studio / Scale / SuperAnnotate |
| 训练 | PyTorch + Hydra + Lightning / Ray Train，Kubeflow / Argo |
| 模型注册 | MLflow / Weights & Biases |
| 推理 | Triton + TensorRT，KServe / Seldon |
| 仿真回放 | Foxglove Studio、CARLA、Isaac Sim、Drake |
| 监控 | OpenTelemetry + Prometheus + Grafana + Loki + Tempo |

## MVP 实施顺序（建议）

1. **edge**：单设备 MCAP 录制 + 触发 + 上行（先打通一条流）
2. **ingestion + storage/raw**：网关 + 对象存储 + Session Catalog
3. **processing/decoders + storage/lake/bronze**：解码到 Iceberg
4. **tools/cli + tools/foxglove**：能查、能放
5. **processing/enrichment + mining**：自动标签 + corner case
6. **labeling 适配 + training MVP**：跑通一个端到端模型
7. **inference shadow + feedback**：闭环跑起来
8. **infra**：把上面所有都搬到 K8s + GitOps
