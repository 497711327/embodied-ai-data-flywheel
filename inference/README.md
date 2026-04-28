# inference/ — 推理服务

把模型安全推上车 / 机器人，并把推理过程的数据回流到飞轮。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `runtimes/` | Triton / TensorRT / ONNX Runtime / TorchServe；端侧 vs 云侧两套 runtime 适配 |
| `gateway/` | KServe / Seldon / 自建网关；mTLS、限流、A/B、版本路由 |
| `shadow/` | 影子模式：新模型在历史 session 或在线流量上并行推理，不影响主链路 |
| `canary/` | 灰度发布：按车队 / 区域 / 比例放量，自动回滚阈值 |
| `monitoring/` | 在线监控：延迟、吞吐、置信度分布、漂移检测、错误样本采样回流 |

## 工业级实践要点

1. **车端推理 ≠ 云端推理**：端侧 runtime（TensorRT / SNPE / Ascend）和云侧 runtime（Triton / vLLM）是两套，模型构件需要同时产出。
2. **必须有 shadow**：上线前用 `shadow/` 把候选模型在最近 N 天的真实数据上回放，看新增 / 修复 / 回归。
3. **灰度自动回滚**：当回归指标超过阈值（如接管率上升、置信度分布偏移）自动回滚，不靠人。
4. **不一致 = 信号**：影子模型与主模型不一致的样本是 `feedback/failure_mining/` 的高优先级输入。
5. **采样回流**：在线推理结果按规则采样（低置信度、OOD、特定场景）写回 `storage/raw` 或独立 inference-log bucket。
6. **PII 不出端**：上传只携带必要字段；图像 / 点云在上传前脱敏。

## 与其它层的契约

* 输入：`training/registry` 模型工件 + `infra/` 的部署能力。
* 输出：在线指标 + 采样样本 → `feedback/`。
