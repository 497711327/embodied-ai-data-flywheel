# processing/ — ETL & 数据挖掘

把 raw MCAP 变成可训练的 dataset。这一层是数据飞轮"飞起来"的关键：corner case 挖掘、自动标签、主动采样都在这里。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `jobs/` | Spark / Ray / Flink 批 & 流作业，统一以 Argo Workflows / Airflow 编排 |
| `decoders/` | MCAP / rosbag → Parquet / Tensor / image shard；点云去畸变；图像去 bayer / ISP |
| `enrichment/` | 自动标签：场景分类、天气、自车状态、感知伪标签、文本场景描述（VLM） |
| `mining/` | Corner case 挖掘：感知失败、规划接管、跟车异常、OOD 检测、文本/语义检索 |
| `sampling/` | 主动学习采样：不确定性、多样性（Coreset）、平衡稀有类别 |
| `versioning/` | 数据集版本：lakeFS / DVC，dataset = query + git commit + freeze time |

## 工业级实践要点

1. **解码一次，到处用**：raw MCAP 解码后写 Iceberg/Parquet（bronze），下游所有任务读 bronze，不再重复解码。
2. **Embedding 优先**：在 enrichment 阶段给每帧图像/点云算一个全局 embedding（CLIP/DINO/Lidar foundation model），后续场景检索、去重、聚类都用它。
3. **Corner case 是飞轮燃料**：mining 既靠规则（接管、AEB），也靠模型（不确定度、disagreement、OOD），还靠自然语言查询（"夜里红色卡车从右侧切入"）。
4. **采样和训练解耦**：`sampling/` 输出是 dataset manifest，不是 tensor；训练 job 自己读 manifest，方便复用。
5. **数据集 = query**：不要把数据物理拷贝到训练机；让 dataset 版本只是一个 query + commit hash，物理数据通过 cache 层提供。
6. **回归测试集不可变**：评测集冻结后写 lakeFS tag，模型迭代不能触碰。

## 与其它层的契约

* 输入：`storage/raw` + `storage/lake/bronze`。
* 输出：`storage/lake/{silver,gold}` + `feedback/` 候选样本队列 + `labeling/` 预标注任务。
