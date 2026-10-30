#!/usr/bin/env python3
"""Cross-build a Windows release without changing the installed Rust toolchain.

Run prepare_windows_toolchain.py first. Build products, Cargo cache and temporary
files stay under this repository. This verifies compilation, not Windows runtime.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import zipfile
import sys

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / 'target/tooling/windows'
TARGET = 'x86_64-pc-windows-gnullvm'

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--offline', action='store_true', help='Use only cached Cargo dependencies')
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--native-library-dir', type=Path, action='append', default=[],
                        help='Additional Windows SDK/import-library directory (repeatable)')
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error('--jobs must be positive')
    manifest = TOOLS / 'toolchain.json'
    if not manifest.is_file():
        parser.error('Run scripts/prepare_windows_toolchain.py first')
    tools = json.loads(manifest.read_text())
    env = os.environ.copy()
    for name, path in {
        'TMPDIR': ROOT / 'target/tooling/tmp',
        'XDG_CACHE_HOME': ROOT / 'target/tooling/cache',
        'CARGO_HOME': ROOT / 'target/tooling/cargo-home',
        'CARGO_TARGET_DIR': ROOT / 'target/windows-build',
    }.items():
        path.mkdir(parents=True, exist_ok=True)
        env[name] = str(path)
    env['RUSTC'] = tools['rustc']
    # --target keeps these flags off the host build scripts/procedural macros.
    rustflags = [
        '--sysroot', tools['sysroot'], '-C', f"linker={tools['linker']}",
        '-C', 'linker-flavor=ld.lld', '-C', 'link-self-contained=yes',
    ]
    # The gnullvm rust-mingw component lacks libopengl32.a (wgpu's GL backend
    # imports it). Generate it from the checked-in export list.
    import_libs = TOOLS / 'importlibs'
    import_libs.mkdir(parents=True, exist_ok=True)
    dlltool = shutil.which('llvm-dlltool') or tools.get('dlltool')
    if dlltool is None:
        parser.error('llvm-dlltool is required to create libopengl32.a (install LLVM)')
    subprocess.run([dlltool, '-m', 'i386:x86-64', '-d', str(ROOT / 'scripts/windows/opengl32.def'),
                    '-l', str(import_libs / 'libopengl32.a')], check=True)
    rustflags.extend(['-L', f'native={import_libs}'])
    for directory in args.native_library_dir:
        if not directory.is_dir():
            parser.error(f'Import-library directory does not exist: {directory}')
        rustflags.extend(['-L', f'native={directory.resolve()}'])
    env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(rustflags)
    command = [tools['cargo'], 'build', '--release', '--locked', '-p', 'mc4d',
               '--target', TARGET, '--jobs', str(args.jobs)]
    if args.offline:
        command.append('--offline')
    log_path = TOOLS / 'build.log'
    with log_path.open('w') as log:
        process = subprocess.Popen(command, cwd=ROOT, env=env,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        assert process.stdout is not None
        for line in process.stdout:
            print(line, end='', flush=True)
            log.write(line)
            log.flush()
        status = process.wait()
    binary = ROOT / 'target/windows-build' / TARGET / 'release/mc4d.exe'
    result = {
        'target': TARGET, 'command': command, 'rustflags': rustflags, 'status': status,
        'toolchain_manifest': str(manifest.relative_to(ROOT)),
        'log': str(log_path.relative_to(ROOT)), 'runtime_verified': False,
    }
    if status == 0 and binary.is_file():
        result.update(binary=str(binary.relative_to(ROOT)), size=binary.stat().st_size,
                      sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
        package = ROOT / 'target/dist/mc4d-windows-x86_64'
        package.mkdir(parents=True, exist_ok=True)
        shutil.copy2(binary, package / 'mc4d.exe')
        (package / 'Play.bat').write_text('@echo off\r\ncd /d "%~dp0"\r\nmc4d.exe --data-dir "%~dp0data"\r\n', newline='')
        (package / 'README.txt').write_text(
            'mc_4d Windows cross-build\n\n'
            'Run Play.bat to keep settings and saves in the adjacent data folder.\n'
            'All game textures, fonts and sounds are embedded in mc4d.exe.\n'
            'Video export additionally requires ffmpeg; normal play does not.\n\n'
            'This artifact was cross-compiled on Linux. Native Windows gameplay,\n'
            'graphics, audio and performance have not yet been verified.\n'
            'The game remains under development; see the repository feature audit.\n')
        notices = package / 'notices'
        notices.mkdir(exist_ok=True)
        for source, label in [
            ('crates/render/assets/minecraft-beta-1.5/NOTICE.txt', 'Minecraft-textures.txt'),
            ('crates/app/assets/audio/NOTICE.txt', 'Audio.txt'),
            ('crates/app/assets/lang/NOTICE.txt', 'Language.txt'),
            ('crates/app/assets/fonts/NOTICE-ClassicPixel.txt', 'ClassicPixel.txt'),
            ('crates/app/assets/fonts/NOTICE-Noto.txt', 'Noto.txt'),
            ('crates/app/assets/fonts/LICENSE-Noto', 'LICENSE-Noto.txt'),
            ('crates/app/assets/fonts/LICENSE-DejaVu', 'LICENSE-DejaVu.txt'),
        ]:
            shutil.copy2(ROOT / source, notices / label)
        archive = package.with_suffix('.zip')
        with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as output:
            for path in sorted(package.rglob('*')):
                if path.is_file() and 'data' not in path.relative_to(package).parts:
                    output.write(path, arcname=Path(package.name) / path.relative_to(package))
        result.update(package=str(archive.relative_to(ROOT)),
                      package_sha256=hashlib.sha256(archive.read_bytes()).hexdigest())
    (TOOLS / 'build-result.json').write_text(json.dumps(result, indent=2) + '\n')
    return status

if __name__ == '__main__':
    sys.exit(main())
