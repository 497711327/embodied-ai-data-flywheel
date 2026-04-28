# labeling/ — 标注平台

只做"对接、预标注、QA、回流"，不重复造标注 UI。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `adapters/` | 多平台适配：CVAT / Label Studio / Scale / SuperAnnotate / V7；统一任务下发与回收 |
| `preflight/` | 预标注：用现网模型 + SAM / Grounding-DINO / VLM 给一遍弱标签，人只做修正 |
| `consensus/` | 多人标注合并：投票、IoU 仲裁、专家裁决；输出可信度分 |
| `ontology/` | 标签体系版本化：类别树、属性、关系；变更走 ADR + 迁移脚本 |
| `export/` | 把标注结果回写到 `storage/lake/silver` 并登记 lineage |
| `qa/` | 抽样质检、金标集（gold set）回放、标注员能力分布 |

## 工业级实践要点

1. **预标注是降本核心**：成熟模型 + SAM / VLM 的 zero-shot 能把人工成本降一半以上。
2. **Ontology 严格版本管理**：类别变更必走 ADR，旧数据要么迁移要么标 `legacy_v1`。
3. **金标集驱动 QA**：每天混入 1–5% 已知答案的样本回打分，监控标注员漂移。
4. **任务粒度小**：单条任务不超过几分钟；长任务拆成"框 + 属性 + 跟踪"流水线。
5. **结果回流要带 lineage**：每个 label record 必须能追到源 frame、标注员、模型预标注版本。
6. **隐私**：含人脸 / 车牌的样本进标注前必须经过 `edge/privacy` 或 `processing/enrichment` 脱敏。

## 与其它层的契约

* 输入：`processing/sampling` 选出的候选样本 + `feedback/` 的失败样本。
* 输出：`storage/lake/silver` 的标签表 + `training/datasets/` 引用。
