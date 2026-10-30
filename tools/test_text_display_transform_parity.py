#!/usr/bin/env python3
"""Compare text-display matrices with the real Java client renderer (no window)."""
import json
import os
try:
    from . import lab, test_rust_parity as base, oracle_harness as harness, test_text_display_parity as metadata
except ImportError:
    import lab
    import test_rust_parity as base
    import oracle_harness as harness
    import test_text_display_parity as metadata


def main():
    metadata.main()
    build = lab.LOCAL / 'text-display-transform-parity'
    build.mkdir(exist_ok=True)
    (build / 'report.json').unlink(missing_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    cp = os.pathsep.join(map(str, [jars['client'], jars['server'], *libraries]))
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaTextDisplayTransformOracle.java'
    sources = [oracle, lab.ROOT/'crates/horizon-client/src/text_display.rs',
        lab.ROOT/'crates/horizon-client/src/game/text_displays.rs']
    tested = base.source_hashes(sources)
    source = lab.LOCAL / 'text-display-parity/java.jsonl'
    # Every billboard mode, coupled and independent entity/camera rotations.
    all_states = [json.loads(line) for line in source.read_text().splitlines()]
    cases = []
    for state in all_states[1:5]:
        for yaw in (-180, 0, 37):
            for pitch in (-45, 0, 75):
                for camera in ((0, 0), (97, -60), (-150, 42)):
                    cases.append(dict(state, angles=[yaw, pitch, *camera]))
    inputs = build / 'cases.jsonl'
    inputs.write_text(''.join(json.dumps(case) + '\n' for case in cases))
    harness.run_logged(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',lab.ROOT/'mods/horizon/src/test/java','-cp',cp,'-d',classes,oracle],build,'compile-java')
    harness.run_logged(['java','-Xmx1G','-cp',str(classes)+os.pathsep+cp,'dev.horizonlab.parity.VanillaTextDisplayTransformOracle',inputs,build/'java.jsonl'],build,'java')
    command = harness.build_probe(build,'text_display_fixture')
    harness.run_logged([*command,inputs,build/'rust.jsonl'],build,'rust')
    java = [json.loads(line) for line in (build/'java.jsonl').read_text().splitlines()]
    rust = [json.loads(line) for line in (build/'rust.jsonl').read_text().splitlines()]
    assert len(java) == len(rust) == len(cases)
    for i,(a,b) in enumerate(zip(java,rust)):
        # JOML and glam evaluate float matrix products in different orders.
        assert all(abs(x-y) <= 0.00002 for x,y in zip(a,b)), (i,a,b)
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(build/'report.json', {'pass':True,'cases':len(cases),'matrix_absolute_tolerance':0.00002,
        'sources_sha256':tested})
    print(f'Text-display transforms: {len(cases)} cases match the native client renderer')


if __name__ == '__main__':
    main()
