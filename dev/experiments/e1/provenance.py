#!/usr/bin/env python3
"""Save reproducible inputs and node logs after the four capture runs."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

experiment = Path(__file__).resolve().parent
fixtures = experiment.parents[1] / 'fixtures'
scratch = Path('/tmp/pm-audit/e1')
people = Path('/tmp/pm-audit/individuality-community')
sdk = scratch / 'polkadot-sdk-stable'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def command(args):
    return subprocess.check_output(args, text=True).strip()

for chain, traced, patched, logname in [
    ('next-people-paseo-official', False, False, 'node-official'),
    ('next-people-paseo-traced-official', True, False, 'node-traced-official'),
    ('next-people-paseo', False, True, 'node-proof-aware'),
    ('next-people-paseo-traced', True, True, 'node-traced-proof-aware'),
]:
    out = fixtures / chain
    binary = scratch / ('target-node/release/polkadot-omni-node' if patched else 'bin/polkadot-omni-node-aarch64-apple-darwin')
    wasm = scratch / ('people-traced.wasm' if traced else 'people-untraced.wasm')
    spec = scratch / ('people-traced.json' if traced else 'people-untraced.json')
    provenance = {
        'peopleCommit': command(['git', '-C', str(people), 'rev-parse', 'HEAD']),
        'peopleCargoLockSha256': sha(people / 'Cargo.lock'),
        'sdkCommit': command(['git', '-C', str(sdk), 'rev-parse', 'HEAD']),
        'sdkRelease': 'polkadot-stable2606-2',
        'cargo': command(['cargo', '+1.93.0', '--version']),
        'rustc': command(['rustc', '+1.93.0', '--version']),
        'nodeJs': command(['node', '--version']),
        'runtimeFeatures': ['with-tracing'] if traced else [],
        'runtimeWasmSha256': sha(wasm),
        'runtimeWasmBytes': wasm.stat().st_size,
        'chainSpecSha256': sha(spec),
        'nodeVersion': command([str(binary), '--version']),
        'nodeBinarySha256': sha(binary),
        'proofAwareDevNodePatch': 'dev/experiments/e1/omni-dev-proof-replay.patch' if patched else None,
        'runtimeFeaturePatch': 'dev/experiments/e1/people-with-tracing.patch' if traced else None,
        'nodeCommand': json.loads((scratch / f'{logname}.command.json').read_text()),
        'fixtureResultFormat': 'RPC result; full JSON-RPC envelope on an RPC error',
    }
    (out / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
    shutil.copyfile(scratch / f'{logname}.log', out / 'node.log')
    print(chain)
