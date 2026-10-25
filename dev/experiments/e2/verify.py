#!/usr/bin/env python3
"""Compare every ordered field of every keyed state event; retain small results.

Usage: python3 dev/experiments/e2/verify.py SCRATCH_CAPTURE_DIR RESULTS_DIR
"""
import collections
import json
from pathlib import Path
import statistics
import sys

source, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
manifest = json.loads((source / 'manifest.json').read_text())
results = []
for block in manifest['blocks']:
    folder = source / block['label']
    summary = json.loads((folder / 'summary.json').read_text())
    events = json.loads((folder / 'events.json').read_text())
    spans = json.loads((folder / 'spans.json').read_text())
    oracle = json.loads((folder / 'oracle.json').read_text())['blockTrace']['events']
    canonical = lambda fields: {k: v for k, v in fields.items() if k != 'ext_id'}
    expected = [canonical(e['data']['stringValues']) for e in oracle]
    actual = [canonical(e['fields']) for e in events if e['target'] == 'state' and 'key' in e['fields']]
    assert actual == expected, f"{block['label']}: keyed event stream mismatch"
    assert summary['root_check'] and summary['offline_verification'] and summary['warm_rpc_calls'] == 0
    by_id = {s['id']: s for s in spans}
    for span in spans:
        assert span['end_ns'] is not None, (block['label'], span)
        if span['parent_id'] is not None:
            parent = by_id[span['parent_id']]
            assert parent['start_ns'] <= span['start_ns'] <= span['end_ns'] <= parent['end_ns'], (parent, span)
    for event in events:
        if event['parent_id'] is not None:
            parent = by_id[event['parent_id']]
            assert parent['start_ns'] <= event['at_ns'] <= parent['end_ns'], (parent, event)
    storage_events = [e for e in events if e['target'] == 'state' and
                      e['fields'].get('method') not in ('Core_execute_block', None)]
    for event in storage_events:
        host = by_id[event['parent_id']]
        assert host['target'].startswith(('sp_io::storage', 'sp_io::default_child_storage'))
        assert host['duration_ns'] > 0
    extra = collections.Counter(e['fields'].get('method', e['fields'].get('message'))
                                for e in events if e['target'] == 'state' and 'key' not in e['fields'])
    iteration = collections.Counter(e['fields']['method'] for e in events if e['target'] == 'e2_backend')
    median_ms = statistics.median(summary['warm_replay_ms'])
    ext_ms = {k: statistics.median(v) for k, v in summary.get('warm_extrinsic_ms', {}).items()}
    row = {**summary, 'label': block['label'], 'oracle_match': True, 'keyed_events': len(actual),
           'extra_events': dict(extra), 'backend_iteration_events': dict(iteration),
           'warm_median_ms': median_ms, 'extrinsic_median_ms': ext_ms,
           'oracle_fields_ignored': ['ext_id'], 'span_intervals_and_parents_valid': True}
    row['storage_events_with_real_host_durations'] = len(storage_events)
    results.append(row)
    (output / (block['label'] + '.json')).write_text(json.dumps(row, indent=2) + '\n')
    print(f"{block['label']:20} keyed={len(actual):3} match=yes root=yes warm={median_ms:.3f}ms RPC={summary['rpc_calls']} extras={dict(extra)}")
    if block['label'] == 'batch' and summary['wasm_spans']:
        nested = []
        for span in spans:
            if span['wasm'] and span['name'] in ('batch', 'transfer_keep_alive'):
                ancestors, current = [], span
                while current:
                    ancestors.append({'id': current['id'], 'name': current['name'], 'target': current['target']})
                    current = by_id.get(current['parent_id'])
                nested.append({'path': list(reversed(ancestors)), 'duration_ns': span['duration_ns']})
        assert len([s for s in nested if s['path'][-1]['name'] == 'transfer_keep_alive']) == 3
        (output / 'batch-nesting.json').write_text(json.dumps(nested, indent=2) + '\n')
    examples = [e for e in events if e['target'] == 'state' and e['fields'].get('method') in ('ClearPrefix', 'StorageRoot')]
    (output / (block['label'] + '-extra-events.json')).write_text(json.dumps(examples, indent=2) + '\n')

assert len(results) == 8
assert json.loads((source / 'probe-after-kill.json').read_text()) is None
(output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
(output / 'all-blocks.json').write_text(json.dumps(results, indent=2) + '\n')
