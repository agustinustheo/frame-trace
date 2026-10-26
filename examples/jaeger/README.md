# Jaeger

Jaeger 2.22.0 in one container: the lightest way to look at a frame-trace trace.

| Port | What |
| --- | --- |
| 16686 | Jaeger web page |
| 4318 | OTLP/HTTP, where frame-trace sends traces |

## Start

Run this from this directory:

```sh
docker compose up -d
```

Jaeger is ready in under a second once the image is downloaded (51 MB).

## Send a trace

From the `frame-trace` directory:

```sh
frame-trace block --offline fixtures/replay/people-traced --otlp
```

```text
...
exported 633 spans to http://127.0.0.1:4318
```

## Open it

Open <http://127.0.0.1:16686>, pick the service `frame-trace` and click **Find Traces**.
[Your first trace](https://agustinustheodorus.com/frame-trace/getting-started/first-trace.html) shows what you will see.

To check from a terminal that the trace arrived:

```sh
curl -s 'http://127.0.0.1:16686/api/v3/traces?query.service_name=frame-trace&query.start_time_min=2020-01-01T00:00:00Z&query.start_time_max=2100-01-01T00:00:00Z' | grep -o '"name":"block #[^"]*"'
```

```text
"name":"block #4 (0x06618995…0637)"
```

## Stop

```sh
docker compose down
```

Jaeger keeps traces in memory, so stopping it deletes them.
