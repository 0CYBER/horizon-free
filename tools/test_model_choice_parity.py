#!/usr/bin/env python3
"""Compare weighted model choice and multipart RNG consumption with the client."""
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking, test_cuboid_model_parity as decoding
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking, test_cuboid_model_parity as decoding


def corpus(weights):
    cases=[]
    for vector in [None,*weights,(1,),(3,),(1,1),(1,2,7),(1073741824,1),(2147483646,1),(1,2147483646),tuple(range(1,100))]:
        cases.append((False,[vector],[0]));cases.append((True,[vector],[]));cases.append((True,[vector],[0]))
        cases.append((True,[vector,vector],[0,1]));cases.append((True,[None,vector,None],[0,1,2]))
    rng=random.Random(0x43484f494345)
    for _ in range(500):
        parts=[None if rng.randrange(3)==0 else [rng.randrange(1,1000) for _ in range(rng.randrange(1,20))] for _ in range(rng.randrange(1,20))]
        selected=[i for i in range(len(parts)) if rng.randrange(2)]
        cases.append((True,parts,selected))
    for i,(multipart,parts,selected) in enumerate(cases):
        for random_kind in ['legacy','xoroshiro']:
            for seed in [0,1,-1,-(1<<63),(1<<63)-1,rng.randrange(-(1<<63),1<<63)]:
                yield {'name':f'{i}/{random_kind}/{seed}','multipart':multipart,'parts':parts,'selected':selected,'random':random_kind,'seed':seed,'repeats':16}


def main():
    lock,jars,_=base.verify_artifacts();build=lab.LOCAL/'rust-model-choice-parity';build.mkdir(parents=True,exist_ok=True);report_path=build/'report.json';report_path.unlink(missing_ok=True)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaModelChoiceOracle.java';helper=oracle.with_name('VanillaCuboidModelOracle.java');crate=lab.ROOT/'crates/horizon-assets'
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_model_baking_parity.py',lab.ROOT/'tools/test_cuboid_model_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',crate/'Cargo.toml',lab.ROOT/'crates/horizon-core/Cargo.toml',*sorted(crate.rglob('*.rs')),*sorted((lab.ROOT/'crates/horizon-core/src').rglob('*.rs'))]
    tested=base.source_hashes(sources);weights=set()
    with zipfile.ZipFile(jars['client']) as archive:
        for name in archive.namelist():
            if not name.startswith('assets/minecraft/blockstates/') or not name.endswith('.json'):continue
            root=json.loads(archive.read(name));models=[*root.get('variants',{}).values(),*(s['apply'] for s in root.get('multipart',[]))]
            for model in models:
                if isinstance(model,list):weights.add(tuple(v.get('weight',1) for v in model))
    rows=list(corpus(sorted(weights)));inputs=build/'inputs.jsonl';inputs.write_text(''.join(json.dumps(row)+'\n' for row in rows));fixtures=base.source_hashes([inputs]);classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,[jars['client'],*baking.client_libraries()]));classpath=str(classes)+os.pathsep+dependencies
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',dependencies,'-d',classes,oracle,helper],'compile-java')
    run(['cargo','build','--locked','-p','horizon-assets','--example','model_choice_fixture'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT));binary=Path(metadata['target_directory'])/'debug/examples'/('model_choice_fixture.exe' if os.name=='nt' else 'model_choice_fixture')
    run([binary,inputs,build/'rust.jsonl'],'rust');run(['java','-Xmx512M','-cp',classpath,'dev.horizonlab.parity.VanillaModelChoiceOracle',inputs,build/'client.jsonl'],'client')
    decoding.canonicalize(build/'client.jsonl');decoding.canonicalize(build/'rust.jsonl');native=[json.loads(line) for line in (build/'client.jsonl').read_text().splitlines()];rust=[json.loads(line) for line in (build/'rust.jsonl').read_text().splitlines()]
    for left,right in zip(native,rust,strict=True):
        if left!=right:raise AssertionError('Model choice mismatch: '+left['name'])
    matched=base.compare(build/'client.jsonl',build/'rust.jsonl');base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged([inputs],fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'bundled_weight_vectors':len(weights),'fixtures':len(rows),'collections':sum(r['repeats'] for r in rows),'sources_sha256':tested,'fixtures_sha256':fixtures,'rust_output_sha256':lab.digest(build/'rust.jsonl','sha256'),'references':[{'side':'client','jar_sha256':lab.digest(jars['client'],'sha256'),'output_sha256':lab.digest(build/'client.jsonl','sha256'),'matched_output_lines':matched}],'scope':'Actual WeightedVariants.collectParts and MultiPartModel.collectParts with inert observable leaf models and preselected boolean conditions. Every distinct bundled blockstate weight vector plus synthetic singletons, large totals, empty/partial multipart subsets and random graphs. Legacy and xoroshiro streams, six seeds per graph, repeated collection and subsequent nextLong/nextInt verify model choice and draw consumption. No actual model baking, material evaluation, transformed quads, renderer integration or gameplay parity claim.'}
    lab.write_json(report_path,report);print(f'Native model choice: {matched:,} fixtures / {report["collections"]:,} collections match; {report_path}',flush=True)


if __name__=='__main__':main()
