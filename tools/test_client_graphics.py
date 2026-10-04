#!/usr/bin/env python3
"""Start real clients and verify a nonuniform GPU framebuffer readback."""

import argparse
import hashlib
import json
import os
import platform
import shutil
import sys

try:
    from . import build_mod, build_pack
    from .test_culling_rays import run_fixture
    from .test_dispatch import failure_summary
except ImportError:
    import build_mod
    import build_pack
    from test_culling_rays import run_fixture
    from test_dispatch import failure_summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-display', action='store_true')
    parser.add_argument('--profile', action='append', choices=sorted(build_pack.PROFILES))
    args = parser.parse_args()
    root = build_pack.ROOT
    fixture = build_mod.build_integration_fixture('client-graphics',
        'dev.horizonlab.metrics.ClientGraphicsIntegration',
        fixture_mixins=['GraphicsBackendTraceMixin'], fixture_dependencies={})
    version = build_pack.read_lock()['pack_version']
    mod = root / 'dist' / f'horizon-lab-{version}.jar'
    original_hash = hashlib.sha256(mod.read_bytes()).hexdigest()
    results = []
    for profile in args.profile or ['core-performance', 'm2-balanced', 'vulkan']:
        instance = root / '.local/runtime' / ('client-graphics-' + profile)
        build_pack.prepare_instance(profile, instance, mod)
        shutil.copy2(fixture, instance / 'mods/client-graphics-fixture.jar')
        command = [sys.executable, str(root / 'tools/runtime.py'), 'demo',
                   '--instance', str(instance), '--heap', '3G']
        if platform.system() == 'Linux' and not args.native_display:
            command = ['xvfb-run', '-a', *command]
        try:
            run = run_fixture(command, instance, os.environ.copy())
        except RuntimeError:
            print('HORIZON_FIXTURE_FAILURE ' + json.dumps({'fixture': 'client-graphics',
                'profile': profile, 'completed': False}), flush=True)
            raise RuntimeError('Graphics probe did not finish; diagnostics remain private') from None
        marker = 'HORIZON_CLIENT_GRAPHICS_RESULT '
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        trace_marker = 'HORIZON_GRAPHICS_BACKEND_ATTEMPT '
        attempts = [json.loads(line.split(trace_marker, 1)[1]) for line in run.stdout.splitlines() if trace_marker in line]
        if run.returncode or len(reports) != 1:
            diagnostics = failure_summary(run, len(reports))
            diagnostics['backend_attempts'] = attempts
            print('HORIZON_FIXTURE_FAILURE ' + json.dumps(diagnostics), flush=True)
            raise RuntimeError('Graphics probe failed; diagnostics remain private')
        report = reports[0]
        if report['title_screen_observed'] is not True or report['title_client_ticks'] < 40:
            raise AssertionError('Client never reached the title screen')
        if report['sampled_pixels'] < 256 or report['distinct_sampled_rgb'] < 8:
            raise AssertionError('GPU readback was uniform')
        if hashlib.sha256((instance / 'mods' / mod.name).read_bytes()).hexdigest() != original_hash:
            raise AssertionError('Runtime original mod differs from the intended build')
        screenshot = instance / 'client-graphics-private.png'
        if not screenshot.is_file() or hashlib.sha256(screenshot.read_bytes()).hexdigest() != report['private_png_sha256']:
            raise AssertionError('Private screenshot evidence is missing or changed')
        report.update(profile=profile, launcher_exit_code=run.returncode, backend_attempts=attempts)
        results.append(report)
        print(json.dumps(report), flush=True)
    output = root / 'dist/client-graphics-smoke.json'
    output.write_text(json.dumps({'schema': 1, 'platform': platform.platform(),
        'architecture': platform.machine(), 'minecraft': '26.3',
        'original_mod_sha256': original_hash, 'results': results,
        'limits': ['Title-screen startup and nonuniform framebuffer readback only.',
                  'No world gameplay, visual-correctness, FPS or M2 Pro performance claim.',
                  'Vulkan preference can fall back; inspect the observed backend.',
                  'PNG images, assets and full logs remain in the private runtime directory.']}, indent=2) + '\n')
    print('Client graphics startup/readback checks passed.', flush=True)


if __name__ == '__main__':
    main()
