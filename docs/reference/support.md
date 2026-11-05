# What frame-trace supports

This page lists the chains, nodes and runtimes frame-trace can replay.

## Chains

| Requirement | Supported |
| --- | --- |
| Chain | A Polkadot SDK chain with Blake2-256 block headers and `u32` block numbers. |
| Runtime | Any runtime that uses only the host functions listed in [Host functions](#host-functions). |
| Decoded names | Need [metadata](glossary.md#metadata) V14, V15 or V16. With older metadata the replay still runs, and keys and results show as hex. |
| `call --args` | Needs metadata V15 or V16. Use `--data` with older metadata. |

## Nodes

| Requirement | Supported |
| --- | --- |
| URL | `ws://`, `wss://`, `http://` or `https://`. |
| RPC methods | Only the safe methods below. No unsafe RPC and no special node build. |
| State | The node must still have the parent state of the block. For old blocks, use an [archive node](glossary.md#archive-node). |

frame-trace sends only these RPC methods:

| Method | Used for |
| --- | --- |
| `chain_getBlock` | The block to replay. |
| `chain_getHeader` | Block headers, including the parent's state root. |
| `chain_getBlockHash` | Turning a block number into a hash. |
| `chain_getFinalizedHead` | `--block finalized`, `call --at finalized`, `report` and `follow`. |
| `state_getRuntimeVersion` | The runtime version at a block. |
| `state_getReadProof` | Every piece of state the replay reads, checked against the state root. |
| `state_getChildReadProof` | State in child tries. |
| `state_getMetadata` | Only with `--unauthenticated-metadata`, when the runtime's own metadata cannot be loaded. |
| `state_call` | Only with `call --check-state-call`. |

## Host functions

frame-trace runs the runtime with the host functions of Polkadot SDK release `polkadot-stable2609`:

| Group | What it covers |
| --- | --- |
| Standard host functions | Storage, child storage, hashing, crypto, trie, logging, allocator, offchain, transaction index and the other functions every Polkadot SDK runtime imports. |
| Storage proof size | The proof size a parachain runtime reads while it runs. |
| Statement store | The statement store runtime interface. |
| Elliptic curves | BLS12-381, Ed-on-BLS12-381-Bandersnatch, Pallas and Vesta. |

A runtime that imports any other host function cannot be replayed.
The error says `runtime requires function imports which are not present on the host`.
See [Troubleshooting](../troubleshooting.md#runtime-requires-function-imports-which-are-not-present-on-the-host).

## Tested chains

| Chain | How |
| --- | --- |
| [Westend Asset Hub](glossary.md#westend-asset-hub) | Public endpoint `wss://westend-asset-hub-rpc.polkadot.io`, with `block`, `report` and `follow`. [Analyse many blocks](../guides/analyse-many-blocks.md) shows a `report` run. |
| Paseo People Next | Public endpoint `wss://paseo-people-next-system-rpc.polkadot.io`, 20 blocks with `block` and `summary`. Each passed the root check. |
| A local [People chain](glossary.md#people-chain) | The saved blocks in `fixtures/replay/`, replayed offline. |

Other chains that meet the requirements above should work.
If one does not, open an issue with the chain, the block number and the error.

## Versioning

frame-trace is at version 0.x.
Any release can change commands, flags or the JSON output.
Each such change is listed in [`CHANGELOG.md`](https://github.com/agustinustheo/frame-trace/blob/main/CHANGELOG.md).
The JSON output also carries a `schema_version`, see [JSON output](json.md).
