#!/usr/bin/env python3
"""Compare all pinned replacement-rule codecs and ordered evaluation."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
try:
    from . import lab, test_rust_parity as base, test_worldgen_parity as worldgen
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen


def corpus(facts, tags):
    defaults=list(facts['defaults'].values())
    all_states=list(range(len(facts['states'])))
    heights=[-2147483648,-2032,-65,-64,-1,0,1,63,64,319,2031,2147483647]
    def cases(definition, states=defaults, repeats=1):
        for legacy in [False,True]:
            yield {'definition':json.dumps(definition,separators=(',',':')),'states':states,'heights':heights,'legacy':legacy,'seed':-1 if legacy else 0x123456789abcdef,'repeats':repeats}
    yes={'predicate_type':'always_true'}
    no={'predicate_type':'not','rule':yes}
    for value in [yes,no,{'predicate_type':'all_of','rules':[]},{'predicate_type':'any_of','rules':[]}]:yield from cases(value,all_states)
    for name in sorted(tags):yield from cases({'predicate_type':'tag_match','tag':name})
    for name in ['minecraft:logs','minecraft:leaves','minecraft:stone_ore_replaceables','horizon:missing']:
        yield from cases({'predicate_type':'tag_match','tag':name},all_states)
    for name, state in facts['defaults'].items():
        yield from cases({'predicate_type':'block_match','block':name},[0,state,(state+1)%len(all_states),all_states[-1]])
    for i,state in enumerate(facts['states']):
        yield from cases({'predicate_type':'blockstate_match','block_state':state},[i,(i+1)%len(all_states)])
    for low,high in [(-2147483648,2147483647),(-64,319),(64,63),(0,0),(4294967297,-4294967295),(0.9,1.9)]:
        yield from cases({'predicate_type':'height_match','min_inclusive':low,'max_inclusive':high},defaults[:24],2)
    for kind,field in [('random_block_match','block'),('random_blockstate_match','block_state')]:
        for block in ['air','water','oak_log',{'id':'water','properties':{'level':'+01'}}]:
            for probability in [-1,-0.0,0,1e-50,0.25,0.5,1,2,1e100,None,False,'0.5']:
                rule={'predicate_type':kind,field:block,'probability':probability}
                yield from cases(rule,all_states[0:160],2)
                for wrapper in [{'predicate_type':'all_of','rules':[rule,yes,rule]}, {'predicate_type':'any_of','rules':[rule,no,rule]}, {'predicate_type':'all_of','rules':[no,rule]}, {'predicate_type':'any_of','rules':[yes,rule]}, {'predicate_type':'not','rule':rule}]:
                    yield from cases(wrapper,all_states[0:160],2)
    invalid=[None,False,0,[],{}, {'predicate_type':'unknown'}, {'predicate_type':False}]
    for kind,field in [('block_match','block'),('blockstate_match','block_state'),('tag_match','tag'),('not','rule'),('all_of','rules'),('any_of','rules')]:
        invalid += [{'predicate_type':kind,field:value} for value in [None,False,0,[],{},'missing','Minecraft:stone',':']]
        invalid.append({'predicate_type':kind})
    for value in invalid:yield from cases(value,defaults[:4])


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--prepare',action='store_true');args=parser.parse_args()
    build=lab.LOCAL/'rust-rule-test-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaRuleTestOracle.java'
    helpers=[oracle.with_name('VanillaBlockStateCodecOracle.java'),oracle.with_name('VanillaHeightmapOracle.java')]
    tags=worldgen.tag_resources(jars['server'])
    if not tags or tags!=worldgen.tag_resources(jars['client']):raise ValueError('client/server tags differ')
    (build/'tags.json').write_text(json.dumps(tags,separators=(',',':')),encoding='utf-8')
    sources=[oracle,*helpers,lab.ROOT/'tools/test_worldgen_parity.py',Path(__file__),lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-core/Cargo.toml',lab.ROOT/'crates/horizon-world/Cargo.toml',*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-world/src').rglob('*.rs')),lab.ROOT/'crates/horizon-world/examples/probe/rule_test_fixture.rs']
    tested_sources=base.source_hashes(sources)
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w',encoding='utf-8') as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',classes,oracle,*helpers],'compile-java')
    def java(side,mode,path):
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jars[side]),dependency_path]),'dev.horizonlab.parity.VanillaRuleTestOracle',mode,path],side+'-'+mode)
    for side in jars:java(side,'export',build/f'facts-{side}.json')
    facts=json.loads((build/'facts-server.json').read_text())
    if facts!=json.loads((build/'facts-client.json').read_text()):raise ValueError('client/server registry metadata differs')
    if len(facts['states'])!=35723:raise ValueError('incomplete pinned state registry')
    fixtures=list(corpus(facts,tags));inputs=build/'inputs.txt';inputs.write_text(''.join(json.dumps(value,separators=(',',':'))+'\n' for value in fixtures),encoding='utf-8')
    fixture_paths=[inputs,build/'tags.json',build/'facts-client.json',build/'facts-server.json'];tested_fixtures=base.source_hashes(fixture_paths)
    run(['cargo','build','--release','-p','horizon-world','--example','world_probe'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples'/('world_probe.exe' if os.name=='nt' else 'world_probe')
    run([binary,'rule_test_fixture',build],'rust');references=[]
    for side,jar in jars.items():
        java(side,side,build);reference=build/f'{side}.txt';matched=base.compare(reference,build/'rust.txt')
        references.append({'side':side,'jar_sha256':lab.digest(jar,'sha256'),'output_sha256':lab.digest(reference,'sha256'),'matched_output_lines':matched})
        print(f'{side}: {matched:,} replacement-rule output lines match Rust',flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'block_states':len(facts['states']),'blocks':len(facts['defaults']),'integer_properties':len(facts['integers']),'codec_inputs':len(fixtures),'rule_types':10,'block_tag_files':len(tags),'requested_tests':sum(len(f['states'])*f['repeats'] for f in fixtures),'sources_sha256':tested_sources,'fixtures_sha256':tested_fixtures,'rust_output_sha256':lab.digest(build/'rust.txt','sha256'),'references':references,'scope':'Actual RuleTest.CODEC and all ten registered replacement-rule types. Every block and block-state identity, all bundled block tags, absent tags, inclusive and reversed height ranges, unrestricted random probabilities, logical short-circuit evaluation, malformed inputs, both random modes and subsequent draws are compared. Registry metadata and tag resources independently agree across authenticated JARs. Bound tags are immutable snapshots. Replacement feature mutations, structure processors, reload ownership and complete gameplay remain outside this standalone comparison.'}
    lab.write_json(report_path,report);print(f'Scoped replacement-rule parity passed: {report_path}')


if __name__=='__main__':main()
