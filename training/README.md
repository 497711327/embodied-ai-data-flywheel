# training/ — 训练管线

把"数据集 + 代码 + 配置"变成"模型 + 评测报告"，全部可复现。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `experiments/` | Hydra / OmegaConf 配置；每次实验自动记录 git commit + dataset version + 配置 hash |
| `pipelines/` | Argo Workflows / Kubeflow Pipelines / Ray，分布式训练编排 |
| `models/` | 业务模型代码：感知、预测、规划、端到端、世界模型、VLA、操作策略 |
| `registry/` | 模型注册表：MLflow / W&B；模型工件、性能、依赖、合规标签 |
| `evaluation/` | 离线评测：开 / 闭环、回放、场景级指标、长尾切片报表 |
| `datasets/` | dataset manifest（指向 `storage/lake/gold`），不复制物理数据 |
| `ci/` | 模型 CI：小数据冒烟、单元测试、评测回归门禁 |

## 工业级实践要点

1. **Dataset = query**：训练任务的 dataset 字段只存一个版本号 / 查询，禁止本地拷贝改后训练。
2. **三件套绑定**：模型构件必须同时记录 `git_commit` + `dataset_version` + `config_hash`，缺一不报存。
3. **评测早于上线**：每个候选模型都要跑回归评测（开环 + 闭环 replay），未通过门禁不能进入 `inference/`。
4. **长尾切片报表**：评测不能只看总指标，必须出"夜间 / 雨 / 长尾类别 / 特定路口"切片。
5. **闭环仿真**：感知 / 规划模型必须用 `feedback/replay/` 在历史 session 上跑一遍闭环，验证不会引入回归。
6. **可重训练**：任何过往模型都能仅凭 registry 里的元数据被重新构建。

## 与其它层的契约

* 输入：`storage/lake/gold` + `labeling/export/` + `feedback/` 失败样本作为加权样本。
* 输出：模型工件 + `evaluation/` 报告 → `inference/` 灰度发布。
