# Glossary

Plain-word meanings of the terms these docs use.

## Archive node

A node that keeps the state of every past block. Ordinary nodes keep only recent state, so tracing an old block needs an archive node.

## Cache

In frame-trace, a read answered from the block's own earlier writes, held in memory. It never touches the trie. See [Cache or trie](../concepts/cache-or-trie.md).

## Extrinsic

One item in a block's body: a transaction or an inherent. frame-trace names each one `extrinsic N: Pallet.call`.

## Inherent

An unsigned extrinsic the block producer must add, such as `Timestamp.set` or a parachain's `ParachainSystem.set_validation_data`.

## Metadata

A description the runtime gives of its own pallets, storage items, calls and types. frame-trace uses it to turn raw keys into names such as `System::Account`.

## OTLP

The OpenTelemetry Protocol, the standard format for sending traces. Jaeger and Grafana Tempo accept it over HTTP, usually on port 4318.

## Pallet

A module of a Polkadot SDK runtime, such as `System` or `Balances`. Each pallet owns its storage items.

## People chain

A Polkadot SDK parachain for identity. The example blocks shipped in `fixtures/replay/` come from a local People chain.

## Phase

One part of a block's execution: initialization, an extrinsic, post inherents or finalization. See [Phases of a block](../concepts/phases.md).

## PoV

Proof of validity: what a parachain collator sends to relay chain validators. It holds the block and a compacted storage proof. frame-trace's proof bytes estimate the storage proof part only. See [Proof bytes](../concepts/proof-bytes.md).

## Proof bytes

How many bytes a storage operation added to the block's storage proof. See [Proof bytes](../concepts/proof-bytes.md).

## Root check

frame-trace's check that the replay produced the same state root as the block header. A pass means the trace shows what the block really did. See [What a replay proves](../concepts/what-a-replay-proves.md).

## Runtime

The chain's logic, compiled to Wasm and stored on chain under the key `:code`. frame-trace runs it on your machine to replay a block.

## SCALE

The binary encoding Polkadot SDK uses for storage values, extrinsics and runtime API arguments. `call --data` takes SCALE bytes in hex.

## Signed and unsigned extrinsics

A signed extrinsic carries a user's signature, such as a transfer. An unsigned one has none; most are inherents.

## Span and trace

A span is one timed operation with attributes, such as one storage read. A trace is the tree of spans for one block or call.

## State root

A 32-byte hash that commits to the whole chain state after a block. Each block header contains one.

## Storage item and key

A storage item is a named value or map in a pallet, such as `System::Account`. A key is the raw bytes that address one entry, such as Alice's account.

## Trie

The tree structure that holds all chain state. Its root hash is the state root. A trie read walks from the root down to the key. See [Cache or trie](../concepts/cache-or-trie.md).

## Wasm

WebAssembly, the portable format the runtime is compiled to. `--runtime-override` takes a Wasm file.

## Westend Asset Hub

A public test parachain of the Westend test network. The live examples in these docs trace it.
