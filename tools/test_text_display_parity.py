#!/usr/bin/env python3
"""Decode text-display metadata produced by vanilla's actual entity accessors/codecs."""
import json
import os
from pathlib import Path
try:
    from . import lab, test_rust_parity as base, oracle_harness as harness
except ImportError:
    import lab
    import test_rust_parity as base
    import oracle_harness as harness


def main():
    build = lab.LOCAL / 'text-display-parity'
    build.mkdir(parents=True, exist_ok=True)
    (build / 'report.json').unlink(missing_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    cases = [{}] + [dict(flags=flags, opacity=opacity, billboard=billboard,
        text={'text': 'Lobby 日本語', 'bold': bool(flags & 1)},
        translation=[1.25, -2.5, 0.0], scale=[0.5, 2.0, 1.0],
        left_rotation=[0, 0, 0, 1], right_rotation=[0, 1, 0, 0],
        line_width=80, background=-2147418368, brightness=15728880,
        interpolation_delay=-1, interpolation_duration=12, teleport_duration=4,
        view_range=2.0, shadow_radius=0.5, shadow_strength=0.75, width=3.0, height=2.0, glow_color=16711680)
        for flags in range(32) for opacity in (-128, -1, 0, 127) for billboard in range(4)]
    fixtures = build / 'cases.jsonl'
    fixtures.write_text(''.join(json.dumps(c, ensure_ascii=False) + '\n' for c in cases))
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    source = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaTextDisplayOracle.java'
    sources = [Path(__file__).resolve(), source,
        lab.ROOT / 'crates/horizon-client/src/text_display.rs',
        lab.ROOT / 'crates/horizon-protocol/src/entity_data.rs', lab.ROOT / 'Cargo.lock']
    tested = base.source_hashes(sources)
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    run = lambda command, name: harness.run_logged(command, build, name)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath',
         lab.ROOT / 'mods/horizon/src/test/java', '-cp', cp, '-d', classes, source], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + cp,
         'dev.horizonlab.parity.VanillaTextDisplayOracle', fixtures, build / 'java.jsonl'], 'java')
    command = harness.build_probe(build, 'text_display_fixture')
    run([*command, build / 'java.jsonl', build / 'rust.jsonl'], 'rust')
    expected = [json.loads(line)['expected'] for line in (build / 'java.jsonl').read_text().splitlines()]
    actual = [json.loads(line) for line in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(expected) == len(actual) == len(cases)
    for index, (a, b) in enumerate(zip(expected, actual)):
        assert a == b, (index, a, b)
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(build / 'report.json', {'minecraft': lab.VERSION, 'cases': len(cases), 'pass': True,
        'sources_sha256': tested, 'java_output_sha256': lab.digest(build / 'java.jsonl', 'sha256')})
    print(f'Text display metadata parity: {len(cases)} cases match vanilla')


if __name__ == '__main__':
    main()
