#!/usr/bin/env python3
"""Compare native item templates and strict scalar stack validation."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import random
import struct
import math
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    edges=[-2147483648,-1,0,1,2,64,99,2147483647]
    def op(kind,**kw):
        v=dict(kind=kind,a=0,b=1,count=1,item=1,patch=[],prototype=[]);v.update(kw);return v
    def row(count=1,patch=None,ops=None):
        return dict(prototypes=[[],[[0,64]],[[0,1],[1,10],[2,0]]],stacks=[dict(item=1,count=count,patch=patch or []),dict(item=2,count=1,patch=[]),dict(item=-1,count=0,patch=[])],ops=ops or [])
    for count in edges:
        for amount in edges:
            for item in range(3):
                for patch in [[],[[0,1],[1,0]],[[0,None]],[[0,64]],[[0,0]],[[1,10],[2,-1]]]:
                    yield row(count,patch,[op('new',item=item,count=count,patch=patch,b=0),op('with',count=amount),op('create',b=2),op('apply-count',count=amount,patch=[[0,99],[1,None]]),op('validate-patch',a=1,patch=[[0,1]])])
    for kind in ['from','from-nonempty','from-count']:
        for count in edges:
            for amount in edges:
                yield row(count,[[0,64],[1,None]],[op(kind,count=amount,b=0),op('prototype',prototype=[[0,2],[1,1]]),op('create',b=1),op('apply',b=2,patch=[[2,7]])])
    rng=random.Random(0x54454d504c415445)
    kinds=['new','from','from-nonempty','from-count','with','create','apply','apply-count','validate-patch','count','prototype']
    def entries():return [[rng.randrange(3),rng.choice([None,*edges])] for _ in range(rng.randrange(6))]
    for _ in range(2400):
        f=row(rng.choice(edges),entries())
        f['ops']=[op(rng.choice(kinds),a=rng.randrange(3),b=rng.randrange(3),count=rng.choice(edges),item=rng.randrange(3),patch=entries(),prototype=[[k,rng.choice(edges)] for k in rng.sample(range(3),rng.randrange(4))]) for _ in range(rng.randrange(10,26))]
        yield f


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-item-template-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaItemTemplateOracle.java'
    sources=[oracle,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/item_template_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaItemTemplateOracle',mode,path],side+'-'+mode)
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'item_template_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} item template output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual ItemStackTemplate constructors, fromStack/fromNonEmptyStack, withCount identity, current-prototype component queries, create/apply patch precedence, canonical EMPTY on failed strict validation, and ItemStack.applyComponentsAndValidate rollback. Real max-stack-size/max-damage/damage scalar components include redundant and removed patches and invalid counts. Contained component validation is a required Rust schema service and is not exercised by this scalar corpus. Component codecs, containers/bundle weight, use-remainder integration and complete gameplay integration remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped item template parity passed: {report_path}')


if __name__=='__main__':main()
