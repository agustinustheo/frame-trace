# Send traces through Grafana Alloy to Tempo

Send frame-trace traces through an agent to Grafana Tempo, the way a cluster collects traces.
Parity's clusters run Grafana Alloy as the agent and Tempo as the trace store.
This guide runs that setup on your machine, then shows how to point frame-trace at an Alloy or OpenTelemetry Collector you already have.

```text
frame-trace --OTLP/HTTP:4318--> Alloy --OTLP/gRPC:4317--> Tempo <--TraceQL-- Grafana
```

## 1. Start Alloy, Tempo and Grafana

Run this from the `frame-trace` directory:

```sh
cd examples/alloy-tempo
docker compose up -d
```

The first start downloads about 690 MB of images.
After that, all three are ready in about 3 seconds.

| Service | Image | Port on your machine |
| --- | --- | --- |
| Alloy | `grafana/alloy:v1.21.2` | 4318 (OTLP/HTTP), 12345 (Alloy web page) |
| Tempo | `grafana/tempo:3.1.1` | 3200 (Tempo HTTP API) |
| Grafana | `grafana/grafana:13.2.4` | 3000 |

> **Warning:** Grafana opens without a login, as a viewer that can use Explore. Use this on your own machine only.

`config.alloy` is the whole Alloy pipeline: receive OTLP, batch, send to Tempo.

```text
otelcol.receiver.otlp "default" {
  http {
    endpoint = "0.0.0.0:4318"
  }
  grpc {
    endpoint = "0.0.0.0:4317"
  }

  output {
    traces = [otelcol.processor.batch.default.input]
  }
}

otelcol.processor.batch "default" {
  output {
    traces = [otelcol.exporter.otlp.tempo.input]
  }
}

otelcol.exporter.otlp "tempo" {
  client {
    endpoint = "tempo:4317"
    tls {
      insecure = true
    }
  }
}
```

## 2. Send a trace

Go back to the `frame-trace` directory:

```sh
cd ../..
frame-trace block --offline fixtures/replay/people-traced --otlp
```

```text
...
exported 633 spans to http://127.0.0.1:4318
```

`exported` means Alloy accepted all 633 spans.
frame-trace cannot see whether Alloy then delivered them to Tempo.

## 3. Check that Tempo has it

Ask Tempo directly:

```sh
curl -s 'http://127.0.0.1:3200/api/search?tags=service.name%3Dframe-trace' | grep -o '"rootTraceName":"[^"]*"'
```

```text
"rootTraceName":"block #4 (0x06618995…0637)"
```

The trace can be searched about a second after frame-trace exits.
`tempo.yaml` shortens two Tempo waits for this; with Tempo's defaults it takes about 30 seconds.

If nothing comes back, open <http://127.0.0.1:12345>.
Alloy's page shows each part of the pipeline and whether it is healthy.

## 4. Open it in Grafana

Open <http://127.0.0.1:3000>.
The same `frame-trace` dashboard as in [Send traces to Grafana](send-traces-to-grafana.md#3-open-the-dashboard) opens.
**Explore** → **Tempo** runs TraceQL queries; [Find storage problems with TraceQL](query-traces-in-grafana.md) has ready ones.

## 5. Stop

From `examples/alloy-tempo`:

```sh
docker compose down -v
```

`-v` also deletes the volume that holds the traces.

## Send to an Alloy or Collector you already run

`--otlp` takes the base URL of any OTLP/HTTP receiver.
frame-trace adds `/v1/traces` unless the URL already ends with it.

```sh
frame-trace block --rpc <url> --block finalized --otlp http://<alloy-host>:4318
```

The receiver needs OTLP over HTTP with protobuf.
In Alloy, that is the `http` block of `otelcol.receiver.otlp`, as above.
In an OpenTelemetry Collector, it is the `http` protocol of the `otlp` receiver.
frame-trace does not send OTLP over gRPC.

`https://` URLs work.

### Headers and authentication

frame-trace has no flag for headers.
It reads them from the standard OpenTelemetry environment variable `OTEL_EXPORTER_OTLP_HEADERS`, as `key=value` pairs separated by commas.
Values are URL-decoded, so write a space as `%20`:

```sh
OTEL_EXPORTER_OTLP_HEADERS="authorization=Basic%20<base64 of user:password>,x-scope-orgid=<tenant>" \
  frame-trace block --rpc <url> --block finalized --otlp https://<alloy-host>:4318
```

frame-trace sent both headers in a test against a local receiver.
`OTEL_EXPORTER_OTLP_TRACES_HEADERS` works the same way and replaces `OTEL_EXPORTER_OTLP_HEADERS` when both are set.

A user name and password inside the URL, such as `https://user:pass@host`, are not sent.
Client certificates are not supported.

[Commands and flags](../reference/commands.md#environment-variables) lists these variables.

## Before you share

Traces hold raw storage keys and decoded names, including account addresses.
Review them before you send them to a shared backend.
See [Send traces](send-traces.md#before-you-share).
