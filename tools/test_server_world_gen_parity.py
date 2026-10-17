#!/usr/bin/env python3
"""A new world's generator settings from server.properties, against vanilla's dedicated server.

DedicatedServerProperties.WorldDimensionData makes a world that doesn't exist yet from `level-type` (a world
preset: `flat`, `minecraft:normal`, or a name it can't find, which it warns about and replaces with
`minecraft:normal`), a flat world's `generator-settings` (FlatLevelGeneratorSettings: its layers, biome,
lakes and features; a layer that doesn't parse is left out and logged, and settings without layers fall back
to the preset's classic ones) and `generate-structures`.

Each case starts vanilla's server and Horizon's (`horizon-client --dedicated --nogui`) in a new folder, waits
for `Done` and stops them. Compared exactly: the world's `data/minecraft/world_gen_settings.dat`, tag types
included, and the warnings and errors logged while the world's settings are read (the `Worker-Main` lines;
the worker's number is left out).

Horizon generates the normal and flat presets only: `large_biomes`, `amplified`, `single_biome_surface` and
`debug_all_block_states` aren't tested.
"""
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

try:
    from . import anvil_dump, lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port
except ImportError:
    import anvil_dump
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port

NAME = 'server-world-gen-parity'
PROPERTIES = {
    'online-mode': 'false',
    'view-distance': '2',
    'simulation-distance': '2',
    'spawn-protection': '0',
}
CASES = {
    # server.properties' default generator-settings ({}) has no layers: the preset's classic ones.
    'flat': {'level-type': 'flat', 'level-seed': '1'},
    'flat-layers': {
        'level-type': 'minecraft:flat',
        'level-seed': '-7',
        'generator-settings': json.dumps({
            'layers': [{'block': 'minecraft:bedrock', 'height': 1}, {'block': 'stone', 'height': 10},
                       {'block': 'minecraft:no_such_block', 'height': 2}, {'block': 'sand', 'height': 3}],
            'biome': 'minecraft:desert', 'lakes': True, 'features': True}, separators=(',', ':')),
    },
    'flat-biome': {
        'level-type': 'FLAT',
        'level-seed': 'flat',
        'generator-settings': '{"layers":[{"block":"minecraft:glass","height":5}],"biome":"nowhere"}',
    },
    'normal': {'level-type': 'minecraft:normal', 'level-seed': 'abc', 'generate-structures': 'false'},
    'unknown': {'level-type': 'Some Type', 'level-seed': '42'},
}
WORKER = re.compile(r'\[Worker-Main-\d+/(WARN|ERROR)\]: (.*)')


def escape(value):
    """java.util.Properties.store's escaping of a value."""
    return value.replace('\\', '\\\\').replace(':', '\\:').replace('=', '\\=').replace('#', '\\#')


def run_case(label, command, build, case):
    directory = build / label / case
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    props = dict(PROPERTIES, **CASES[case], **{'server-port': str(free_port())})
    (directory / 'server.properties').write_text(''.join(f'{k}={escape(v)}\n' for k, v in props.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    server = Server(command, directory, f'{label}-{case}')
    try:
        server.wait_for(r'Done \(')
    finally:
        server.stop()
    out = [f'# {case}']
    out += [f'{m.group(1)}: {m.group(2)}' for l in server.lines if (m := WORKER.search(l))]
    settings = anvil_dump.plain(anvil_dump.read_dat(directory / 'world/data/minecraft/world_gen_settings.dat'),
                                True, typed=True)
    out += json.dumps(settings, indent=1, sort_keys=True).splitlines()
    return out


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    subprocess.run(['cargo', 'build', '--release', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'release' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main',
                      'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    vanilla, horizon = [], []
    for case in CASES:
        vanilla += run_case('vanilla', vanilla_server, build, case)
        horizon += run_case('horizon', horizon_server, build, case)
    (build / 'vanilla.txt').write_text('\n'.join(vanilla) + '\n', encoding='utf-8')
    (build / 'horizon.txt').write_text('\n'.join(horizon) + '\n', encoding='utf-8')
    if vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=2, lineterm=''))
        raise AssertionError(f'world settings differ (see {build}/vanilla.txt and horizon.txt):\n' + diff[:6000])
    if not any('minecraft:flat' in l for l in vanilla) or not any(l.startswith('ERROR: ') for l in vanilla):
        raise AssertionError(f'vanilla made no flat world or logged no error (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        cases=list(CASES), lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='a new dedicated-server world: level-type (flat, normal, unknown), a flat world\'s '
              'generator-settings, generate-structures, as world_gen_settings.dat and the lines logged.'))
    print(f'server world gen: {len(CASES)} cases, {len(vanilla)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
