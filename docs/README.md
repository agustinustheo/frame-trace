# frame-trace

Replay any block of a Polkadot SDK chain on your machine and see every storage read and write as a trace.

![A Utility.batch block in Jaeger with numbered callouts](images/hero.png)

The picture shows block #4 of a People chain in Jaeger:

1. the block;
2. one [phase](concepts/phases.md) of it, here extrinsic 2, a `Utility.batch`;
3. the runtime calls inside it, `batch` and then `transfer_keep_alive`;
4. one storage read, Bob's account;
5. where the read was answered: from the [cache or the trie](concepts/cache-or-trie.md);
6. how many [proof bytes](concepts/proof-bytes.md) the read added;
7. how long the read took on your machine.

## What you get

- Every storage read and write of a block, in order, with decoded names such as `System::Account(5Grw…)`.
- For each read, whether it came from the cache or the trie, and how many bytes it added to the block's proof.
- Tables of the busiest storage items, repeated reads and costs per extrinsic, for one block or thousands.
- A [root check](concepts/what-a-replay-proves.md) on every block: the replay reached the same state as the chain, so the trace is what really happened.

## How it works

frame-trace asks any node for the block and the state it needs, over normal RPC.
It checks every value against the parent block's state root.
Then it runs the block with the chain's own runtime on your machine and records each storage call.

[What a replay proves](concepts/what-a-replay-proves.md) shows the steps in a diagram.

## Where to start

- New here? [Install frame-trace](getting-started/install.md), then [get your first trace](getting-started/first-trace.md).
- Have a question about a block? Pick a guide, such as [Find repeated reads](guides/find-repeated-reads.md).
- Not sure what a number means? Read the [concepts](concepts/what-a-replay-proves.md).
- Looking up a flag? See [Commands and flags](reference/commands.md).
