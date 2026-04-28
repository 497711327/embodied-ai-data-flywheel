# tools/ — 可视化与调试

工程师效率层。**好工具直接决定数据飞轮跑多快**。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `foxglove/` | Foxglove Studio layouts、自定义面板、扩展（panel / source） |
| `rerun/` | rerun.io 录制脚本，3D / 多模态调试 |
| `dashboards/` | Grafana / Streamlit / Superset 看板：数据质量、模型监控、飞轮 KPI |
| `cli/` | 数据飞轮 CLI（`dfw`）：查询 session、导出片段、跑 ETL、提交标注任务 |
| `notebooks/` | Jupyter / Marimo 分析模板，禁止把生产逻辑留在 notebook |

## 工业级实践要点

1. **统一入口 CLI**：所有人通过一个 `dfw` 命令访问飞轮，不要让大家直接戳 S3 / Kafka。
2. **Foxglove + rerun 组合**：Foxglove 看 ROS / MCAP，rerun 看模型推理的 3D 几何 / 张量。
3. **看板对齐 SLO**：每层（采集成功率、ETL 时延、训练吞吐、模型在线指标）都要有看板。
4. **Notebook 不上生产**：notebook 只做分析；稳定逻辑沉淀到 `processing/` 或 `tools/cli/`。
5. **样本播放器**：能根据 session_id + 时间区间一键拉数据并在浏览器里 replay，是降低排障成本的杀手锏。
