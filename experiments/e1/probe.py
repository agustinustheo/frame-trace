#!/usr/bin/env python3
"""Probe safe RPC or pruning on the scratch node started by run-node.py."""
import json
import os
from pathlib import Path
import sys
import time
import urllib.request

mode, directory = sys.argv[1:]
out = Path(directory)
out.mkdir(parents=True, exist_ok=True)
port = os.environ.get('RPC_PORT', '19944')
measurements = []

def rpc(method, params=None, file=None):
    request = {'jsonrpc': '2.0', 'id': len(measurements) + 1, 'method': method, 'params': params or []}
    start = time.perf_counter()
    req = urllib.request.Request(f'http://127.0.0.1:{port}', json.dumps(request).encode(), {'Content-Type': 'application/json'})
    with urllib.request.urlopen(req, timeout=60) as response:
        raw = response.read()
    result = json.loads(raw)
    measurements.append({'request': request, 'elapsedMs': (time.perf_counter() - start) * 1000, 'responseBytes': len(raw), 'file': file})
    if file:
        (out / file).write_text(json.dumps(result, indent=2) + '\n')
    (out / 'measurements.json').write_text(json.dumps(measurements, indent=2) + '\n')
    return result

for _ in range(120):
    block = rpc('chain_getBlockHash', [1]).get('result')
    if block:
        break
    time.sleep(0.5)
else:
    raise RuntimeError('No block 1')
rpc('state_traceBlock', [block, 'state', '', ''], 'trace-before.json')
rpc('chain_getBlock', [block], 'block.json')
if mode == 'safe':
    rpc('rpc_methods', file='rpc-methods.json')
    rpc('state_traceBlock', [block, 'state', '', ''], 'trace-safe.json')
    # Control: safe RPC really disables this unsafe method.
    rpc('author_insertKey', ['aura', '//Alice', '0x' + '00' * 32], 'unsafe-control.json')
elif mode == 'pruning':
    deadline = time.monotonic() + 180
    last_report = 0
    while time.monotonic() < deadline:
        header = rpc('chain_getHeader')['result']
        finalized = rpc('chain_getFinalizedHead')['result']
        finalized_header = rpc('chain_getHeader', [finalized])['result']
        if time.monotonic() - last_report > 10:
            print(f"head={int(header['number'], 16)} finalized={int(finalized_header['number'], 16)}", flush=True)
            last_report = time.monotonic()
        if int(finalized_header['number'], 16) >= 300:
            break
        time.sleep(0.3)
    rpc('chain_getHeader', file='head.json')
    rpc('chain_getHeader', [finalized], 'finalized-header.json')
    rpc('state_traceBlock', [block, 'state', '', ''], 'trace-after.json')
    rpc('state_getStorage', ['0x3a65787472696e7369635f696e646578', block], 'storage-old.json')
    recent = rpc('chain_getBlockHash').get('result')
    rpc('state_traceBlock', [recent, 'state', '', ''], 'trace-recent.json')
else:
    raise ValueError(mode)
