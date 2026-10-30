#!/usr/bin/env python3
"""Compare effect modifier templates and native AttributeInstance integration."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    rng = random.Random(0x45464641545452)
    def bits(v): return struct.pack('>d', float(v)).hex()
    values = [-float('inf'), -1e30, -1, -0., 0., 1e-40, .1, 1, 2, 1e16, 1e30, float('inf'), float('nan')]
    amplifiers = [-2**31, -2**31 + 1, -2, -1, 0, 1, 2, 127, 255, 256, 2**31 - 2, 2**31 - 1]
    names = ['a', 'minecraft:a', ':a', '', 'minecraft:', ':', 'horizon:', 'horizon:/a', 'a:b.c-_/0']
    def template(name='horizon:a', amount=.1, operation=0):
        return dict(id=name, amount=bits(amount), operation=operation)
    def op(kind, **kw):
        row = dict(op=kind, key=0, template=0, target=0, amplifier=0, fail=-1, mutate=-1, mutations=[], read=False)
        row.update(kw); return row
    def hashes(mode, n):
        def unmix(x):
            h = ((x ^ (x >> 16)) * pow(0x9e3779b9, -1, 2**32)) & 0xffffffff
            return h if h < 0x80000000 else h - 2**32
        if mode == 0: return [0] * n
        if mode == 1: return [unmix(31)] * n
        if mode == 2: return [unmix(127 + i % 4) for i in range(n)]
        if mode == 3: return list(range(n))
        return [rng.randrange(-2**31, 2**31) for _ in range(n)]
    def fixture(h, templates, setup, ops, routes=None):
        return dict(hashes=h, templates=templates, defaults=[bits(1), bits(20), bits(-0.)],
                    routes=routes if routes is not None else [i % 3 for i in range(len(h) + 1)], setup=setup, ops=ops)
    for amount in values:
        for operation in range(3):
            for amp in amplifiers:
                yield fixture([0], [template(amount=amount, operation=operation)], [op('define')],
                              [op('create', amplifier=amp), op('add', amplifier=amp), op('value'),
                               op('add', amplifier=amp, read=True), op('value'), op('remove'), op('value')])
    pool = [template(name, values[i % len(values)], i % 3) for i, name in enumerate(names)]
    yield fixture([0] * len(pool), pool, [op('define', key=i, template=i) for i in range(len(pool))],
                  [op('create'), op('add'), op('add', amplifier=2), op('remove'), op('remove')])
    for mode in range(5):
        for n in [0, 1, 8, 24, 25, 49, 64]:
            h = hashes(mode, n)
            pool = [template('horizon:m' + str(i), values[i % len(values)], i % 3) for i in range(max(n, 1))]
            for null in [False, True]:
                setup = [op('define', key=i, template=i) for i in range(n)]
                if null: setup.append(op('define', key=-1))
                ops = [op('create'), op('add'), op('value', target=0), op('value', target=1), op('value', target=2),
                       op('add', amplifier=2, read=True), op('remove'), op('remove')]
                yield fixture(h, pool, setup, ops)
    # Native methods defer template reads until getInstance returns; add reads
    # the template again after removeModifier (including its dirty observer).
    for mode in range(5):
        h = hashes(mode, 64)
        pool = [template('horizon:m' + str(i % 5), values[i % len(values)], i % 3) for i in range(64)]
        for n in [1, 24, 25, 49]:
            setup = [op('define', key=i, template=i) for i in range(n)]
            mutations = [[op('define', key=i, template=(i + 1) % 64) for i in range(n)],
                         [op('null', key=i) for i in range(n)], [op('clear')],
                         [op('drop', key=i) for i in range(n - 1)],
                         [op('define', key=i, template=i) for i in range(n, 64)],
                         [op('route', key=i, target=-1) for i in range(n)],
                         [op('route', key=i, target=2) for i in range(n)],
                         [op('define', key=-1, template=7)]]
            for kind in ['add', 'remove', 'create']:
                for actions in mutations:
                    for at in [0, 1, 2, 3, 5]:
                        yield fixture(h, pool, setup, [op('add'), op(kind, amplifier=-1, mutate=at, mutations=actions, read=True),
                                                       op('create', amplifier=2**31 - 1), op('remove')])
    for null in [False, True]:
        key = -1 if null else 0
        for route in [-1, 0, 1]:
            yield fixture([0], [template()], [op('null', key=key), op('route', key=key, target=route)],
                          [op('add'), op('remove'), op('create')])
    for fail in range(12):
        for read in [False, True]:
            for kind in ['add', 'remove', 'create']:
                pool = [template(), template('horizon:b', .3, 1), template('horizon:c', -.7, 2)]
                yield fixture([0, 0, 0], pool, [op('define', key=i, template=i) for i in range(3)],
                              [op('add'), op(kind, amplifier=3, fail=fail, read=read), op('value'), op('add'), op('remove')])
    for _ in range(300):
        n = rng.choice([1, 8, 24, 25, 49])
        pool = [template(rng.choice(names + ['horizon:m' + str(i) for i in range(12)]), rng.choice(values), rng.randrange(3)) for __ in range(20)]
        h = hashes(rng.randrange(5), n)
        setup = [op('define', key=i, template=rng.randrange(len(pool))) for i in range(n)]
        def raw():
            return op(rng.choice(['define', 'define', 'null', 'drop', 'clear', 'route']), key=rng.randrange(-1, n),
                      template=rng.randrange(len(pool)), target=rng.randrange(-1, 3))
        ops = []
        for __ in range(30):
            if rng.randrange(5) == 0: ops.append(raw()); continue
            ops.append(op(rng.choice(['add', 'remove', 'create', 'seed', 'value']),
                          target=rng.randrange(3), template=rng.randrange(len(pool)), amplifier=rng.choice(amplifiers),
                          fail=rng.choice([-1, -1, -1, 0, 1, 2, 3]), mutate=rng.choice([-1, -1, 0, 1, 2, 3]),
                          read=bool(rng.randrange(2)), mutations=[raw() for ___ in range(rng.randrange(5))]))
        yield fixture(h, pool, setup, ops)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-effect-attribute-modifiers-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEffectAttributeModifiersOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/effect_attribute_modifiers_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaEffectAttributeModifiersOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'effect_attribute_modifiers_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} effect attribute modifier output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'fastutil_sha256':{str(p.relative_to(lab.ROOT)):lab.digest(p,'sha256') for p in libraries if 'fastutil' in str(p)},'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual MobEffect addAttributeModifier/createModifiers/addAttributeModifiers/removeAttributeModifiers with native fastutil definition map and AttributeInstance permanent modifier storage, cached values and dirty observers. Tests wrapping amplifier addition, double scaling, fresh modifier identity, map entry rereads after virtual getInstance/remove callbacks, null/missing instance skipping, map growth/shrink/replacement and partial failures. AttributeMap instance lookup and dirty receiver are controlled; callback mutation touches only definitions/routes. Registry effect definitions, pending attribute set ownership, entity notifications, full gameplay and playable platforms remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped effect attribute modifier parity passed: {report_path}')


if __name__=='__main__':main()
