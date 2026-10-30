#!/usr/bin/env python3
"""Prepare matching official Windows Rust components without modifying rustup.

Archives and the isolated target sysroot live only under target/tooling/windows.
The installed Rust channel manifest supplies the exact component URLs and hashes.
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
TARGET = 'x86_64-pc-windows-gnullvm'
TOOLS = ROOT / 'target/tooling/windows'

def sha256(path: Path) -> str:
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()

def prepare() -> dict:
    host = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
    manifest_path = host / 'lib/rustlib/multirust-channel-manifest.toml'
    manifest = tomllib.loads(manifest_path.read_text())
    version = manifest['pkg']['rustc']['version'].split()[0]
    sysroot = TOOLS / f'rust-{version}'
    archives = TOOLS / 'downloads'
    staging = TOOLS / 'components'
    for directory in (archives, staging, sysroot):
        directory.mkdir(parents=True, exist_ok=True)
    verified = []
    for package in ('rust-std', 'rust-mingw'):
        component = manifest['pkg'][package]['target'][TARGET]
        if not component['available']:
            raise RuntimeError(f'{package} is unavailable for {TARGET} with Rust {version}')
        url, expected = component['xz_url'], component['xz_hash']
        if not url.startswith('https://static.rust-lang.org/dist/'):
            raise RuntimeError('refusing a component outside the official Rust distribution server')
        archive = archives / url.rsplit('/', 1)[1]
        if not archive.exists() or sha256(archive) != expected:
            temporary = archive.with_suffix(archive.suffix + '.part')
            print(f'Downloading {archive.name}', flush=True)
            with urllib.request.urlopen(url, timeout=90) as response, temporary.open('wb') as output:
                shutil.copyfileobj(response, output)
            if sha256(temporary) != expected:
                raise RuntimeError(f'SHA-256 mismatch for {archive.name}; archive was not installed')
            temporary.replace(archive)
        extracted = staging / archive.name.removesuffix('.tar.xz')
        stamp = extracted / '.verified-sha256'
        if not stamp.exists() or stamp.read_text().strip() != expected:
            if extracted.exists():
                shutil.rmtree(extracted)
            print(f'Extracting verified {archive.name}', flush=True)
            with tarfile.open(archive) as source:
                source.extractall(staging, filter='data')
            stamp.write_text(expected + '\n')
        for name in (extracted / 'components').read_text().splitlines():
            libraries = extracted / name / 'lib'
            if libraries.exists():
                shutil.copytree(libraries, sysroot / 'lib', dirs_exist_ok=True)
        verified.append({'package': package, 'url': url, 'sha256': expected})
    linker = host / 'lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld'
    if not linker.is_file():
        raise RuntimeError(f'host Rust linker missing: {linker}')
    result = {
        'target': TARGET, 'rust_version': version, 'manifest_date': manifest['date'],
        'manifest_sha256': sha256(manifest_path), 'host_sysroot': str(host),
        'sysroot': str(sysroot), 'rustc': str(host / 'bin/rustc'),
        'cargo': str(host / 'bin/cargo'), 'linker': str(linker), 'components': verified,
    }
    (TOOLS / 'toolchain.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2), flush=True)
    return result

if __name__ == '__main__':
    os.environ.setdefault('TMPDIR', str(ROOT / 'target/tooling/tmp'))
    prepare()
