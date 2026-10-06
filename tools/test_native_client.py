#!/usr/bin/env python3
"""Launch the Rust development scene and check bounded GPU presentation.

Requires prepared private resources and a desktop display (or --vscreen on
headless Linux). This is a renderer startup check, not a gameplay parity gate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

try:
    from . import lab
except ImportError:
    import lab


def resource_fingerprint(root):
    files = [root / 'facts.json', root / 'block-tags.json', root / 'horizon-resources.json',
             *sorted(path for path in (root / 'assets').rglob('*') if path.is_file())]
    digest = hashlib.sha256()
    for path in files:
        data = path.read_bytes()
        digest.update(path.relative_to(root).as_posix().encode() + b'\0')
        digest.update(len(data).to_bytes(8, 'big')); digest.update(data)
    return {'files': len(files), 'sha256': digest.hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--vscreen', action='store_true')
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    if args.prepare:
        subprocess.run([sys.executable, str(lab.ROOT / 'tools/prepare_native_client.py'),
                        '--prepare'], cwd=lab.ROOT, check=True)
    root = lab.LOCAL / 'horizon-client'
    root.mkdir(parents=True, exist_ok=True)
    report = root / 'startup-smoke.json'
    report.unlink(missing_ok=True)
    resources = resource_fingerprint(root / '26.3')
    sources = [lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock', Path(__file__).resolve(),
               lab.ROOT / 'crates/horizon-client/Cargo.toml', lab.ROOT / 'crates/horizon-assets/Cargo.toml',
               *sorted((lab.ROOT / 'crates/horizon-client/src').glob('*')),
               *sorted((lab.ROOT / 'crates/horizon-assets/src').glob('*.rs'))]
    tested_sources = {
        str(path.relative_to(lab.ROOT)): lab.digest(path, 'sha256')
        for path in sources
        if path.is_file()
    }
    environment = dict(os.environ, RUSTUP_TOOLCHAIN='stable')
    subprocess.run(['cargo', 'build', '--locked', '-p', 'horizon-client'],
                   cwd=lab.ROOT, env=environment, check=True, timeout=3600)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'],
        cwd=lab.ROOT, env=environment))
    executable = Path(metadata['target_directory']) / 'debug' / (
        'horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    command = (['vscreen', 'run'] if args.vscreen else []) + [
        str(executable), '--radius', '1', '--frames', '30',
        '--smoke-report', str(report), '--timeout-seconds', '60']
    with (root / 'startup-smoke.log').open('w') as log:
        subprocess.run(command, cwd=lab.ROOT, env=environment, check=True,
                       stdout=log, stderr=subprocess.STDOUT, timeout=120)
    result = json.loads(report.read_text())
    if (result.get('error') is not None or result.get('presented_frames') != 30
            or result.get('resident_chunks') != 9 or result.get('merged_quads', 0) <= 0
            or result.get('model_quads') != 145 or len(result.get('model_gallery', [])) != 12
            or result.get('sprite_layers') != 18 or result.get('animated_sprites') != 1
            or not result.get('adapter') or min(result.get('window_size', [0])) <= 0):
        raise RuntimeError('Incomplete native client presentation: ' + str(report))
    if resource_fingerprint(root / '26.3') != resources:
        raise RuntimeError('Resources changed during native client verification')
    result['resource_files'] = resources
    result['executable_sha256'] = lab.digest(executable, 'sha256')
    result['resource_manifest_sha256'] = lab.digest(
        root / '26.3/horizon-resources.json', 'sha256')
    actual_sources = {str(path.relative_to(lab.ROOT)): lab.digest(path, 'sha256') for path in sources if path.is_file()}
    if actual_sources != tested_sources:
        raise RuntimeError('Sources changed during native client verification')
    result['sources'] = tested_sources
    lab.write_json(report, result)
    print(f'Native renderer startup passed: {report}')


if __name__ == '__main__':
    main()
