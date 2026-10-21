#!/usr/bin/env python3
"""Check recording-start graphics context in a real integrated-server report."""
import argparse
import hashlib
import json
import os
import platform
import shutil
import sys

try:
    from . import lab
except ImportError:
    import lab

try:
    from . import build_mod, build_pack
    from .test_culling_rays import run_fixture
except ImportError:
    import build_mod, build_pack
    from test_culling_rays import run_fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-display', action='store_true')
    parser.add_argument('--backend', choices=['opengl', 'vulkan'], help='Require this observed backend; default accepts the game-selected backend')
    args = parser.parse_args()
    root = build_pack.ROOT
    fixture = build_mod.build_integration_fixture('server-graphics',
        'dev.horizonlab.metrics.ServerGraphicsIntegration', fixture_dependencies={})
    version = build_pack.read_lock()['pack_version']
    mod = root / 'dist' / f'horizon-{version}.jar'
    digest = hashlib.sha256(mod.read_bytes()).hexdigest()
    instance = root / '.local/runtime' / ('server-graphics-' + (args.backend or 'default'))
    if instance.exists():
        raise RuntimeError('Use a fresh private fixture directory; existing worlds and reports are not overwritten')
    build_pack.prepare_instance('core-performance', instance, mod)
    if args.backend:
        options = instance / 'options.txt'
        import re
        text, count = re.subn(r'^preferredGraphicsBackend:.*$', 'preferredGraphicsBackend:' + json.dumps(args.backend), options.read_text(), flags=re.MULTILINE)
        if count != 1:
            raise AssertionError('Expected one backend preference')
        options.write_text(text)
    if hashlib.sha256((instance / 'mods' / mod.name).read_bytes()).hexdigest() != digest:
        raise AssertionError('Runtime mod differs from intended build')
    shutil.copy2(fixture, instance / 'mods/server-graphics-fixture.jar')
    command = [sys.executable, str(root / 'tools/runtime.py'), 'demo', '--instance', str(instance), '--heap', '3G', '--compact-headers']
    if platform.system() == 'Linux' and not args.native_display:
        command = ['xvfb-run', '-a', *command]
    run = run_fixture(command, instance, os.environ.copy())
    marker = 'HORIZON_SERVER_GRAPHICS_RESULT '
    results = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
    if run.returncode or len(results) != 1:
        raise RuntimeError('Integrated-server graphics fixture failed; inspect its private runtime log')
    result = results[0]
    if result.get('server_runtime_kind') != 'integrated' or result.get('client_graphics_capture_status') != 'captured_at_recording_start':
        raise AssertionError('Integrated graphics were not captured')
    if args.backend and result.get('graphics_backend', '').lower() != args.backend:
        raise AssertionError('Observed graphics backend differs from the requested experiment')
    if not (instance / 'saves/Demo_World/level.dat').is_file():
        raise AssertionError('Disposable world was not saved')
    report = {'schema': 1, 'platform': platform.platform(), 'architecture': platform.machine(),
              'minecraft': lab.VERSION, 'original_mod_sha256': digest, 'launcher_exit_code': run.returncode,
              'requested_backend': args.backend or 'pack_default', 'result': result, 'limits': ['Short report-context check, not FPS or physical M2 performance.',
                  'Graphics context is captured at recording start, not continuously monitored.',
                  'World files and full logs remain private.']}
    output = root / 'dist/server-graphics-smoke.json'
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    main()
