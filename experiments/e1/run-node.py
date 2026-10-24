#!/usr/bin/env python3
"""Own a scratch node for the duration of a command, with guaranteed cleanup.

Usage: python3 run-node.py SPEC -- command [args...]
Environment: RPC_PORT=19944, BLOCK_TIME=1000, RPC_METHODS=unsafe,
STATE_PRUNING=archive, RPC_MAX_RESPONSE_SIZE=64, LOG=/tmp/pm-audit/e1/node.log.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import urllib.request

scratch = Path('/tmp/pm-audit/e1')
port = os.environ.get('RPC_PORT', '19944')
split = sys.argv.index('--')
spec = sys.argv[1]
command = sys.argv[split + 1:]
args = [os.environ.get('OMNI_NODE', str(scratch / 'bin/polkadot-omni-node-aarch64-apple-darwin')),
        '--chain', spec, '--dev', '--dev-block-time', os.environ.get('BLOCK_TIME', '1000'),
        '--rpc-methods', os.environ.get('RPC_METHODS', 'unsafe'),
        '--rpc-port', port, '--port', str(int(port) + 10000),
        '--rpc-max-response-size', os.environ.get('RPC_MAX_RESPONSE_SIZE', '64'),
        '--no-telemetry', '--tmp', '--no-prometheus']
pruning = os.environ.get('STATE_PRUNING', 'archive')
if pruning != 'default':
    args += ['--state-pruning', pruning]
logpath = Path(os.environ.get('LOG', str(scratch / 'node.log')))
logpath.with_suffix('.command.json').write_text(json.dumps(args, indent=2) + '\n')
with logpath.open('w') as log:
    node = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        for _ in range(120):
            if node.poll() is not None:
                raise RuntimeError(f'Node exited {node.returncode}; see {logpath}')
            try:
                request = urllib.request.Request(f'http://127.0.0.1:{port}',
                    json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'system_health', 'params': []}).encode(),
                    {'Content-Type': 'application/json'})
                with urllib.request.urlopen(request, timeout=1) as response:
                    if 'result' in json.load(response):
                        break
            except Exception:
                time.sleep(1)
        else:
            raise RuntimeError(f'Node not ready; see {logpath}')
        sys.exit(subprocess.call(command))
    finally:
        os.killpg(node.pid, signal.SIGINT) if node.poll() is None else None
        try:
            node.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(node.pid, signal.SIGKILL)
            node.wait()
