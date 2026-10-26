# Grafana Alloy → Tempo → Grafana

frame-trace sends to Grafana Alloy, Alloy forwards to Grafana Tempo, and Grafana shows the traces.
This is the shape of a cluster where Alloy runs as the agent next to your services.

```text
frame-trace --OTLP/HTTP:4318--> Alloy --OTLP/gRPC:4317--> Tempo <--TraceQL-- Grafana
```

| Service | Image | Port on your machine |
| --- | --- | --- |
| Alloy | `grafana/alloy:v1.21.2` | 4318 (OTLP/HTTP), 12345 (Alloy web page) |
| Tempo | `grafana/tempo:3.1.1` | 3200 (Tempo HTTP API) |
| Grafana | `grafana/grafana:13.2.4` | 3000 |

| File | What |
| --- | --- |
| `docker-compose.yml` | The three services. |
| `config.alloy` | Alloy: receive OTLP, batch, send to Tempo. |
| `tempo.yaml` | Tempo as a single binary with local storage. |
| `grafana/datasources.yaml` | The Tempo data source. |
| `grafana/dashboards.yaml` | Loads the `frame-trace` dashboard from `../grafana-lgtm/dashboards`. |

> **Warning:** Grafana opens without a login, as a viewer that can use Explore. Use this on your own machine only.

## Start

Run this from this directory:

```sh
docker compose up -d
```

The first start downloads about 690 MB of images. After that, all three are ready in about 3 seconds.

## Send a trace

From the `frame-trace` directory:

```sh
frame-trace block --offline fixtures/replay/people-traced --otlp
```

```text
...
exported 633 spans to http://127.0.0.1:4318
```

`exported` means Alloy accepted the spans.
Tempo can search the trace about a second later.

## Open it

- <http://127.0.0.1:3000> opens the `frame-trace` dashboard.
- **Explore** → **Tempo** runs TraceQL queries.
- <http://127.0.0.1:12345> shows Alloy's pipeline and whether each part is healthy.

To check from a terminal that the trace reached Tempo:

```sh
curl -s 'http://127.0.0.1:3200/api/search?tags=service.name%3Dframe-trace' | grep -o '"rootTraceName":"[^"]*"'
```

```text
"rootTraceName":"block #4 (0x06618995…0637)"
```

## Stop

```sh
docker compose down -v
```

`-v` also deletes the `tempo-data` volume, which holds the traces.
Without it, the traces are still there next time.
