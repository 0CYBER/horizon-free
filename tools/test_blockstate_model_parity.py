#!/usr/bin/env python3
"""Compare blockstate resource decoding with the pinned client loader's codec."""
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


def corpus():
    model={'model':'block/stone'}
    def root(value): return {'variants':{'':value}}
    for value in [None,{},[],False,0,'x',{'variants':{}},{'multipart':[]},{'variants':None},{'variants':None,'multipart':[{'apply':model}]},root(model),root([model]),root([])]:yield json.dumps(value)
    values=[None,True,False,0,1,-1,90,90.9,-90,270,360,450,4294967296,4294967386,1e100,'90','true','',[],{},[model]]
    for key in ['model','x','y','z','uvlock','weight']:
        for value in values:
            variant=dict(model);variant[key]=value
            yield json.dumps(root(variant));yield json.dumps(root([variant]))
    for weights in [[2147483647],[2147483647,1],[2147483646,1],[0,1],[-1,2],[1,1],[1.9,2.9]]:
        yield json.dumps(root([dict(model,weight=w) for w in weights]))
    for name in ['block/stone',':block/stone',':','minecraft:','a:b:c','A:a','a:é','../a']:
        yield json.dumps(root({'model':name}))
    for x in [-360,-90,0,90,180,270,360,450]:
        for y in [0,90,180,270]:
            for z in [0,90,180,270]:yield json.dumps(root(dict(model,x=x,y=y,z=z,uvlock=True)))
    terms=[None,True,False,0,1,1.9,-1,4294967297,'true','false','!true','true|false','!true|false','!!true','!','', '|','true|','|true','a||b',' a ',[],{},['true']]
    conditions=[{},None,True,[],{'OR':[]},{'AND':[]},{'OR':[{'a':'b'}]},{'AND':[{'a':'b'},{'c':'d'}]},{'OR':'true'},{'OR':[],'AND':[]},{'OR':[],'a':'b'}]
    for key in ['a','OR','AND','']:
        conditions.extend({key:t} for t in terms)
    for condition in conditions:
        for apply in [model,[model],[],None]:yield json.dumps({'multipart':[{'when':condition,'apply':apply}]})
    for raw in ['{} trailing','{} {}','\ufeff'+json.dumps(root(model)), '{"variants":{"b":{"model":"b"},"a":{"model":"a"},"b":{"model":"c"}}}', '{"variants":{"":{"model":"a"}},"multipart":[{"apply":{"model":"b"}}]}']:
        yield raw
    rng=random.Random(0x424c4f434b)
    for _ in range(500):
        selectors=[]
        for _ in range(rng.randrange(1,8)):
            condition={rng.choice(['a','b','c']):rng.choice(['x','y','!x','x|y'])}
            if rng.randrange(2):condition={rng.choice(['OR','AND']):[condition,{'a':'x'}]}
            variants=[dict(model,weight=rng.randrange(1,10),x=rng.randrange(-4,5)*90,y=rng.randrange(4)*90,z=rng.randrange(4)*90,uvlock=bool(rng.randrange(2))) for _ in range(rng.randrange(1,6))]
            selectors.append({'when':condition,'apply':variants if rng.randrange(2) else variants[0]})
        yield json.dumps({'multipart':selectors})
    for number in ['1844674407370955161601','18446744073709551616.1','1e10000','1e9999','1' * 1024]:
        yield '{"variants":{"":{"model":"block/stone","x":'+number+'}}}'
        yield '{"variants":{"":{"model":"block/stone"}},"unknown":'+number+'}'


def main():
    lock,jars,libraries=base.verify_artifacts()
    build=lab.LOCAL/'rust-blockstate-model-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockStateModelOracle.java';helper=oracle.with_name('VanillaCuboidModelOracle.java');crate=lab.ROOT/'crates/horizon-assets'
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_cuboid_model_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',crate/'Cargo.toml',*sorted(crate.rglob('*.rs'))]
    tested=base.source_hashes(sources)
    with zipfile.ZipFile(jars['client']) as archive:
        names=sorted(n for n in archive.namelist() if n.startswith('assets/minecraft/blockstates/') and n.endswith('.json'))
        rows=[(n,archive.read(n).decode()) for n in names]
    synthetic=list(corpus());rows.extend((f'synthetic/{i}',raw) for i,raw in enumerate(synthetic))
    inputs=build/'inputs.jsonl';inputs.write_text(''.join(json.dumps(row)+'\n' for row in rows));fixtures=base.source_hashes([inputs]);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,[jars['client'],*libraries]))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',dependencies,'-d',classes,oracle,helper],'compile-java')
    run(['cargo','build','--locked','-p','horizon-assets','--example','blockstate_model_fixture'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'debug/examples'/('blockstate_model_fixture.exe' if os.name=='nt' else 'blockstate_model_fixture')
    run([binary,inputs,build/'rust.jsonl'],'rust');run(['java','-Xmx512M','-cp',str(classes)+os.pathsep+dependencies,'dev.horizonlab.parity.VanillaBlockStateModelOracle',inputs,build/'client.jsonl'],'client')
    rejected=decoding.canonicalize(build/'client.jsonl');decoding.canonicalize(build/'rust.jsonl')
    native=[json.loads(line) for line in (build/'client.jsonl').read_text().splitlines()];rust=[json.loads(line) for line in (build/'rust.jsonl').read_text().splitlines()]
    for left,right in zip(native,rust,strict=True):
        if left!=right:raise AssertionError('Blockstate model mismatch: '+left['name'])
    matched=base.compare(build/'client.jsonl',build/'rust.jsonl');assert all('error' not in row for row in native[:len(names)]),'Bundled blockstate rejected'
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged([inputs],fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'resource_blockstates':len(names),'synthetic_definitions':len(synthetic),'rejected':rejected,'sources_sha256':tested,'fixtures_sha256':fixtures,'rust_output_sha256':lab.digest(build/'rust.jsonl','sha256'),'references':[{'side':'client','jar_sha256':lab.digest(jars['client'],'sha256'),'output_sha256':lab.digest(build/'client.jsonl','sha256'),'matched_output_lines':matched}],'scope':'Actual StrictJsonParser and BlockStateModelDispatcher codec used by the pinned resource loader. Every bundled blockstate JSON plus variants, XYZ quarter turns, UV-lock flags, weighted entries, source order, multipart conditions and malformed synthetic cases. No registry binding, property selection, random draw selection, transformed baking, rendered pixels or gameplay parity claim.'}
    lab.write_json(report_path,report);print(f'Native blockstate models: {matched:,} definitions match ({rejected:,} rejected); {report_path}',flush=True)


if __name__=='__main__':main()
