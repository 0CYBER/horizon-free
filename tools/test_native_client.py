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
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking


def resource_fingerprint(root):
    files = [root / 'facts.json', root / 'model-states.json', root / 'block-tags.json', root / 'horizon-resources.json',
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
               *sorted((lab.ROOT / 'crates/horizon-client/src').rglob('*')),
               *sorted((lab.ROOT / 'crates/horizon-assets/src').rglob('*.rs'))]
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaGallerySelectionOracle.java'
    helper = oracle.with_name('VanillaCuboidModelOracle.java')
    sources += [oracle, helper, lab.ROOT/'tools/test_model_baking_parity.py', lab.ROOT/'tools/test_rust_parity.py', lab.ROOT/'tools/lab.py', lab.ROOT/'tools/prepare_native_client.py']
    for crate in ['horizon-core', 'horizon-world']:
        sources += [lab.ROOT/'crates'/crate/'Cargo.toml', *sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
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
            or result.get('model_quads') != 209 or len(result.get('model_gallery', [])) != 16
            or result.get('sprite_layers') != 18 or result.get('animated_sprites') != 1
            or not result.get('adapter') or min(result.get('window_size', [0])) <= 0):
        raise RuntimeError('Incomplete native client presentation: ' + str(report))
    lock, jars, _ = base.verify_artifacts()
    classes = root/'gallery-classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    native = root/'gallery-selection.json'
    with (root/'gallery-selection.log').open('w') as log:
        subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-cp', dependencies,
                        '-d', str(classes), str(oracle), str(helper)], cwd=lab.ROOT, check=True,
                       stdout=log, stderr=subprocess.STDOUT, timeout=3600)
        subprocess.run(['java', '-Xmx512M', '-cp', str(classes)+os.pathsep+dependencies,
                        'dev.horizonlab.parity.VanillaGallerySelectionOracle', str(report), str(native)],
                       cwd=lab.ROOT, check=True, stdout=log, stderr=subprocess.STDOUT, timeout=3600)
    selections = json.loads(native.read_text())
    if len(selections) != len(result['model_gallery']):
        raise RuntimeError('Incomplete native gallery selection check')
    result['native_selection'] = {'matched_placements': len(selections),
                                  'selected_parts': sum(len(s['models']) for s in selections),
                                  'client_jar_sha256': lab.digest(jars['client'], 'sha256'),
                                  'output_sha256': lab.digest(native, 'sha256'),
                                  'scope': 'Actual native registry states, blockstate dispatch, root baking with tagged empty geometry, and legacy collection using state.getSeed(position). Model names and transform matrices match scene selections; no shader/pixel parity claim.'}
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
