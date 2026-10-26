# Trace a block from any node

frame-trace needs only the normal RPC methods every Polkadot SDK node serves.
You do not need your own node, unsafe RPC or a special build.

## Pick the block

`--block` takes one of four values:

| Value | Example | Picks |
| --- | --- | --- |
| `finalized` | `--block finalized` | The newest block the chain has finalized. A good default. |
| `best` | `--block best` | The newest block the node knows. A reorg can still replace it. |
| A number | `--block 1000000` | The block at that height on the node's chain. |
| A hash | `--block 0x8494…c58b` | That exact block. Use the full 32-byte hash. |

Run it in a terminal.
This example traces block 1,000,000 of Westend Asset Hub:

```sh
frame-trace block --rpc wss://westend-asset-hub-rpc.polkadot.io --block 1000000
```

```text
replay #1000000: fetching state (up to 8 concurrent proof requests)
replay #1000000: loading runtime and preparing state
root check: PASS; RPC calls: 17 (timed replay: 0); execution proof: 9840 bytes
fetching state #1000000: 13 proof request rounds; 17 RPC calls
state root: computed 0x614e27580d5a379f5a7a57dd7598df672545859d85d387ae5e5ab7ebfcd9b608; header 0x614e27580d5a379f5a7a57dd7598df672545859d85d387ae5e5ab7ebfcd9b608
block 0x849423aff4eae359240b694f1dc46d108522393ffd1cb4cb02401cfb95b5c58b: 4 phases, 143 storage operations (68 reads, 73 writes, 8 duplicate reads). Use --otlp or --json to export it.
```

Without an output flag, frame-trace prints this one-line count.
Add an output to see the trace:

| Flag | Output |
| --- | --- |
| `--otlp` | Send the trace to Grafana, Jaeger or any OTLP receiver. See [Choose where to send traces](send-traces.md). |
| `--json <file>` | Write the whole trace as JSON. `-` writes to stdout. See [JSON output](../reference/json.md). |
| `--save <dir>` | Keep everything needed to replay the block later with no network. See [Save a block and replay it offline](save-and-replay.md). |

To see tables in the terminal instead, use `summary` with the same source flags:

```sh
frame-trace summary --rpc wss://westend-asset-hub-rpc.polkadot.io --block finalized
```

[Find the heaviest storage items](find-heaviest-items.md) explains its tables.

## Choose an endpoint

The URL can be `ws://`, `wss://`, `http://` or `https://`.
The node must serve the standard methods, including `state_getReadProof`, which frame-trace uses for every piece of state.

One block takes about 90 requests on Westend Asset Hub, and `report` and `follow` send that for every block.
Before you trace ranges or follow a chain, check the endpoint's:

- request limits: public endpoints can rate-limit you, and frame-trace then retries and slows down;
- pricing, for paid providers that charge per request;
- historical state, if you need old blocks.

For an old block, the node must still have that block's parent state.

> **Warning:** Most nodes keep only recent state. For older blocks, use an archive node. Otherwise the replay fails with `parent state may be pruned (use an archive endpoint)`.

The public Parity endpoints, such as `wss://westend-asset-hub-rpc.polkadot.io`, served block 1,000,000 in the example above.

## Make repeated runs faster

- Add `--cache-dir <dir>` to keep fetched state and the compiled runtime between runs.
- Raise `--proof-concurrency` on a fast connection to send more requests at once.

Defaults and limits are in [Commands and flags](../reference/commands.md).

## Names and metadata

frame-trace decodes keys such as `System::Account(5Grw…)` using the runtime's own metadata.
It gets the metadata by running the runtime's metadata API on your machine, checked like any other state.
Runtimes need metadata V14 or newer.
If that fails, frame-trace prints a notice and shows raw keys and hex values.
`--unauthenticated-metadata` lets it fall back to the node's `state_getMetadata` instead.
That metadata is not checked, and the output says so.
