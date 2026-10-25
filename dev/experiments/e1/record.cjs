// Run with NODE_PATH=/tmp/pm-audit/e1/client/node_modules node record.cjs [chain-name]
// All trace JSON files contain the RPC result, without the JSON-RPC envelope.
const fs = require('node:fs');
const path = require('node:path');
const { performance } = require('node:perf_hooks');
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { cryptoWaitReady, xxhashAsHex } = require('@polkadot/util-crypto');
const root = process.env.FIXTURES || path.resolve(__dirname, '../../fixtures');
const name = process.argv[2] || 'next-people-paseo';
const out = path.join(root, name);
const port = process.env.RPC_PORT || '19944';
const url = `http://127.0.0.1:${port}`;
fs.mkdirSync(out, { recursive: true });
const write = (p, v) => fs.writeFileSync(p, JSON.stringify(v, null, 2) + '\n');
const measurements = [];
let id = 0;
async function rpc(method, params = [], file) {
  const request = { jsonrpc: '2.0', id: ++id, method, params };
  const start = performance.now();
  const res = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(request) });
  const raw = await res.text();
  const elapsedMs = performance.now() - start;
  const response = JSON.parse(raw);
  measurements.push({ method, params, file, elapsedMs, responseBytes: Buffer.byteLength(raw), httpStatus: res.status, error: response.error || null });
  write(path.join(out, 'measurements.json'), measurements);
  if (file) write(path.join(out, file), response.error ? response : response.result);
  if (response.error && method !== 'state_traceBlock') throw Error(JSON.stringify(response.error));
  return response.error ? response : response.result;
}
function canonical(events) {
  return events.map(e => ({ ...e, data: { stringValues: Object.fromEntries(Object.entries(e.data.stringValues).filter(([k]) => k !== 'ext_id').sort()) } }));
}
async function record(hash, label, api) {
  const header = await rpc('chain_getHeader', [hash]);
  const number = parseInt(header.number, 16);
  const dir = String(number);
  fs.mkdirSync(path.join(out, dir), { recursive: true });
  write(path.join(out, dir, 'header.json'), header);
  const block = await rpc('chain_getBlock', [hash], `${dir}/block.json`);
  const metadata = await rpc('state_getMetadata', [hash]);
  fs.writeFileSync(path.join(out, dir, 'metadata.hex'), metadata + '\n');
  const at = await api.at(hash);
  const decoded = block.block.extrinsics.map((hex, index) => {
    const ext = at.registry.createType('Extrinsic', hex);
    return { index, section: ext.method.section, method: ext.method.method, args: ext.method.toHuman().args, hex };
  });
  write(path.join(out, dir, 'extrinsics.json'), decoded);
  write(path.join(out, dir, 'events.json'), (await at.query.system.events()).toHuman());
  const storageKeys = {
    extrinsicIndex: '3a65787472696e7369635f696e646578',
    executionPhase: xxhashAsHex('System', 128).slice(2) + xxhashAsHex('ExecutionPhase', 128).slice(2),
    systemAccount: xxhashAsHex('System', 128).slice(2) + xxhashAsHex('Account', 128).slice(2),
    extrinsicData: xxhashAsHex('System', 128).slice(2) + xxhashAsHex('ExtrinsicData', 128).slice(2),
  };
  const variants = [
    ['state', 'state', '', ''],
    ['runtime', 'state,runtime,frame,pallet,indiv', '', ''],
    ['account', 'state', storageKeys.systemAccount, ''],
    ['account-0x', 'state', '0x' + storageKeys.systemAccount, ''],
    ['prefix-clear', 'state', storageKeys.extrinsicData, ''],
    ['methods', 'state', '', 'Get,Put'],
    ['method-prefix', 'state', '', 'Ge'],
    ['clear-prefix-method', 'state', '', 'ClearPrefix'],
    ['all-targets', '', '', ''],
    ['omitted', null, null, null],
  ];
  let first;
  const counts = {};
  for (const [variant, targets, keys, methods] of variants) {
    const params = variant === 'omitted' ? [hash] : [hash, targets, keys, methods];
    const result = await rpc('state_traceBlock', params, `${dir}/trace-${variant}.json`);
    if (variant === 'state') first = result;
    counts[variant] = { events: result.blockTrace?.events.length, spans: result.blockTrace?.spans.length, errorCode: result.error?.code };
  }
  const repeat = await rpc('state_traceBlock', [hash, 'state', '', ''], `${dir}/trace-repeat.json`);
  if (first.error && label === 'transfer' && process.env.DIAGNOSE) {
    try {
      await rpc('state_call', ['Core_execute_block', at.registry.createType('Block', block.block).toHex(), header.parentHash], `${dir}/execute-error.json`);
    } catch (error) {
      console.log(`Diagnostic Core_execute_block: ${error.message.slice(0, 180)}`);
    }
  }
  const events = first.blockTrace?.events || [];
  const markers = events.map((e, eventIndex) => ({ eventIndex, ...e.data.stringValues })).filter(v => [storageKeys.extrinsicIndex, storageKeys.executionPhase].includes(v.key));
  const methods = {};
  for (const [eventIndex, e] of events.entries()) {
    const v = e.data.stringValues;
    const m = v.method || '<no method>';
    const entry = methods[m] ||= { count: 0, keys: Object.keys(v).sort(), firstEventIndex: eventIndex, example: e };
    entry.count++;
  }
  const summary = { label, hash, number, storageKeys, extrinsics: decoded.map(({ hex, ...v }) => v), counts, markers, methods,
    replayEventsEqualIgnoringExtId: first.blockTrace ? JSON.stringify(canonical(events)) === JSON.stringify(canonical(repeat.blockTrace?.events || [])) : null,
    parentIds: [...new Set(events.map(e => e.parentId))],
    extIds: [...new Set(events.map(e => e.data.stringValues.ext_id))],
  };
  write(path.join(out, dir, 'summary.json'), summary);
  console.log(JSON.stringify({ label, hash, number, counts }));
  return { label, hash, number };
}
async function submit(api, signer, tx, label) {
  console.log(`Submitting ${label}: ${tx.method.toHuman().section}.${tx.method.toHuman().method}`);
  let next = (await api.rpc.chain.getHeader()).number.toNumber() + 1;
  await tx.signAndSend(signer, { nonce: -1 });
  // The development node finalizes immediately; inspect canonical blocks rather
  // than relying on legacy transaction subscription notifications.
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    const head = (await api.rpc.chain.getHeader()).number.toNumber();
    while (next <= head) {
      const hash = await api.rpc.chain.getBlockHash(next++);
      const block = await api.rpc.chain.getBlock(hash);
      const index = block.block.extrinsics.findIndex(ext => ext.toHex() === tx.toHex());
      if (index === -1) continue;
      const at = await api.at(hash);
      const events = (await at.query.system.events()).filter(e => e.phase.isApplyExtrinsic && e.phase.asApplyExtrinsic.toNumber() === index);
      const errors = events.filter(({ event }) => event.section === 'system' && event.method === 'ExtrinsicFailed');
      if (errors.length) throw Error(`${label}: ${JSON.stringify(errors.map(e => e.toHuman()))}`);
      const sudo = events.filter(({ event }) => event.section === 'sudo' && event.method === 'Sudid');
      if (sudo.some(({ event }) => event.data[0].isErr)) throw Error(`${label}: sudo inner failure`);
      if (!events.some(({ event }) => event.section === 'system' && event.method === 'ExtrinsicSuccess')) throw Error(`${label}: missing success event`);
      if (label === 'batch' && (!events.some(({ event }) => event.section === 'utility' && event.method === 'BatchCompleted') ||
          events.filter(({ event }) => event.section === 'balances' && event.method === 'Transfer').length !== 3)) {
        throw Error('Batch did not complete all three transfers');
      }
      return hash.toHex();
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw Error(`Timeout: ${label}`);
}
async function main() {
  await cryptoWaitReady();
  const api = await ApiPromise.create({ provider: new WsProvider(`ws://127.0.0.1:${port}`), noInitWarn: true });
  try {
    write(path.join(out, 'runtime-version.json'), await rpc('state_getRuntimeVersion'));
    write(path.join(out, 'node-version.json'), { name: await rpc('system_name'), version: await rpc('system_version'), chain: await rpc('system_chain'), genesisHash: await rpc('chain_getBlockHash', [0]) });
    write(path.join(out, 'rpc-methods.json'), await rpc('rpc_methods'));
    const keyring = new Keyring({ type: 'sr25519' });
    const alice = keyring.addFromUri('//Alice');
    const bob = keyring.addFromUri('//Bob');
    const charlie = keyring.addFromUri('//Charlie');
    const blocks = [];
    let head;
    for (let i = 0; i < 60; i++) {
      head = await rpc('chain_getBlockHash', [1]);
      if (head) break;
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
    if (!head) throw Error('No block 1 after 60 seconds');
    blocks.push(await record(head, 'inherents', api));
    const unit = 10n ** BigInt(api.registry.chainDecimals[0]);
    const probePrefix = '0x3a6672616d652d74726163652d65313a';
    const probeKey = probePrefix + '01';
    const cases = [
      ['transfer', api.tx.balances.transferKeepAlive(bob.address, unit.toString())],
      ['remark', api.tx.system.remark('0x6672616d652d74726163652d4531')],
      ['batch', api.tx.utility.batch([
        api.tx.balances.transferKeepAlive(bob.address, (2n * unit).toString()),
        api.tx.balances.transferKeepAlive(charlie.address, (3n * unit).toString()),
        api.tx.balances.transferKeepAlive(bob.address, (4n * unit).toString()),
      ])],
      ['individuality-grant', api.tx.sudo.sudo(api.tx.peopleLite.increaseAttestationAllowance(bob.address, 7))],
      ['individuality-clear', api.tx.sudo.sudo(api.tx.peopleLite.clearAttestationAllowance(bob.address))],
      ['large-put', api.tx.sudo.sudo(api.tx.system.setStorage([[probeKey, '0x' + 'a5'.repeat(4096)]]))],
      ['clear-prefix', api.tx.sudo.sudo(api.tx.system.killPrefix(probePrefix, 1))],
    ];
    for (const [label, tx] of cases) {
      const hash = await submit(api, alice, tx, label);
      blocks.push(await record(hash, label, api));
      if (label === 'large-put' || label === 'clear-prefix') {
        await rpc('state_getStorage', [probeKey, hash], `${blocks.at(-1).number}/probe-storage.json`);
      }
    }
    write(path.join(out, 'manifest.json'), { chain: name, capturedAt: new Date().toISOString(), blocks });
  } finally { await api.disconnect(); }
}
module.exports = { rpc, submit, out, write };
if (require.main === module) main().catch(error => { console.error(error); process.exitCode = 1; });
