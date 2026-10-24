# Model fixtures

Handcrafted `state_traceBlock` responses (full JSON-RPC envelopes) used by the model, otel and
CLI tests. They follow the real wire shape (`sp_rpc::tracing::BlockTrace`, field formats from
`sp_state_machine::Ext`) and the real FRAME phase sequence, but are much shorter than a real
block. Keys are real `twox128` hashes; values are made up.

- `two-extrinsics.json`: `Call` event, initialization (with a duplicate read of
  `System::Number`), extrinsic 0 (`Timestamp.set`, bytes taken from Westend), extrinsic 1
  (duplicate `System::Account` read, `Append`, a child-trie read, `Exists`), the gap before
  finalization, then finalization with `ClearPrefix` and `StorageRoot`.
- `no-extrinsics.json`: phase markers only, no extrinsic between `ApplyExtrinsic(0)` and
  `Finalization`.

Real recorded fixtures live under `fixtures/<chain>/`.
