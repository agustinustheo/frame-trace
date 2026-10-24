#!/usr/bin/env python3
"""Validate the recorded RPC payloads and filter behavior, without a running node."""
import collections
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[2] / 'fixtures'
chains = sys.argv[1:] or [p.name for p in root.iterdir() if (p / 'manifest.json').exists()]

def read(path):
    return json.loads(path.read_text())

def canonical(events):
    return [{**e, 'data': {'stringValues': {k: v for k, v in e['data']['stringValues'].items() if k != 'ext_id'}}} for e in events]

for chain in chains:
    out = root / chain
    manifest = read(out / 'manifest.json')
    analysis = {'chain': chain, 'blocks': [], 'methodKeys': {}, 'totalNullEventParents': 0, 'totalSetEventParents': 0}
    for block in manifest['blocks']:
        directory = out / str(block['number'])
        header = read(directory / 'header.json')
        assert int(header['number'], 16) == block['number']
        assert read(directory / 'block.json')['block']['header'] == header
        assert (directory / 'metadata.hex').read_text().startswith('0x6d657461')
        decoded = read(directory / 'extrinsics.json')
        assert len(decoded) == len(read(directory / 'block.json')['block']['extrinsics'])
        dispatch_events = read(directory / 'events.json')
        assert not any(e['event']['method'] == 'ExtrinsicFailed' for e in dispatch_events)
        if block['label'] == 'batch':
            assert any(e['event']['section'] == 'utility' and e['event']['method'] == 'BatchCompleted' for e in dispatch_events)
            assert sum(e['event']['section'] == 'balances' and e['event']['method'] == 'Transfer' for e in dispatch_events) == 3
        trace = read(directory / 'trace-state.json')
        if 'error' in trace:
            assert 'official' in chain, f'Unexpected replay failure in {chain}/{block["number"]}'
            analysis['blocks'].append({**block, 'errorCode': trace['error']['code']})
            continue
        trace = trace['blockTrace']
        assert trace['blockHash'] == block['hash'][2:]
        assert trace['parentHash'] == header['parentHash'][2:]
        assert set(trace) == {'blockHash', 'parentHash', 'tracingTargets', 'storageKeys', 'methods', 'spans', 'events'}
        events = trace['events']
        for event in events:
            assert set(event) == {'target', 'data', 'parentId'}
            assert set(event['data']) == {'stringValues'}
            values = event['data']['stringValues']
            assert all(isinstance(k, str) and isinstance(v, str) for k, v in values.items())
            method = values['method']
            keys = sorted(values)
            assert analysis['methodKeys'].setdefault(method, keys) == keys
            analysis['totalNullEventParents' if event['parentId'] is None else 'totalSetEventParents'] += 1
        for span in read(directory / 'trace-all-targets.json')['blockTrace']['spans']:
            assert set(span) == {'id', 'parentId', 'name', 'target', 'wasm'}
        if block['label'] == 'large-put':
            value = next(e['data']['stringValues'] for e in events if e['data']['stringValues'].get('key') == '3a6672616d652d74726163652d65313a01' and e['data']['stringValues']['method'] == 'Put')
            assert value['value'] == 'Some(' + 'a5' * 4096 + ')'
            assert len(value['value_encoded']) == 2051 and '...' in value['value_encoded']
            assert read(directory / 'probe-storage.json') == '0x' + 'a5' * 4096
        if block['label'] == 'clear-prefix':
            assert read(directory / 'probe-storage.json') is None
            assert sum(s['name'] == 'clear_prefix_version_2' for s in read(directory / 'trace-all-targets.json')['blockTrace']['spans']) == 2
        repeat = read(directory / 'trace-repeat.json')['blockTrace']['events']
        assert canonical(events) == canonical(repeat), f'Replay order differs: {chain}/{block["number"]}'
        summary = read(directory / 'summary.json')
        prefix = summary['storageKeys']['systemAccount']
        variants = {
            'account': [e for e in events if e['data']['stringValues']['key'].startswith(prefix)],
            'account-0x': [],
            'method-prefix': [e for e in events if e['data']['stringValues']['method'].startswith('Ge')],
            'methods': [e for e in events if e['data']['stringValues']['method'].startswith(('Get', 'Put'))],
            'clear-prefix-method': [],
            'omitted': [],
        }
        for variant, expected in variants.items():
            actual = read(directory / f'trace-{variant}.json')['blockTrace']['events']
            assert canonical(actual) == canonical(expected), f'Filter differs: {chain}/{block["number"]}/{variant}'
        analysis['blocks'].append({**block, 'events': len(events), 'methods': dict(collections.Counter(e['data']['stringValues']['method'] for e in events)),
            'runtimeSpans': len(read(directory / 'trace-runtime.json')['blockTrace']['spans']),
            'allTargetsSpans': len(read(directory / 'trace-all-targets.json')['blockTrace']['spans'])})
    timings = read(out / 'measurements.json')
    analysis['traceMeasurements'] = [m for m in timings if m['method'] == 'state_traceBlock']
    (out / 'analysis.json').write_text(json.dumps(analysis, indent=2) + '\n')
    print(f'{chain}: {len(manifest["blocks"])} blocks verified')

probes = root / 'e1-probes'
if (probes / 'safe/trace-safe.json').exists():
    assert read(probes / 'safe/trace-safe.json')['error']['code'] == 4003
if (probes / 'default-pruning/trace-after.json').exists():
    assert 'blockTrace' in read(probes / 'default-pruning/trace-before.json')['result']
    assert read(probes / 'default-pruning/trace-after.json')['error']['code'] == 4001
    assert 'State already discarded' in read(probes / 'default-pruning/trace-after.json')['error']['message']
    assert 'blockTrace' in read(probes / 'default-pruning/trace-recent.json')['result']
if (probes / 'response-limit/trace-full.json').exists():
    assert read(probes / 'response-limit/trace-full.json')['error']['code'] == -32008
    assert len(read(probes / 'response-limit/trace-filtered.json')['blockTrace']['events']) == 1
print('Available safe-RPC, pruning and response-limit probes verified')
