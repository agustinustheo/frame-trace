# Metadata fixtures

Real runtime metadata used by the decode, otel and CLI tests.

| File | Chain | Runtime | Metadata | Fetched |
| --- | --- | --- | --- | --- |
| `westend-v14.scale` | Westend relay chain | `westend` spec version 1025002 (transaction version 27) | V14 | 2026-10-24 19:08 UTC |
| `westend-v16.scale` | Westend relay chain | `westend` spec version 1025002 | V16 | 2026-10-24 19:08 UTC |
| `westend-block-33443133-extrinsics.json` | Westend relay chain | spec version 1025002 | - | 2026-10-24 19:08 UTC |

All from `https://westend-rpc.polkadot.io`. The `.scale` files are raw SCALE bytes
(`RuntimeMetadataPrefixed`, starting with the `meta` magic):

```sh
# V14
curl -s -H 'Content-Type: application/json' \
  -d '{"id":1,"jsonrpc":"2.0","method":"state_getMetadata","params":[]}' \
  https://westend-rpc.polkadot.io
# V16: state_call Metadata_metadata_at_version(16); the result is Option<OpaqueMetadata>,
# so the leading 0x01 and the compact length were stripped before saving.
curl -s -H 'Content-Type: application/json' \
  -d '{"id":1,"jsonrpc":"2.0","method":"state_call","params":["Metadata_metadata_at_version","0x10000000"]}' \
  https://westend-rpc.polkadot.io
```

The extrinsics file holds the body of block 33443133 (`chain_getBlock`): `Timestamp.set`,
`ParaInherent.enter` and a signed `Balances.transfer_keep_alive`.
