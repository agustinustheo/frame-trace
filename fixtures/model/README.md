# Model fixtures

Handcrafted `state_traceBlock` responses (full JSON-RPC envelopes) used by the model, decode,
otel and CLI tests. They follow the real wire shape (`sp_rpc::tracing::BlockTrace`, field formats
from `sp_state_machine::Ext`) and the FRAME write sequence of `frame-executive`/`frame-system`,
but are much shorter than a real block. Keys are real `twox128` hashes; values are made up.

- `two-extrinsics.json`: a `Call` event; initialization with a duplicate `System::Number` read
  and the `initialize_block` tail after the `ApplyExtrinsic(0)` marker
  (`MultiBlockMigrations::Cursor`, `System::BlockHash(0)`); extrinsic 0 (`Timestamp.set`, bytes
  from Westend) ending with the `ExtrinsicWeightReclaimed` cleanup; `inherents_applied`
  (`InherentsApplied = true`, an `on_poll` read); extrinsic 1 (a signed
  `Balances.transfer_keep_alive` from Westend: duplicate `System::Account` read, `Append`, a
  child-trie read, `Exists`); `note_finished_extrinsics`; finalization with `ClearPrefix` and
  `StorageRoot`.
- `no-extrinsics.json`: phase markers only, with `inherents_applied` running between the last
  extrinsic marker and finalization.
- `real/`: one block recorded from a real node, see [real/README.md](real/README.md).

Other recorded fixtures live under `fixtures/<chain>/`.
