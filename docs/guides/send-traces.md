# Send traces to Tempo, Jaeger or JSON

frame-trace has two outputs: OpenTelemetry traces over OTLP/HTTP, and JSON files.
`block`, `call` and `follow` send traces with `--otlp`.
`block` and `call` write JSON with `--json`.

## Jaeger

```sh
docker run -d --rm --name jaeger -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:latest
frame-trace block --offline fixtures/replay/people-traced --otlp
```

```text
...
exported 633 spans to http://127.0.0.1:4318
```

Open <http://127.0.0.1:16686> and pick the service `frame-trace`.
[Your first trace](../getting-started/first-trace.md) shows what you will see.

## Grafana Tempo or any OTLP collector

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

Point `--otlp` at your collector instead, for example `--otlp http://<tempo-host>:4318`.

For Tempo, enable its OTLP receiver with the HTTP protocol.
Its usual port is 4318.
An OpenTelemetry Collector in front of any other backend works the same way.

Every trace uses the service name `frame-trace`.
Search by span name, such as `extrinsic 2: Utility.batch`, or by attribute, such as `frame.block.number`.
[Span names and attributes](../reference/spans.md) lists them all.

## What `--otlp` sends

| Command | Traces |
| --- | --- |
| `block` | One trace per block, rooted at `block #<number> (<short hash>)`. |
| `call` | One trace per call, rooted at `call <method> at #<number>`. |
| `follow` | One trace per block, as each block is traced. |

frame-trace waits until the receiver confirms every span.
If some are lost, it fails:

```text
Error: exporting to http://127.0.0.1:4999: 428 of 428 spans were not exported (first error: Operation failed: HTTP export failed: network error)
```

See [Troubleshooting](../troubleshooting.md#n-of-n-spans-were-not-exported).

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
