"""Linux runner; all databases live in a fresh /tmp directory, never project data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import selectors
import shutil
import subprocess
import tempfile
import time

p = argparse.ArgumentParser()
p.add_argument('--binary', required=True)
p.add_argument('--output', required=True)
p.add_argument('--repeats', type=int, default=3)
args = p.parse_args()
root = Path(tempfile.mkdtemp(prefix='clustering-turso-validation-'))
results = {'environment': {'platform': platform.platform(), 'cpu_count': os.cpu_count(),
           'meminfo': Path('/proc/meminfo').read_text().splitlines()[:3],
           'filesystem': subprocess.check_output(['df', '-T', str(root)], text=True),
           'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
           'root': str(root)}, 'runs': []}

def run(mode, engine, path, **extra):
    start = time.monotonic()
    try:
        r = subprocess.run([args.binary, mode, engine, str(path)], capture_output=True,
                           text=True, timeout=180)
        item = {'mode': mode, 'engine': engine, 'exit_code': r.returncode,
                'stdout': r.stdout, 'stderr': r.stderr, **extra}
    except subprocess.TimeoutExpired as e:
        item = {'mode': mode, 'engine': engine, 'error': 'timeout',
                'stdout': str(e.stdout), 'stderr': str(e.stderr), **extra}
    item['elapsed_seconds'] = time.monotonic() - start
    results['runs'].append(item)
    print(json.dumps(item), flush=True)
    return item

for engine in ['sqlite', 'turso']:
    for trial in range(args.repeats):
        folder = root / f'{engine}-suite-{trial}'
        folder.mkdir()
        run('suite', engine, folder / 'analysis.db', trial=trial)
    for trial in range(args.repeats):
        folder = root / f'{engine}-crash-{trial}'
        folder.mkdir()
        db = folder / 'analysis.db'
        child = subprocess.Popen([args.binary, 'crash', engine, str(db)],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        selector = selectors.DefaultSelector()
        selector.register(child.stdout, selectors.EVENT_READ)
        output = b''
        deadline = time.monotonic() + 60
        while b'UNCOMMITTED\n' not in output and time.monotonic() < deadline:
            if selector.select(timeout=1):
                chunk = os.read(child.stdout.fileno(), 65536)
                if not chunk:
                    break
                output += chunk
        reached = b'ACKNOWLEDGED\n' in output and b'UNCOMMITTED\n' in output
        if child.poll() is None:
            child.kill()  # SIGKILL: deliberately no graceful database shutdown.
        _, err = child.communicate(timeout=10)
        selector.close()
        item = {'mode': 'kill', 'engine': engine, 'trial': trial,
                'reached_fault_point': reached, 'exit_code': child.returncode,
                'stdout': output.decode(errors='replace'), 'stderr': err.decode(errors='replace')}
        results['runs'].append(item)
        print(json.dumps(item), flush=True)
        if reached:
            verified = run('verify', engine, db, trial=trial)
            if verified.get('exit_code') == 0:
                # All engine processes have exited. Copy main DB and every sidecar.
                backup = root / f'{engine}-backup-{trial}'
                shutil.copytree(folder, backup)
                hashes = {f.name: hashlib.sha256(f.read_bytes()).hexdigest()
                          for f in folder.iterdir() if f.is_file()}
                assert all(hashlib.sha256((backup/n).read_bytes()).hexdigest() == digest
                           for n, digest in hashes.items())
                run('verify', engine, backup / 'analysis.db', trial=trial,
                    backup_method='stopped-process copy of complete DB directory',
                    backup_sha256=hashes)

Path(args.output).write_text(json.dumps(results, indent=2) + '\n')
print(f'Results: {args.output}; preserved test databases: {root}', flush=True)
