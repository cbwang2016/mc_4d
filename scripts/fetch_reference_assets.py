"""Download and verify the official legacy reference assets inside target/ only.

The pre-1.6 index also contains later-era resources; it is not a Beta 1.5
feature list. Original assets are references, not bundled game assets.
"""
import concurrent.futures
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1] / 'target/reference/beta-1.5'


def fetch(entry):
    name, meta = entry
    digest = meta['hash']
    assert len(digest) == 40 and all(c in '0123456789abcdef' for c in digest)
    dest = ROOT / 'objects' / digest[:2] / digest
    dest.parent.mkdir(parents=True, exist_ok=True)
    if not dest.exists() or hashlib.sha1(dest.read_bytes()).hexdigest() != digest:
        result = subprocess.run([
            'curl', '-fsSL', '--connect-timeout', '10', '--max-time', '45', '--retry', '1',
            f'https://resources.download.minecraft.net/{digest[:2]}/{digest}', '-o', str(dest),
        ], capture_output=True, text=True)
        if result.returncode:
            return {'name': name, 'status': 'failed', 'error': result.stderr[-500:]}
    data = dest.read_bytes()
    valid = len(data) == meta['size'] and hashlib.sha1(data).hexdigest() == digest
    return {'name': name, 'hash': digest, 'size': len(data), 'status': 'verified' if valid else 'invalid',
            'header_hex': data[:16].hex()}


def main():
    entries = json.loads((ROOT / 'pre-1.6.json').read_text())['objects']
    results = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        for result in pool.map(fetch, entries.items()):
            results.append(result)
            if len(results) % 50 == 0:
                print(f'{len(results)}/{len(entries)} assets inspected', flush=True)
    (ROOT / 'external-assets-audit.json').write_text(json.dumps(results, indent=2))
    failed = [r for r in results if r['status'] != 'verified']
    print(f'{len(results) - len(failed)} verified, {len(failed)} failed', flush=True)
    raise SystemExit(bool(failed))


if __name__ == '__main__':
    main()
