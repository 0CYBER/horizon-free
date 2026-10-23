#!/usr/bin/env python3
"""The dedicated server's files against vanilla's own classes.

server.properties is read through DedicatedServerProperties (every key, its
default, parser and serializer, the legacy keys, the Java Properties syntax
and the UTF-8/ISO-8859-1 fallback) and stored back; eula.txt through Eula;
ops.json, whitelist.json, banned-players.json and banned-ips.json through
StoredUserList (Gson, Java HashMap order, ban dates, expiry, scripted adds and
removals). VanillaServerFilesOracle runs on the pinned server JAR, the
`server_files_probe` example on Horizon's port; the outputs must be identical.
Random defaults (the management secret, an empty level seed) and the stored
timestamps are placeholders on both sides."""
import os
from pathlib import Path
import platform
import subprocess

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
except ImportError:
    import lab
    import oracle_harness as oh
    import test_rust_parity as base

NAME = 'server-files-parity'
U1 = '069a79f4-44e9-4726-a5be-fca90e38aaf5'
U2 = '853c80ef-3c37-49fd-aa49-938b674adae6'
U3 = '61699b2e-d327-4a01-9f1e-0ea8c3f06bc6'
T0 = 1_760_000_000_000


def properties_cases():
    yield 'props-missing', None
    yield 'props-empty', b''
    yield 'props-vanilla-defaults', (
        b'#Minecraft server properties\n#Mon Oct 12 15:20:50 GMT 2026\n'
        b'level-type=minecraft\\:normal\nmotd=A Minecraft Server\nmanagement-server-secret=abc\n')
    yield 'props-numbers', '\n'.join([
        'difficulty=3', 'gamemode=1', 'server-port=+25570', 'max-players=-5', 'view-distance= 12',
        'simulation-distance=99999999999', 'max-tick-time=-1', 'spawn-protection=abc', 'rate-limit=',
        'max-world-size=0', 'op-permission-level=9', 'function-permission-level=-3',
        'network-compression-threshold=-1', 'entity-broadcast-range-percentage=5000',
        'player-idle-timeout=30', 'pause-when-empty-seconds=-1', 'query.port=25599', 'rcon.port=1',
        'text-filtering-version=2', 'management-server-port=8080']).encode()
    yield 'props-names', '\n'.join([
        'difficulty=hard', 'gamemode=spectator', 'online-mode=FALSE', 'white-list=yes',
        'enforce-whitelist=True', 'hardcore=tRuE', 'allow-flight=1', 'level-type=FLAT',
        'level-name=My World', 'motd=\\u00a7aHello \\u00e9 World', 'enable-status=false',
        'hide-online-players=true', 'enforce-secure-profile=false', 'accepts-transfers=true']).encode()
    yield 'props-bad-enums', b'difficulty=Hard\ngamemode=bogus\nop-permission-level=x\n'
    yield 'props-wrapping', b'difficulty=-1\ngamemode=7\nop-permission-level=2\n'
    yield 'props-syntax', (
        b'# comment\n  ! bang comment\n\n   key\\ with\\ space = value with space  \n'
        b'a:b\nc d\ne  =  f\nmulti=one\\\n    two\\\r\n  three\ncrlf=1\r\ncr=2\rnext=3\n'
        b'esc=\\t\\n\\r\\f\\x\\\\\ncolon\\:key=v\\=w\nempty\nlast=1\nlast=2\nmotd=dup\nmotd=final\n'
        b'trailing=back\\')
    yield 'props-latin1', b'motd=caf\xe9 \xff\nlevel-name=w\xf6rld\n'
    yield 'props-utf8', 'motd=日本語 ✓ ü\nlevel-name=wörld\nbug-report-link=https://example.com/ü\n'.encode()
    yield 'props-legacy', b'announce-player-achievements=false\nresource-pack=http://x/p.zip\nresource-pack-hash=0123456789abcdef0123456789abcdef01234567\n'
    yield 'props-pack', ('resource-pack=https://example.com/pack.zip\nresource-pack-sha1=0123456789ABCDEF0123456789ABCDEF01234567\n'
                         'resource-pack-id=1-2-3-4-5\nrequire-resource-pack=true\n'
                         'resource-pack-prompt={"text":"Please","color":"red"}\nresource-pack-hash=ffff\n').encode()
    yield 'props-pack-bad', b'resource-pack=https://example.com/a.zip\nresource-pack-id=not-a-uuid\n'
    yield 'props-pack-noid', b'resource-pack=https://example.com/b.zip\nresource-pack-sha1=bad\nresource-pack-prompt=[1,\n'
    yield 'props-pack-prompt-string', b'resource-pack=x\nresource-pack-prompt="hi"\n'
    yield 'props-seeds-number', b'level-seed=-12345\n'
    yield 'props-seeds-text', b'level-seed=  Minecraft  \n'
    yield 'props-seeds-blank', b'level-seed=   \n'
    yield 'props-generator', b'level-type=minecraft:flat\ngenerator-settings= { "layers" : [ {"block":"stone","height":1.50} ], "biome": "minecraft:plains", "s":"a<b\\u00e9" }\n'
    yield 'props-generator-bad', b'generator-settings={layers}\n'
    yield 'props-generator-array', b'generator-settings=[]\n'
    yield 'props-broadcast-bad', b'entity-broadcast-range-percentage=lots\n'
    yield 'props-packs', b'initial-enabled-packs=vanilla, minecart_improvements ,,\ninitial-disabled-packs= trade_rebalance\n'
    yield 'props-custom-keys', b'zzz=1\n#c\nAAA=2\n\xc3\xa9=3\n~=4\n'
    yield 'props-spaces', b'motd=  leading\nserver-ip = 127.0.0.1\nrcon.password=p#s!w=d:\n'


def eula_cases():
    yield 'eula-missing', None
    yield 'eula-true', b'eula=true\n'
    yield 'eula-TRUE', b'#comment\neula = TRUE\n'
    yield 'eula-false', b'eula=false\n'
    yield 'eula-garbage', b'eula=yes\n'
    yield 'eula-malformed', b'eula=\\uzzzz\n'


def list_cases():
    ops = ('[{"uuid":"%s","name":"Notch","level":4,"bypassesPlayerLimit":true},'
           '{"uuid":"%s","name":"jeb_","level":9},{"uuid":"bad","name":"x"},{"name":"nouuid"},'
           '{"uuid":"%s","name":"Dinnerbone","level":"2","bypassesPlayerLimit":"true"}]') % (U1, U2, U3)
    yield 'lists-load', {'ops.json': ops,
                         'whitelist.json': '[{"uuid":"%s","name":"Notch"},{"uuid":"%s","name":"<b>&\'=x"}]' % (U1, U2),
                         'banned-players.json': '[{"uuid":"%s","name":"griefer","created":"2025-01-02 03:04:05 +0100",'
                                                '"source":"Server","expires":"forever","reason":"Banned by an operator."},'
                                                '{"uuid":"%s","name":"temp","created":"2025-13-40 25:61:61 -0530","expires":"2099-01-01 00:00:00 +0000"}]' % (U2, U3),
                         'banned-ips.json': '[{"ip":"10.0.0.1","created":"2025-06-07 08:09:10 +0000","source":"Console","expires":"forever","reason":"spam"},'
                                            '{"ip":"192.168.1.2","created":"2025-06-07 08:09:10 +0000"}]'}
    yield 'lists-empty-files', {'ops.json': '', 'whitelist.json': '[]', 'banned-players.json': '  \n',
                                'banned-ips.json': '[]'}
    yield 'lists-bad-json', {'ops.json': '{"not":"an array"}', 'whitelist.json': '[1,2]'}
    names = ['Alex', 'Steve', 'Notch', 'jeb_', 'Dinnerbone', 'Grumm', 'Ä', 'player_%d']
    actions = []
    import uuid as _uuid
    for i in range(40):
        name = names[i % len(names)].replace('%d', str(i))
        u = str(_uuid.UUID(int=(i * 0x9E3779B97F4A7C15 * 0x1F) % (1 << 128), version=4))
        actions.append('\t'.join(['op', name, u, str(i % 6), 'true' if i % 3 == 0 else 'false']))
        actions.append('\t'.join(['white', name, u]))
        actions.append('\t'.join(['ban', name, u, str(T0 + i * 1000), '-' if i % 4 == 0 else 'Server',
                                  '-' if i % 2 else str(T0 + 10**12), '-' if i % 5 == 0 else 'reason <%d> & \'x\'' % i]))
        actions.append('\t'.join(['banip', '10.0.%d.%d' % (i // 7, i), str(T0 + i), 'Console', '-', 'flood']))
        if i % 7 == 3:
            actions.append('\t'.join(['deop', name, u]))
            actions.append('\t'.join(['unwhite', name, u]))
            actions.append('\t'.join(['pardon', name, u]))
            actions.append('\t'.join(['pardonip', '10.0.%d.%d' % (i // 7, i)]))
        if i % 9 == 0:
            actions.append('\t'.join(['op', name, u, str(i % 6), 'true' if i % 3 == 0 else 'false']))
            actions.append('\t'.join(['isbanned', name, u]))
            actions.append('\t'.join(['isop', name, u]))
    actions.append('\t'.join(['ban', 'expired', U1, str(T0), 'X', str(T0 + 1), '-']))
    actions.append('\t'.join(['isbanned', 'expired', U1]))
    actions.append('\t'.join(['op', 'expiredop', U1, '4', 'false']))
    yield 'lists-actions', {'actions.txt': '\n'.join(actions) + '\n'}


def make_cases(build):
    root = build / 'cases'
    if root.exists():
        import shutil
        shutil.rmtree(root)
    root.mkdir(parents=True)
    count = 0
    for name, data in properties_cases():
        # Numbered: macOS file systems ignore case, and eula-true and
        # eula-TRUE must not share a folder.
        d = root / f'{count:04d}-{name}'
        d.mkdir()
        if data is None:
            (d / 'properties.default').write_bytes(b'')
        else:
            (d / 'server.properties').write_bytes(data)
        count += 1
    for name, data in eula_cases():
        # Numbered: macOS file systems ignore case, and eula-true and
        # eula-TRUE must not share a folder.
        d = root / f'{count:04d}-{name}'
        d.mkdir()
        if data is None:
            (d / 'eula.missing').write_bytes(b'')
        else:
            (d / 'eula.txt').write_bytes(data)
        count += 1
    for name, files in list_cases():
        # Numbered: macOS file systems ignore case, and eula-true and
        # eula-TRUE must not share a folder.
        d = root / f'{count:04d}-{name}'
        d.mkdir()
        for f, text in files.items():
            (d / f).write_text(text, encoding='utf-8')
        count += 1
    return root, count


def explain(build):
    """The first differing lines of each differing case."""
    def sections(path):
        out, name = {}, None
        for line in path.read_text(encoding='utf-8', errors='replace').splitlines():
            if line.startswith('== '):
                name = line[3:]
                out[name] = []
            elif name is not None:
                out[name].append(line)
        return out
    left, right = sections(build / 'server.txt'), sections(build / 'rust.txt')
    text = ''
    for name in left:
        a, b = left[name], right.get(name, [])
        if a != b:
            diffs = [(x, y) for x, y in zip(a, b) if x != y][:4]
            if len(a) != len(b):
                diffs.append((f'<{len(a)} lines>', f'<{len(b)} lines>'))
            text += f'\n  {name}: (vanilla, horizon) {diffs}'
    return text


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaServerFilesOracle.java'
    sources = [Path(__file__).resolve(), oracle, lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/lab.py',
               lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-client/examples/probe/server_files_probe.rs',
               *sorted((lab.ROOT / 'crates/horizon-client/src/server').glob('*.rs'))]
    tested = base.source_hashes(sources)
    cases, count = make_cases(build)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    oh.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath',
                   lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                  build, 'compile-java')
    command = oh.build_probe(build, 'server_files_probe')
    oh.run_logged([*command, cases, build / 'rust.txt'], build, 'rust')
    oh.run_logged(['java', '-Xmx1g', '-Duser.timezone=UTC', '-Djava.awt.headless=true', '-cp',
                   str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaServerFilesOracle',
                   cases, build / 'server.txt'], build, 'server')
    try:
        matched = base.compare(build / 'server.txt', build / 'rust.txt')
    except AssertionError as error:
        raise AssertionError(str(error) + explain(build)) from None
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], manifest_sha1=lock['minecraft']['manifest_sha1'],
        platform=platform.platform(), rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0], cases=count,
        sources_sha256=tested, server_jar_sha256=lab.digest(jars['server'], 'sha256'),
        matched_output_lines=matched, output_sha256=lab.digest(build / 'rust.txt', 'sha256'),
        scope='DedicatedServerProperties (all keys, defaults, parsers, serializers, legacy keys, Java Properties '
              'syntax and encoding fallback, stored file), Eula, and the four StoredUserList files (load, '
              'add/remove, expiry, save order and Gson output).'))
    print(f'{count} server file cases match the pinned server ({matched:,} output lines): {report}', flush=True)


if __name__ == '__main__':
    main()
