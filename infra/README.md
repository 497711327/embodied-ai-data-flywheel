# infra/ — 基础设施

所有跨服务的运行时与平台能力。**业务代码不应自己写 K8s YAML / Terraform，全部沉淀到这里**。

## 子目录

| 目录 | 工业级职责 |
| --- | --- |
| `docker/` | 各服务镜像 Dockerfile（dev / runtime / GPU），多阶段构建 |
| `k8s/` | Helm charts、Kustomize overlays、按 dev/staging/prod 分环境 |
| `terraform/` | 云资源：VPC、对象存储、IAM、Kafka、数据库、GPU 节点池 |
| `argo/` | Argo Workflows / Argo Events：ETL、训练、评测、发布工作流 |
| `ci/` | GitHub Actions / GitLab CI：build、test、schema check、镜像签名、SBOM |
| `observability/` | OpenTelemetry collector、Prometheus、Grafana、Loki、Tempo、Sentry |
| `secrets/` | 外部 Secret 引用（Vault / AWS Secrets Manager），仓库不存明文 |

## 工业级实践要点

1. **环境一致**：dev / staging / prod 用同一组 Helm values 模板，差异只在 overlay。
2. **GitOps**：K8s 由 Argo CD / Flux 同步；任何上线变更走 PR + review。
3. **Secrets 不进仓库**：用 External Secrets Operator + Vault；`secrets/` 只存引用 manifest。
4. **可观测三件套必备**：metrics（Prom）+ logs（Loki）+ traces（Tempo），所有服务接 OTel SDK。
5. **GPU 调度**：训练用 Kueue / Volcano 做队列与配额；推理用 KServe / Seldon。
6. **供应链安全**：镜像签名（cosign）、SBOM（syft）、漏扫（trivy）进 CI 门禁。
