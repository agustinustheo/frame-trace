# Choose where to send traces

frame-trace has two outputs: OpenTelemetry traces over OTLP/HTTP, and JSON files.
`block`, `call` and `follow` send traces with `--otlp`.
`block` and `call` write JSON with `--json`.

## Pick a trace viewer

The repository has a ready Docker Compose setup for each of these, in [`examples/`](https://github.com/agustinustheo/frame-trace/tree/main/examples).

| Setup | Use it when | Download | Ready in | Guide |
| --- | --- | --- | --- | --- |
| Grafana LGTM | You want a dashboard and TraceQL search. Parity uses this image for its own local tracing. Recommended. | 535 MB | about 3 s | [Send traces to Grafana](send-traces-to-grafana.md) |
| Grafana Alloy, Tempo and Grafana | You want the shape of a cluster, or already run Alloy or a Collector. | about 690 MB | about 3 s | [Send traces through Grafana Alloy to Tempo](send-traces-through-alloy.md) |
| Jaeger | You want to look at one trace, fast. | 51 MB | under 1 s | [Jaeger](#jaeger), below |

Download sizes are compressed images. Start times were measured on a laptop once the images were downloaded, until the receiver and the web page answered.

All three listen for OTLP/HTTP on port 4318, so bare `--otlp` works with each.
Run one at a time.

## Jaeger

Run this from the `frame-trace` directory:

```sh
cd examples/jaeger
docker compose up -d
cd ../..
frame-trace block --offline fixtures/replay/people-traced --otlp
```

```text
...
exported 633 spans to http://127.0.0.1:4318
```

Open <http://127.0.0.1:16686> and pick the service `frame-trace`.
[Your first trace](../getting-started/first-trace.md) shows what you will see.
Stop it with `docker compose down` from `examples/jaeger`.

## Any other OTLP receiver

`--otlp` takes the base URL of an OTLP/HTTP receiver.
frame-trace adds `/v1/traces` unless the URL already ends with it.
Both of these send to the same place:

```sh
frame-trace block --offline fixtures/replay/people-transfer --otlp http://127.0.0.1:4318
frame-trace block --offline fixtures/replay/people-transfer --otlp http://127.0.0.1:4318/v1/traces
```

```text
...
exported 428 spans to http://127.0.0.1:4318/v1/traces
```

The receiver must accept OTLP over HTTP with protobuf, usually on port 4318.
Headers, such as an authorization header, come from `OTEL_EXPORTER_OTLP_HEADERS`.
See [Send traces through Grafana Alloy to Tempo](send-traces-through-alloy.md#send-to-an-alloy-or-collector-you-already-run).

Every trace uses the service name `frame-trace`.
Search by span name, such as `extrinsic 2: Utility.batch`, or by attribute, such as `frame.block.number`.
[Span names and attributes](../reference/spans.md) lists them all.

## What `--otlp` sends

| Command | Traces |
| --- | --- |
| `block` | One trace per block, rooted at `block #<number> (<short hash>)`. |
| `call` | One trace per call, rooted at `call <method> at #<number>`. |
| `follow` | One trace per block, as each block is traced. |

frame-trace waits for the receiver to answer and fails if it rejects any span.
A receiver may still drop data later, for example through sampling or retention.

If the receiver cannot be reached:

```text
Error: exporting to http://127.0.0.1:4999: 428 of 428 spans were not exported (first error: Operation failed: HTTP export failed: network error)
```

If it answers but rejects some spans:

```text
Error: exporting to http://127.0.0.1:4319: 5 of 428 spans rejected by the receiver: span attribute limit exceeded
```

Both exit with code 1.

See [Troubleshooting](../troubleshooting.md#n-of-m-spans-were-not-exported).

## JSON

```sh
frame-trace block --offline fixtures/replay/people-traced --json block.json
```

`--json -` writes to stdout.
With `call --summary --json -`, the tables go to stderr so stdout stays valid JSON.

Use JSON to:

- keep a block's trace without a trace viewer;
- read it again with `summary --from-json block.json`;
- add up many blocks with `report block-1.json block-2.json`;
- load it into your own scripts.

[JSON output](../reference/json.md) describes the fields.

## Both at once

`--otlp` and `--json` can be combined in one run, together with `--save`:

```sh
frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block finalized \
  --otlp --json block.json --save saved-block
```

## Before you share

Traces and JSON hold raw storage keys and decoded names, including account addresses such as `5Grw…`.
With `--runtime-override`, they also hold the local path of your Wasm file.
Storage keys are not secrets on a public chain, but review traces before you upload them to a shared or hosted backend.
