#!/usr/bin/env python3
"""Compare blockstate selector binding and multipart parts over the native registry."""
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_cuboid_model_parity as decoding
except ImportError:
    import lab, test_rust_parity as base, test_cuboid_model_parity as decoding


def corpus(facts):
    def variant(i):return {'model':f'horizon:probe_{i}'}
    names=['stone','water','oak_stairs','oak_fence','oak_log','oak_door','redstone_wire','item_frame','glow_item_frame']
    for name in names:
        block='minecraft:'+name;schema=facts[block]['schema']
        def row(value):return {'block':block,'json':json.dumps(value)}
        keys=['',',','=','==','=x','unknown','unknown=x']
        for key,domain in schema.items():
            values=[str(domain['integer'][0]),str(domain['integer'][1])] if 'integer' in domain else domain['named']
            for value in values:
                selector=key+'='+value;keys.extend([selector,selector+',',','+selector,selector+','+selector])
            keys.extend([key,key+'=',key+'=unknown',key+'=true=false',key+'= '+values[0],key+'='+values[0]+','+key+'='+values[-1]])
        for key in keys:
            yield row({'variants':{key:variant(0)}})
            yield row({'variants':{'':variant(0),key:variant(1)},'multipart':[{'apply':variant(2)}]})
            yield row({'variants':{key:variant(0),'':variant(1)}})
        conditions=[None,{'OR':[]},{'AND':[]},{'unknown':'x'},{'OR':[{'AND':[]},{'unknown':'x'}]},{'AND':[{'OR':[]},{'unknown':'x'}]}]
        for key,domain in schema.items():
            values=[str(domain['integer'][0]),str(domain['integer'][1])] if 'integer' in domain else domain['named']
            terms=['unknown',values[0],'!'+values[0],values[0]+'|'+values[-1],'!'+values[0]+'|'+values[-1],values[0]+'|unknown',values[0]+'|!'+values[0]+'|unknown']
            if 'integer' in domain:terms+=['+0','-0','00','+01','-01','1.0','1e0','2147483648','-2147483649','9'*80,'0'*80+'1','\u0661','\uff11','\U0001d7d9']
            for term in terms:
                conditions.append({key:term})
                yield row({'variants':{key+'='+term:variant(0)}})
        for condition in conditions:
            parts=[{'when':condition,'apply':variant(0)},{'apply':variant(1)}]
            yield row({'multipart':parts})
            yield row({'variants':{'':variant(2)},'multipart':parts})
        rng=random.Random(name)
        valid=[k for k in keys if k and 'unknown' not in k]
        for _ in range(60):
            chosen=rng.sample(valid,min(len(valid),rng.randrange(1,8)))
            yield row({'variants':{k:variant(i) for i,k in enumerate(chosen)},'multipart':[{'apply':variant(9)}]})


def main():
    lock,jars,libraries=base.verify_artifacts();build=lab.LOCAL/'rust-blockstate-binding-parity';build.mkdir(parents=True,exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockStateBindingOracle.java';helper=oracle.with_name('VanillaCuboidModelOracle.java');crate=lab.ROOT/'crates/horizon-assets'
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_cuboid_model_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',crate/'Cargo.toml',lab.ROOT/'crates/horizon-core/Cargo.toml',*sorted(crate.rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs'))]
    tested=base.source_hashes(sources);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,[jars['client'],*libraries]));classpath=str(classes)+os.pathsep+dependencies
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',dependencies,'-d',classes,oracle,helper],'compile-java')
    facts_path=build/'registry.json';run(['java','-Xmx512M','-cp',classpath,'dev.horizonlab.parity.VanillaBlockStateBindingOracle','export',facts_path],'export');facts=json.loads(facts_path.read_text())
    rows=[];unbound=[]
    with zipfile.ZipFile(jars['client']) as archive:
        names=sorted(n for n in archive.namelist() if n.startswith('assets/minecraft/blockstates/') and n.endswith('.json'))
        for name in names:
            block='minecraft:'+name.removeprefix('assets/minecraft/blockstates/').removesuffix('.json')
            if block not in facts:unbound.append(name);continue
            rows.append({'name':name,'block':block,'json':archive.read(name).decode()})
    resources=len(rows)
    assert not unbound, 'Bundled blockstate lacks native state definition'
    assert len(facts)==1288 and sum(len(v['states']) for v in facts.values())==35727, 'Incomplete state definitions'
    for i,row in enumerate(corpus(facts)):rows.append(dict(name=f'synthetic/{i}',**row))
    inputs=build/'inputs.jsonl';inputs.write_text(''.join(json.dumps(row)+'\n' for row in rows));fixtures=base.source_hashes([inputs,facts_path])
    run(['cargo','build','--locked','-p','horizon-assets','--example','blockstate_binding_fixture'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'debug/examples'/('blockstate_binding_fixture.exe' if os.name=='nt' else 'blockstate_binding_fixture')
    run([binary,inputs,build/'rust.jsonl',facts_path],'rust');run(['java','-Xmx512M','-cp',classpath,'dev.horizonlab.parity.VanillaBlockStateBindingOracle',inputs,build/'client.jsonl'],'client')
    rejected=decoding.canonicalize(build/'client.jsonl');decoding.canonicalize(build/'rust.jsonl');native=[json.loads(line) for line in (build/'client.jsonl').read_text().splitlines()];rust=[json.loads(line) for line in (build/'rust.jsonl').read_text().splitlines()]
    for left,right in zip(native,rust,strict=True):
        if left!=right:raise AssertionError('Blockstate binding mismatch: '+left['name'])
    matched=base.compare(build/'client.jsonl',build/'rust.jsonl');assert all('error' not in row for row in native[:resources]),'Bundled blockstate rejected'
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged([inputs,facts_path],fixtures)
    states=sum(len(row.get('selected',[])) for row in native)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'resource_blockstates':resources,'unbound_resource_names':unbound,'state_definitions':len(facts),'definition_states':sum(len(v['states']) for v in facts.values()),'definitions':len(rows),'selected_state_results':states,'rejected':rejected,'sources_sha256':tested,'fixtures_sha256':fixtures,'rust_output_sha256':lab.digest(build/'rust.jsonl','sha256'),'references':[{'side':'client','jar_sha256':lab.digest(jars['client'],'sha256'),'output_sha256':lab.digest(build/'client.jsonl','sha256'),'matched_output_lines':matched}],'scope':'Actual BlockStateModelDispatcher.instantiate and MultiPartModel visual-equality selection on every bundled blockstate definition, including the native fake item-frame definitions, and its native ordered possible states. Synthetic malformed selectors, overlapping recovery, eager multipart errors, per-term negation, combined predicates, integer-property conversion and partial coverage. Property schema and state order are exported from the authenticated client. No weighted random selection, transformed baking, renderer integration or gameplay parity claim.'}
    lab.write_json(report_path,report);print(f'Native blockstate binding: {matched:,} definitions / {states:,} state results match ({rejected:,} rejected); {report_path}',flush=True)


if __name__=='__main__':main()
