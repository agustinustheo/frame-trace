# 08. Parity observability stack: what is actually used

Research date: 2026-10-26. Method: `gh search code --owner paritytech` (terms listed below), `gh api` reads of file contents and commit dates, `gh pr/issue view`, and public docs via WebFetch/WebSearch. Dates are last-commit dates on the file unless noted.

## Bottom line

- **Active OTLP tracing today is Grafana LGTM** (`grafana/otel-lgtm`, which bundles Tempo, Loki, Prometheus, Pyroscope and Grafana). Two product codebases in `paritytech/trinity-apps` use it, with OTLP HTTP on 4318 and explicit "LGTM is the only observability stack" language.
- **Standalone Grafana Tempo is dormant in Parity infra.** `paritytech/devops-cloud-infra` scaled Tempo to zero on 2026-10-07 (PR #4602). The preceding issue (closed) says Tempo was "not collecting any traces" and only ever collected Loki's traces. The cluster Tempo still has OTLP and Jaeger receivers enabled, so the config is reusable.
- **Jaeger is legacy for Substrate.** polkadot-sdk removed Jaeger (PR #5830 merged 2024-09-27, PR #5875 "Remove jaeger everywhere" merged 2024-10-04). Nothing replaced it: no OpenTelemetry or OTLP dependency in the root `Cargo.toml`, and `gh search code --repo paritytech/polkadot-sdk otlp` returned nothing.
- **Metrics for node operators are Prometheus + Grafana + Alertmanager**, per the official docs. No tracing, no Loki, no Tempo.

## Recommended frame-trace examples (ranked)

1. **Grafana LGTM (`grafana/otel-lgtm`), primary.** Matches the only live OTLP tracing in Parity code. Ports: 4317 OTLP gRPC, 4318 OTLP HTTP, 3000 Grafana, 3200 Tempo query, 3100 Loki.
   - Pins in use: `grafana/otel-lgtm:0.11.16` (`paritytech/trinity-apps` `repos/identity-backend-community/infra/observability/Dockerfile`, last change 2026-07-20); `0.11.9` (`paritytech/trinity-apps` `apps/web/otel/docker-compose.yml`, 2026-08-01); `latest` (identity-backend `docker/local/docker-compose.yaml`, not reproducible).
   - Evidence: `repos/identity-backend-community/infra/docs/observability-lgtm.md` (2026-07-20) says "LGTM is the only observability stack ... no Datadog, no New Relic" and pre-provisions Prometheus, Tempo, Loki and Pyroscope datasources.
2. **Grafana Tempo fed by an OpenTelemetry Collector or Grafana Alloy, secondary.** This is the Parity ops shape, but it is currently scaled to zero, so present it as the "bring your own Tempo" path.
   - `paritytech/devops-cloud-infra` `kubernetes/tempo/` (chart `tempo-distributed` 3.14.0 from `grafana-community`, 2026-10-19; Redpanda Kafka ingest; OTLP gRPC and HTTP plus Jaeger gRPC, thrift HTTP and thrift compact receivers all enabled).
   - `paritytech/zombienet` `javascript/packages/orchestrator/src/providers/podman/resources/tempoResource.ts` uses `docker.io/grafana/tempo:latest`. Its `static-configs/tempo.yaml` enables Jaeger, Zipkin, OTLP and OpenCensus receivers (2022 config).
   - Grafana Alloy is the cluster agent everywhere (`kubernetes/alloy` chart `alloy` 1.8.1, wrapper 2026-10-15). Its config currently has logs and metrics pipelines only, but it can receive OTLP, so an Alloy example is credible.
3. **Jaeger v2 (OTLP-native), quick-start only.** Keep the current Jaeger example but treat it as a local UI, not the Parity stack. Jaeger v2 accepts OTLP on 4317/4318.
   - Still used by: `paritytech/polkadot-introspector` `jaeger/` (queries the Jaeger UI API, last change 2023-05-09); the zombienet k8s path (`jaegertracing/jaeger-agent:1.28.0` sidecar in `providers/k8s/resources/nodeResource.ts`); the `node` Helm chart's `jaegerAgent` (default tag `latest`, `paritytech/helm-charts` `charts/node/`, last repo commit 2026-09-10).
4. **Loki and Prometheus/Grafana as companions, not trace backends.** Loki is used in production for logs (`devops-cloud-infra` `kubernetes/alloy/configs/logs.alloy`, `sre-docs/.../loki.md` updated 2026-10-08). Prometheus runs via `kube-prometheus-stack` 84.5.0 (2026-10-19) with Thanos. Ship a log-correlation note inside the LGTM example rather than a separate Loki example.

## Stack inventory (what Parity uses)

| Tool | Where used (path, last change) | Use | OTLP traces? |
|---|---|---|---|
| grafana/otel-lgtm | trinity-apps `identity-backend-community/infra/observability/Dockerfile` (2026-07-20), `apps/web/otel/docker-compose.yml` (2026-08-01) | All-in-one traces, logs, metrics, profiles | Yes, 4317/4318 |
| Grafana Tempo | devops-cloud-infra `kubernetes/tempo` (2026-10-19, scaled to 0 since 2026-10-07); `grafana/values-parity-mgmt.yaml` deleted old `tempo.*` datasources; zombienet podman | Trace store | Yes (OTLP, Jaeger, Zipkin in zombienet) |
| Jaeger | polkadot-sdk (removed 2024); helm-charts `charts/node` (`jaegerAgent`); zombienet k8s; applied-engineering doc (2024-07-30); polkadot-introspector | Legacy trace path | Jaeger agent only, no OTLP |
| Zipkin | zombienet `static-configs/tempo.yaml` receiver only | Not used | n/a |
| Grafana / Prometheus | devops `kubernetes/grafana` (chart 13.7.0, 2026-10-19); `kube-prometheus-stack` (2026-10-19); polkadot-sdk `polkadot/grafana` (2024-11-26) and `substrate/scripts/ci/monitoring` (2023-09-04); zombienet podman (`grafana/grafana`, `prom/prometheus`, `:latest`) | Dashboards, metrics | No |
| Loki | devops `kubernetes/loki-distributed` (wrapper 2.9.2, chart 13.7.2, 2026-10-19); `kubernetes/alloy` (2026-10-15); `sre-docs/.../loki.md` (2026-10-08); toaster-infra | Logs | No |
| Grafana Alloy | devops `kubernetes/alloy` (chart 1.8.1, 2026-10-15) | Collector for logs and metrics | Possible, not configured |
| Pyroscope | devops `kubernetes/pyroscope` and helm-charts-internal `pyroscope/` (profiling, chart 0.2.92 upstream) | Profiling | No (profiles only) |
| Sentry | `paritytech/sentry` repo; product frontends (`polkadot-desktop-community`, `t3rminal`, `revx`) | Error reporting | No |
| Thanos | devops `kubernetes/thanos`, `sre-docs/.../thanos.md` | Long-term Prometheus | No |
| substrate-telemetry | devops `kubernetes/substrate-telemetry` (chart 2.3.3, last change 2024-11-07) | Substrate's own node telemetry feed | No (not OTel) |
| Datadog, Honeycomb, New Relic, SigNoz, VictoriaMetrics | Only in docs or skill text (`polkadot-product-engineering/references/observability-and-tracing.md` says "any OTLP backend acceptable"), theme files, or unrelated repos | Not deployed | n/a |
| Elastic/Kibana | `helm-charts-internal/elasticsearch` exists; not investigated further | Unknown | n/a |

## Substrate and polkadot-sdk

- Jaeger removal: PR #5830 (approval subsystems, merged 2024-09-27) and PR #5875 "Remove jaeger everywhere" (merged 2024-10-04). The stated reason was that nobody used it.
- Remaining observability: Prometheus `/metrics` on 9615 (`substrate/utils/prometheus/README.md`), and `tracing` logs. `polkadot/node/gum/README.md` still describes Jaeger spans and is stale.
- Latest release: `polkadot-stable2609` (2026-10-12).

## Public docs

- Node monitoring guide (`docs.polkadot.com/node-infrastructure/run-a-validator/operational-tasks/general-management/`, reached via the wiki redirect): Prometheus v3.0.0, Grafana from apt (latest), Alertmanager v0.28.0-rc.0. No Loki, Tempo, Jaeger or OpenTelemetry.

## Helm charts and zombienet details

- `paritytech/helm-charts` `charts/node`: `node.tracing.enabled` defaults to false. When enabled, it adds a `jaeger-agent` sidecar (default image `jaegertracing/jaeger-agent:latest`) and passes `--jaeger-agent` to the binary. That flag predates the 2024 removal, so the path is probably dead against current binaries (not run). ServiceMonitor is available (`serviceMonitor.enabled`, default false).
- zombienet (pushed 2026-10-20): podman deploys `grafana/tempo:latest`, `grafana/grafana`, `prom/prometheus`. k8s deploys `jaegertracing/jaeger-agent:1.28.0` when `jaegerUrl` is `localhost:6831`, pointing to `tempo-tempo-distributed-distributor.tempo.svc.cluster.local:14250`. `cmdGenerator.ts` still emits `--jaeger-agent`, which is likely stale.

## Not used (do not build examples)

Datadog, Honeycomb, New Relic, SigNoz, VictoriaMetrics, standalone Zipkin, Pyroscope (profiling only), Sentry (errors only), substrate-telemetry (not OTel). Elastic/Kibana was not checked further.

## Caveats

- Tempo's scaled-to-zero state can change. Re-check `paritytech/devops-cloud-infra` `kubernetes/tempo/values.yaml` before writing docs that say Parity runs Tempo.
- Image pins for `otel/opentelemetry-collector-contrib` and `jaegertracing/jaeger` v2 were not found in any Parity repo. The identity-backend e2e uses an otel-collector image, but I did not check its tag.
- `gh search code` hit the rate limit only lightly; a few "no result" outcomes (for example `otlp` in polkadot-sdk) were not cross-checked with a second query.
