#!/usr/bin/env python3
"""Compare integrated contextual block collision selection with pinned vanilla."""
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
    def bits(value):return struct.pack('>d',value).hex()
    modes=['empty','position','player','marker','falling','placement','placement_null','with_player','with_marker','with_falling','with_null']
    positions=[[0,0,0],[17,-64,-33],[2147483647,2147483647,-2147483648]]
    for state in range(35723):
        for i,mode in enumerate(modes):
            for always in [False,True]:
                p=positions[(state+i)%len(positions)]
                y=p[1]+[0.5,1.0,1.5][(state+i)%3]
                yield {'mode':mode,'y':bits(y),'other':bits(y),'descending':bool(i%2),'always':always,'item':2,'count':1,'stand':True,'fail':False,'registry':[state,*p]}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-contextual-collision-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionContextOracle.java'
    helpers=[oracle.with_name(name+'.java') for name in ['VanillaFaceSupportOracle','VanillaCollisionOracle','VanillaContextualCollisionAudit']]
    runner=lab.ROOT/'tools/test_collision_context_parity.py'
    sources=[oracle,*helpers,runner,Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/collision_context_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaCollisionContextOracle',mode,path],side+'-'+mode)
    run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars['server']),dependency_path]),'dev.horizonlab.parity.VanillaCollisionOracle','generate',build],'export')
    fixtures=list(corpus());inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'block-registry.nbt',build/'cached-collision.nbt'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'collision_context_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaContextualCollisionAudit',side,build],side+'-audit')
        audited=base.compare(build/(side+'-audit.txt'),build/'rust-audit.txt')
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched,'audited_states':audited,'audit_sha256':lab.digest(build/(side+'-audit.txt'),'sha256')})
        print(f'{side}: {matched:,} contextual-collision output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'fixtures':len(fixtures),'block_states':35723,'independent_cached_states':35492,'context_sensitive_cached_states':32,'uncached_states':199,'audit_positions_per_cached_state':7,'audit_context_types':2,'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Integrated contextual dispatcher across every pinned block state, eleven context variants and both always-fluid flag inputs. The native audit verifies 35,492 cached states return the identical cached shape without world/context queries at seven positions with interface and entity context providers that reject queries; liquid states query context instead. Integrated outputs compare cache-relative shape equality or full uncached geometry and ordered entity/world callbacks. Missing block entities are used here; active shulker/piston states retain separate coverage. Experimental minecart contexts, entity lifecycle and live movement remain pending.'}
    lab.write_json(report_path,report);print(f'Scoped contextual collision parity passed: {report_path}')


if __name__=='__main__':main()
