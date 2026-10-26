# Grafana LGTM

Grafana, Tempo, Loki, Prometheus and an OpenTelemetry Collector in one container, `grafana/otel-lgtm:0.11.16`.
Parity uses the same image and version for its own local tracing.
This example adds a `frame-trace` dashboard.

| Port | What |
| --- | --- |
| 3000 | Grafana |
| 4318 | OTLP/HTTP, where frame-trace sends traces |
| 4317 | OTLP/gRPC, for other tools |

| File | What |
| --- | --- |
| `docker-compose.yml` | The container, with the dashboard mounted. |
| `provisioning/frame-trace.yaml` | Tells Grafana to load the dashboard. |
| `dashboards/frame-trace.json` | The dashboard. |

> **Warning:** Grafana opens without a login, as a viewer that can use Explore. Use this on your own machine only.

## Start

Run this from this directory:

```sh
docker compose up -d
```

The first start downloads a 535 MB image. After that, Grafana and Tempo are ready in about 3 seconds.

## Send a trace

From the `frame-trace` directory:

```sh
frame-trace block --offline fixtures/replay/people-traced --otlp
```

```text
...
exported 633 spans to http://127.0.0.1:4318
```

The trace can be searched about a second later.

## Open it

- <http://127.0.0.1:3000> opens the `frame-trace` dashboard.
- **Explore** → **Tempo** runs TraceQL queries.
  [Find storage problems with TraceQL](https://agustinustheodorus.com/frame-trace/guides/query-traces-in-grafana.html) has ready ones.

To check from a terminal that the trace arrived:

```sh
curl -s 'http://127.0.0.1:3000/api/datasources/proxy/uid/tempo/api/search?tags=service.name%3Dframe-trace' | grep -o '"rootTraceName":"[^"]*"'
```

```text
"rootTraceName":"block #4 (0x06618995…0637)"
```

## Stop

```sh
docker compose down
```

The container keeps traces inside itself, so stopping it deletes them.
