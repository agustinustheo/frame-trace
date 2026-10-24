// NODE_PATH=/tmp/pm-audit/e1/client/node_modules node experiments/e2/local.cjs OUT
// Run inside E1 run-node.py; no node lifecycle or unsafe RPC exists in the Rust replay.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { cryptoWaitReady } = require('@polkadot/util-crypto');
const out = path.resolve(process.argv[2]);
const port = process.env.RPC_PORT || '19954';
const url = `http://127.0.0.1:${port}`;
const binary = process.env.E2_BINARY || '/tmp/pm-audit/e2-target/release/frame-trace-e2';
fs.mkdirSync(out, { recursive: true });
const write = (p, v) => fs.writeFileSync(p, JSON.stringify(v, null, 2) + '\n');
async function rpc(method, params = []) {
  const response = await fetch(url, {method: 'POST', headers: {'content-type': 'application/json'},
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params })});
  const data = await response.json();
  if (data.error) throw new Error(`${method}: ${JSON.stringify(data.error)}`);
  return data.result;
}
async function submit(api, signer, tx, label) {
  let next = (await api.rpc.chain.getHeader()).number.toNumber() + 1;
  await tx.signAndSend(signer, { nonce: -1 });
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    const head = (await api.rpc.chain.getHeader()).number.toNumber();
    while (next <= head) {
      const hash = await api.rpc.chain.getBlockHash(next++);
      const block = await api.rpc.chain.getBlock(hash);
      const index = block.block.extrinsics.findIndex(ext => ext.toHex() === tx.toHex());
      if (index < 0) continue;
      const at = await api.at(hash);
      const events = (await at.query.system.events()).filter(e => e.phase.isApplyExtrinsic && e.phase.asApplyExtrinsic.toNumber() === index);
      if (events.some(({event}) => event.section === 'system' && event.method === 'ExtrinsicFailed') ||
          events.some(({event}) => event.section === 'sudo' && event.method === 'Sudid' && event.data[0].isErr)) throw Error(`${label}: dispatch failed`);
      if (!events.some(({event}) => event.section === 'system' && event.method === 'ExtrinsicSuccess')) throw Error(`${label}: no success event`);
      if (label === 'batch' && (!events.some(({event}) => event.section === 'utility' && event.method === 'BatchCompleted') ||
          events.filter(({event}) => event.section === 'balances' && event.method === 'Transfer').length !== 3)) throw Error('batch incomplete');
      return { hash: hash.toHex(), events: events.map(e => e.toHuman()), index };
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw Error(`Timeout submitting ${label}`);
}
async function main() {
  await cryptoWaitReady();
  const api = await ApiPromise.create({provider: new WsProvider(`ws://127.0.0.1:${port}`), noInitWarn: true});
  try {
    const ring = new Keyring({type: 'sr25519'});
    const alice = ring.addFromUri('//Alice'), bob = ring.addFromUri('//Bob'), charlie = ring.addFromUri('//Charlie');
    const unit = 10n ** BigInt(api.registry.chainDecimals[0]);
    const prefix = '0x3a6672616d652d74726163652d65323a', key = prefix + '01';
    const deadline = Date.now() + 60000;
    while ((await api.rpc.chain.getHeader()).number.toNumber() < 1) {
      if (Date.now() > deadline) throw Error('No block 1 after 60 seconds');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    const blocks = [{label: 'inherents', hash: (await api.rpc.chain.getBlockHash(1)).toHex()}];
    const cases = [
      ['transfer', api.tx.balances.transferKeepAlive(bob.address, unit.toString())],
      ['remark', api.tx.system.remark('0x6672616d652d74726163652d4532')],
      ['batch', api.tx.utility.batch([api.tx.balances.transferKeepAlive(bob.address, (2n * unit).toString()),
        api.tx.balances.transferKeepAlive(charlie.address, (3n * unit).toString()), api.tx.balances.transferKeepAlive(bob.address, (4n * unit).toString())])],
      ['individuality-grant', api.tx.sudo.sudo(api.tx.peopleLite.increaseAttestationAllowance(bob.address, 7))],
      ['individuality-clear', api.tx.sudo.sudo(api.tx.peopleLite.clearAttestationAllowance(bob.address))],
      ['large-put', api.tx.sudo.sudo(api.tx.system.setStorage([[key, '0x' + 'a5'.repeat(4096)]]))],
      ['clear-prefix', api.tx.sudo.sudo(api.tx.system.killPrefix(prefix, 1))],
    ];
    for (const [label, tx] of cases) {
      console.log(`Submitting ${label}`);
      blocks.push({label, ...await submit(api, alice, tx, label)});
    }
    write(path.join(out, 'manifest.json'), {captured_at: new Date().toISOString(), blocks});
    for (const block of blocks) {
      const dir = path.join(out, block.label); fs.mkdirSync(dir, {recursive: true});
      write(path.join(dir, 'oracle.json'), await rpc('state_traceBlock', [block.hash, 'state', '', '']));
      const at = await api.at(block.hash);
      write(path.join(dir, 'extrinsics.json'), (await api.rpc.chain.getBlock(block.hash)).block.extrinsics.map((e, index) => {
        const ext = at.registry.createType('Extrinsic', e.toHex());
        return {index, section: ext.method.section, method: ext.method.method};
      }));
      write(path.join(dir, 'chain-events.json'), (await at.query.system.events()).toHuman());
      console.log(`Replaying ${block.label} ${block.hash}`);
      try {
        const result = execFileSync(binary, [url, block.hash, dir, '5'], {encoding: 'utf8', timeout: 240000, stdio: ['ignore', 'pipe', 'pipe']});
        fs.writeFileSync(path.join(dir, 'replay.log'), result);
        console.log(result.trim());
      } catch (error) {
        fs.writeFileSync(path.join(dir, 'replay-error.log'), String(error.stderr || error));
        console.error(`${block.label}: ${error.stderr || error}`);
      }
    }
    write(path.join(out, 'probe-after-kill.json'), await rpc('state_getStorage', [key, blocks.at(-1).hash]));
  } finally { await api.disconnect(); }
}
main().catch(error => {console.error(error); process.exitCode = 1;});
