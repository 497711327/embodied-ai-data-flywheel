# storage/ — 数据湖

工业级数据飞轮的"中央银行"。所有派生数据可被丢弃，唯独 raw 必须不可变、可审计。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `raw/` | 对象存储路径规范：`s3://raw/{tenant}/{fleet}/{device}/{date}/{session_id}/*.mcap`，附 manifest |
| `lake/` | 表格式数据湖：Apache Iceberg / Delta Lake / Hudi；分 bronze/silver/gold |
| `index/` | 元数据索引：Postgres + 全文/向量索引；支持按场景、传感器、时间、地理范围检索 |
| `cache/` | 训练热缓存：高速 NVMe / Alluxio / JuiceFS / WebDataset shard |
| `lineage/` | 数据血缘：DataHub / OpenMetadata / OpenLineage，把 raw → dataset → model 串起来 |
| `retention/` | 生命周期：归档（Glacier）、删除、合规保留、法务保全（legal hold） |

## 数据湖分层（Medallion）

```text
raw     ── 端侧原样（MCAP / 原始二进制）
bronze  ── 解码 + 校时 + 字段标准化（Parquet/Iceberg，按 session 分区）
silver  ── 多传感器对齐 + 特征 + embedding + 自动标签
gold    ── 训练就绪 dataset（带 split、负样本、平衡采样）
```

## 工业级实践要点

1. **路径即契约**：raw 路径一旦确定不能改；变更走新前缀，老数据保留。
2. **Iceberg / Delta 分区策略**：按 `tenant / date / session_id` 分区；过细会导致小文件，过粗会扫描爆炸，按数据量调。
3. **元数据可查询**：业务方按"夜间高速 + 雨天 + 雷达 + 自车速度 > 80"这种 SQL 查得到对应 session_id。
4. **lineage 必接**：每个 dataset 表里都要写它来自哪些 raw session、哪个 ETL job、哪个 git commit。
5. **冷热分层**：最近 30 天放 hot bucket / 缓存层；老数据落归档层；通过 `retention/` 策略自动迁移。
6. **合规优先**：删除 / 撤销同意（GDPR right to be forgotten）必须能从 raw 一路追溯到所有衍生数据。

## 与其它层的契约

* 写入：`ingestion/gateway` 写 raw；`processing/jobs` 写 bronze/silver/gold。
* 读出：`processing/`、`labeling/`、`training/`、`tools/` 通过元数据查询接口拿数据。
