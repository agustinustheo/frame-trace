# Trace a runtime API call

`frame-trace call` runs one runtime API, such as a nonce query or a fee query, against a block's state.
You get the result, every storage read it made and its proof size.
Writes the call makes are thrown away afterwards.

## 1. Run a call

This asks Westend Asset Hub for Alice's nonce at the latest final block:

```sh
frame-trace call --rpc wss://westend-asset-hub-rpc.polkadot.io --at finalized \
  --method AccountNonceApi_account_nonce \
  --data 0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d \
  --check-state-call --otlp
```

```text
call AccountNonceApi_account_nonce at #18596955: loading runtime and preparing state
call AccountNonceApi_account_nonce at #18596955: 1 storage operations; result: 520
root check: NOT APPLICABLE (runtime API call); authenticated state proofs; RPC calls: 11 (timed replay: 0); execution proof: 3713 bytes; proof request rounds: 1
state_call check: MATCH
exported 3 spans to http://127.0.0.1:4318
```

What the lines mean:

| Line | Meaning |
| --- | --- |
| `1 storage operations; result: 520` | The call made one read and returned 520, decoded with the runtime's metadata. |
| `root check: NOT APPLICABLE` | A call makes no new block, so there is no root to compare. Every read is still checked by proof. See [What a replay proves](../concepts/what-a-replay-proves.md). |
| `execution proof: 3713 bytes` | The storage proof this call needed. |
| `state_call check: MATCH` | The node's own `state_call` at the same block returned the same bytes. |

In Jaeger the trace is named `call AccountNonceApi_account_nonce at #18596955`.

## 2. Choose the state

`--at` picks the block whose state the call reads:

| Value | Picks |
| --- | --- |
| `best` | The newest block. This is the default. |
| `finalized` | The latest final block. |
| A number or a full hash | That block. |

## 3. Pass the arguments

Use exactly one of `--data` or `--args`.

- `--data` takes the SCALE-encoded arguments in hex. Use `0x` for a call with no arguments.
- `--args` takes JSON and encodes it with the runtime's metadata. It needs metadata V15 or newer.

```sh
frame-trace call --rpc wss://westend-asset-hub-rpc.polkadot.io --at finalized \
  --method TransactionPaymentApi_query_weight_to_fee \
  --args '{"weight":{"ref_time":1000000,"proof_size":0}}'
```

```text
call TransactionPaymentApi_query_weight_to_fee at #18596958: loading runtime and preparing state
call TransactionPaymentApi_query_weight_to_fee at #18596958: 0 storage operations; result: 800000
root check: NOT APPLICABLE (runtime API call); authenticated state proofs; RPC calls: 9 (timed replay: 0); execution proof: 0 bytes; proof request rounds: 0
```

`--args` accepts a JSON array of positional arguments or an object with named ones.
Values can be integers, booleans, strings, byte arrays, records and enums.
Write an enum as `{"$variant":"Some","$values":[16]}`.
Write an account as a 32-byte array.
For an SS58 address or any shape JSON cannot express, use `--data`.

## 4. See the reads

- `--summary` prints the same tables as `frame-trace summary`.
- `--json <file>` writes the trace, the input, the raw result and the decoded result.
- `--otlp` sends the trace to Jaeger or Tempo.

## Compare with the node

`--check-state-call` sends the same call to the node with `state_call` and compares the bytes.
On a mismatch frame-trace prints `state_call check: MISMATCH` and exits with code 3.
It still writes the JSON, saves and exports first.

## Replay a call offline

`--save <dir>` keeps the call and its proofs.
`--offline <dir>` runs it again with no network:

```sh
frame-trace call --offline saved-call
```

```text
call AccountNonceApi_account_nonce at #18596955: loading runtime and preparing state
call AccountNonceApi_account_nonce at #18596955: 1 storage operations; result: 520
root check: NOT APPLICABLE (runtime API call); authenticated state proofs; RPC calls: 0 (timed replay: 0); execution proof: 3713 bytes; proof request rounds: 0
state_call check: MATCH (recorded; unverified)
```

`recorded; unverified` means the comparison used the node's answer stored in the saved directory.
Nothing proves that stored answer came from a node, because anyone can edit the file.
