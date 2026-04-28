# docs/ — 架构与文档

| 目录 | 用途 |
| --- | --- |
| `architecture/` | 系统架构、数据流、模块边界、技术选型说明 |
| `data_contracts/` | 跨服务消息 / 事件 / Manifest 字段说明（与 `ingestion/schemas/` 对应） |
| `operations/` | 运维手册：上线、回滚、扩缩容、应急处置、容量规划 |
| `runbooks/` | 故障 Runbook：每个常见告警一篇，写明症状 / 排查 / 恢复 |
| `adr/` | Architecture Decision Records：每个重大技术决策一篇，记录上下文与取舍 |

## 写作约定

1. ADR 用 [MADR](https://adr.github.io/madr/) 风格，文件名 `NNNN-title.md`。
2. Runbook 模板：症状 → 影响范围 → 立即缓解 → 根因排查 → 长期修复 → 相关告警。
3. 数据契约文档优先以 schema 文件为准，文档只解释字段语义。
4. 文档变更与代码变更同 PR，避免文档滞后。
