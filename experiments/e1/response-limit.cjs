// Use the proof-aware scratch node with RPC_MAX_RESPONSE_SIZE=1.
const fs = require('node:fs');
const path = require('node:path');
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { cryptoWaitReady } = require('@polkadot/util-crypto');
process.argv[2] = 'e1-probes/response-limit';
const { rpc, submit, out, write } = require('./record.cjs');
(async () => {
  await cryptoWaitReady();
  const api = await ApiPromise.create({ provider: new WsProvider(`ws://127.0.0.1:${process.env.RPC_PORT || 19944}`), noInitWarn: true });
  try {
    const alice = new Keyring({ type: 'sr25519' }).addFromUri('//Alice');
    const items = Array.from({ length: 120 }, (_, i) => [`0x3a65312d73697a653a${i.toString(16).padStart(4, '0')}`, '0x' + 'a5'.repeat(4096)]);
    const hash = await submit(api, alice, api.tx.sudo.sudo(api.tx.system.setStorage(items)), 'response-limit');
    await rpc('chain_getBlock', [hash], 'block.json');
    await rpc('chain_getHeader', [hash], 'header.json');
    const metadata = await rpc('state_getMetadata', [hash]);
    fs.writeFileSync(path.join(out, 'metadata.hex'), metadata + '\n');
    await rpc('state_getRuntimeVersion', [hash], 'runtime-version.json');
    const full = await rpc('state_traceBlock', [hash, 'state', '', ''], 'trace-full.json');
    const filtered = await rpc('state_traceBlock', [hash, 'state', items[0][0].slice(2), ''], 'trace-filtered.json');
    write(path.join(out, 'summary.json'), { hash, items: items.length, valueBytes: 4096, fullError: full.error || null, filteredEvents: filtered.blockTrace?.events.length });
    if (!full.error) throw Error('Expected full trace to exceed the 1 MiB response limit');
    if (filtered.blockTrace?.events.length !== 1) throw Error('Expected exactly one filtered write');
    console.log('Response limit confirmed:', full.error);
  } finally { await api.disconnect(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
