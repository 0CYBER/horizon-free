#!/usr/bin/env python3
"""Start real clients and verify a nonuniform GPU framebuffer readback."""

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import sys

try:
    from . import lab
except ImportError:
    import lab

try:
    from . import build_mod, build_pack
    from .test_culling_rays import FixtureTimeout, run_fixture
    from .test_dispatch import failure_summary
except ImportError:
    import build_mod
    import build_pack
    from test_culling_rays import FixtureTimeout, run_fixture
    from test_dispatch import failure_summary



def graphics_failure_summary(run, report_count):
    """Selected stages/types/codes/methods only; messages and paths stay private."""
    result = failure_summary(run, report_count)
    pattern = r'(?<![\w.$])((?:java|org\.lwjgl|com\.mojang|net\.minecraft|net\.fabricmc|org\.spongepowered|dev\.horizonlab)\.[A-Za-z_$][\w.$]{0,160}(?:Exception|Error))(?=[:\s]|$)'
    observed = set(result['java_exception_types']) | set(re.findall(pattern, run.stdout, re.ASCII))
    expected_auth = {name for name in observed if name.startswith(('com.mojang.authlib.', 'com.mojang.realmsclient.'))}
    result['java_exception_types'] = sorted(observed - expected_auth)[:8]
    result['expected_demo_authentication_errors_observed'] = bool(expected_auth)
    for key, marker in [('backend_attempts', 'HORIZON_GRAPHICS_BACKEND_ATTEMPT '),
                        ('created_devices', 'HORIZON_GRAPHICS_DEVICE_CREATED ')]:
        result[key] = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line][:16]
    result['client_initializer_reached'] = 'HORIZON_CLIENT_GRAPHICS_INITIALIZER_READY' in run.stdout
    result['readback_requested'] = 'HORIZON_CLIENT_GRAPHICS_READBACK_REQUESTED' in run.stdout
    result['readback_callback_received'] = 'HORIZON_CLIENT_GRAPHICS_READBACK_RECEIVED' in run.stdout
    inspection_marker = 'HORIZON_CLIENT_GRAPHICS_READBACK_INSPECTED '
    inspections = []
    fields = ('framebuffer_width', 'framebuffer_height', 'sampled_pixels', 'distinct_sampled_rgb')
    for line in run.stdout.splitlines():
        if inspection_marker not in line:
            continue
        try:
            item = json.loads(line.split(inspection_marker, 1)[1])
        except json.JSONDecodeError:
            continue
        if isinstance(item, dict) and all(type(item.get(field)) is int
                and 0 <= item[field] <= 2_147_483_647 for field in fields):
            inspections.append({field: item[field] for field in fields})
        if len(inspections) == 4:
            break
    result['readback_inspections'] = inspections
    waiting_marker = 'HORIZON_CLIENT_GRAPHICS_WAIT_STATE '
    boolean_fields = ('world_loaded', 'player_present', 'overlay_present',
                      'screen_present', 'window_active', 'paused')
    result['last_waiting_state'] = None
    for line in run.stdout.splitlines():
        if waiting_marker not in line:
            continue
        try:
            state = json.loads(line.split(waiting_marker, 1)[1])
        except json.JSONDecodeError:
            continue
        if (isinstance(state, dict) and all(type(state.get(field)) is bool for field in boolean_fields)
                and type(state.get('ready_client_ticks')) is int and 0 <= state['ready_client_ticks'] <= 2_147_483_647):
            result['last_waiting_state'] = {field: state[field] for field in (*boolean_fields, 'ready_client_ticks')}
    # These exact messages belong to our private fixture. Never emit arbitrary
    # exception messages or infer that a separately logged Vulkan code caused it.
    categories = {'Uniform title readback': 'uniform_readback',
                  'Incomplete title framebuffer': 'small_framebuffer',
                  'Private client graphics readback failed': 'callback_exception',
                  'Required shader pack is inactive': 'shader_inactive',
                  'Unexpected active shader pack': 'unexpected_shader_pack'}
    result['probe_failure_categories'] = [category for message, category in categories.items()
        if 'java.lang.AssertionError: ' + message in run.stdout]
    result['vulkan_error_codes'] = sorted(set(re.findall(r'\bVK_ERROR_[A-Z_0-9]{1,100}\b', run.stdout)))[:16]
    result['fatal_jvm_error'] = 'A fatal error has been detected by the Java Runtime Environment' in run.stdout
    # Only Java stack-frame lines from graphics namespaces. Strip module prefixes,
    # filenames, line numbers and messages; these identify locations, not causes.
    stack_pattern = (r'^\s*at (?:[A-Za-z0-9_.@$-]{1,80}/{1,2})?'
                     r'((?:com\.mojang\.(?:renderpearl|blaze3d)\.|'
                     r'net\.minecraft\.client\.(?:renderer\.|Screenshot\.))'
                     r'[A-Za-z0-9_.$<>]{1,180})\(')
    methods = re.findall(stack_pattern, run.stdout, re.MULTILINE | re.ASCII)
    result['observed_graphics_stack_methods'] = list(dict.fromkeys(methods))[:8]
    return result


def validate_shader_report(report, required):
    if required and (report.get('shader_required') is not True
            or report.get('iris_capture_status') != 'available'
            or report.get('iris_shaders_enabled') is not True
            or report.get('iris_expected_pack_matches') is not True):
        raise AssertionError('Required runtime shader activation was not verified')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-display', action='store_true')
    parser.add_argument('--profile', action='append', choices=sorted(build_pack.PROFILES))
    parser.add_argument('--backend', choices=['opengl', 'vulkan'], help='Override backend in these private instances only')
    parser.add_argument('--world', action='store_true', help='Create/open a disposable built-in demo world instead of stopping at the title')
    parser.add_argument('--shaders', action='store_true', help='Require the pinned cinematic shader pack in a private world instance')
    args = parser.parse_args()
    if args.shaders and (not args.world or args.profile not in [None, ['cinematic']]):
        parser.error('--shaders requires --world and the cinematic profile')
    root = build_pack.ROOT
    fixture = build_mod.build_integration_fixture('client-graphics',
        'dev.horizonlab.metrics.ClientGraphicsIntegration',
        fixture_mixins=['GraphicsBackendTraceMixin'], fixture_dependencies={})
    version = build_pack.read_lock()['pack_version']
    mod = root / 'dist' / f'horizon-{version}.jar'
    original_hash = hashlib.sha256(mod.read_bytes()).hexdigest()
    results = []
    for profile in args.profile or (['cinematic'] if args.shaders else ['core-performance', 'm2-balanced']):
        instance = root / '.local/runtime' / ('client-graphics-' + profile
            + ('-override-' + args.backend if args.backend else '') + ('-world' if args.world else '')
            + ('-shaders' if args.shaders else ''))
        build_pack.prepare_instance(profile, instance, mod)
        if args.world:
            (instance / 'config/client-graphics-world.flag').touch()
        if args.shaders:
            expected = build_pack.read_lock()['projects']['complementary-reimagined']['filename']
            (instance / 'config/iris.properties').write_text('enableShaders=true\nshaderPack=' + expected + '\n')
            (instance / 'config/client-graphics-shaders.flag').write_text(expected + '\n')
        if args.backend:
            options = instance / 'options.txt'
            text, count = re.subn(r'^preferredGraphicsBackend:.*$',
                'preferredGraphicsBackend:' + json.dumps(args.backend), options.read_text(), flags=re.MULTILINE)
            if count != 1:
                raise AssertionError('Private backend preference is missing or ambiguous')
            options.write_text(text)
        shutil.copy2(fixture, instance / 'mods/client-graphics-fixture.jar')
        command = [sys.executable, str(root / 'tools/runtime.py'), 'demo',
                   '--instance', str(instance), '--heap', '6G' if args.shaders else '3G']
        if platform.system() == 'Linux' and not args.native_display:
            command = ['xvfb-run', '-a', *command]
        try:
            run = run_fixture(command, instance, os.environ.copy())
        except FixtureTimeout as error:
            diagnostics = graphics_failure_summary(error.run, 0)
            diagnostics.update(fixture='client-graphics', profile=profile,
                completed=False, timed_out=True, timeout_seconds=240)
            print('HORIZON_FIXTURE_FAILURE ' + json.dumps(diagnostics), flush=True)
            raise RuntimeError('Graphics probe did not finish; diagnostics remain private') from None
        marker = 'HORIZON_CLIENT_GRAPHICS_RESULT '
        reports = [json.loads(line.split(marker, 1)[1]) for line in run.stdout.splitlines() if marker in line]
        trace_marker = 'HORIZON_GRAPHICS_BACKEND_ATTEMPT '
        attempts = [json.loads(line.split(trace_marker, 1)[1]) for line in run.stdout.splitlines() if trace_marker in line]
        if run.returncode or len(reports) != 1:
            diagnostics = graphics_failure_summary(run, len(reports))
            diagnostics['profile'] = profile
            diagnostics['preferred_backend'] = args.backend or build_pack.PROFILES[profile].get('backend', 'opengl')
            print('HORIZON_FIXTURE_FAILURE ' + json.dumps(diagnostics), flush=True)
            raise RuntimeError('Graphics probe failed; diagnostics remain private')
        report = reports[0]
        validate_shader_report(report, args.shaders)
        if report['observed_mode'] != ('world' if args.world else 'title') or report['ready_client_ticks'] < 40:
            raise AssertionError('Client never reached the requested graphics scene')
        if report['overlay_present'] is not False:
            raise AssertionError('Client graphics scene still has a loading overlay')
        if args.world:
            if report['world_loaded'] is not True or report['window_active'] is not True:
                raise AssertionError('Client never reached focused demo-world gameplay')
            if not list((instance / 'saves/Demo_World').rglob('*.dat')):
                raise AssertionError('Disposable world did not leave saved data')
        if report['sampled_pixels'] < 256 or report['distinct_sampled_rgb'] < 8:
            raise AssertionError('GPU readback was uniform')
        if hashlib.sha256((instance / 'mods' / mod.name).read_bytes()).hexdigest() != original_hash:
            raise AssertionError('Runtime original mod differs from the intended build')
        screenshot = instance / 'client-graphics-private.png'
        if not screenshot.is_file() or hashlib.sha256(screenshot.read_bytes()).hexdigest() != report['private_png_sha256']:
            raise AssertionError('Private screenshot evidence is missing or changed')
        report.update(profile=profile, preferred_backend=args.backend or build_pack.PROFILES[profile].get('backend', 'opengl'),
                      launcher_exit_code=run.returncode, backend_attempts=attempts)
        results.append(report)
        print(json.dumps(report), flush=True)
    output = root / 'dist/client-graphics-smoke.json'
    output.write_text(json.dumps({'schema': 1, 'platform': platform.platform(),
        'architecture': platform.machine(), 'minecraft': lab.VERSION,
        'original_mod_sha256': original_hash, 'results': results,
        'limits': ['Short scene startup and nonuniform framebuffer readback only.',
                  'World mode uses the ordinary limited demo; world files remain private.',
                  'No exhaustive gameplay, visual-correctness, FPS or M2 Pro performance claim.',
                  'Vulkan preference can fall back; inspect the observed backend.',
                  'PNG images, assets and full logs remain in the private runtime directory.']}, indent=2) + '\n')
    print('Client graphics startup/readback checks passed.', flush=True)


if __name__ == '__main__':
    main()
