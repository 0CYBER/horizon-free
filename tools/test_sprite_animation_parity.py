#!/usr/bin/env python3
"""Compare animation metadata, frame selection and tick state with the pinned client."""
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
import zipfile
try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking, test_cuboid_model_parity as decoding
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking, test_cuboid_model_parity as decoding


def corpus():
    def row(name, metadata, width=16, height=64, ticks=128):
        return dict(name=name,json=json.dumps(metadata),width=width,height=height,ticks=ticks)
    yield row('default', {})
    for key in ['width','height','frametime','interpolate','frames']:
        for i,value in enumerate([None,True,False,0,1,-1,1.9,-0.9,4294967297,2147483648,1e100,'1','true',{},[],[0,1], [None], [{'index':1,'time':2}]]):
            yield row(f'field-{key}-{i}',{key:value})
    for key in ['index','time']:
        for i,value in enumerate([None,True,False,0,1,-1,1.9,-0.9,4294967297,2147483648,1e100,'1',{},[],[1]]):
            yield row(f'frame-{key}-{i}',{'frames':[0,dict(index=1,**{key:value})]}) if key=='time' else row(f'frame-{key}-{i}',{'frames':[0,{'index':value}]})
    for i,frames in enumerate([[],[3],[99],[1,99],[2,2],[0,1,0],[3,2,1,0],[3,99,1,2],[{},0],[{'index':2,'time':None},0],[0,{'index':1,'time':2147483647}]]):
        for interpolate in [False,True]: yield row(f'sequence-{i}-{interpolate}',{'frames':frames,'interpolate':interpolate})
    for width,height in [(16,16),(16,48),(48,16),(32,48),(15,32),(1,4),(64,64)]:
        for w,h in [(None,None),(8,None),(None,8),(8,8),(16,32),(32,16),(1,1),(65,65)]:
            yield row(f'dim-{width}-{height}-{w}-{h}',{'width':w,'height':h},width,height,16)
    rng=random.Random(0x535052495445)
    for i in range(512):
        frames=[{'index':rng.randrange(7),'time':rng.choice([1,2,3,7,19,31,2147483647])} for _ in range(rng.randrange(12))]
        yield row(f'random-{i}',{'frames':frames,'frametime':rng.randrange(1,20),'interpolate':bool(rng.randrange(2))},ticks=512)
    for i,raw in enumerate(['null','[]','true','1','"x"','{"width":1e9999}','{"frametime":1e-9999}','{"width":4294967312.9}','{"frames":[0.9,1.9,2.9]}']):
        item=row(f'raw-{i}',{}); item['json']=raw;yield item

    numbers=['-0.0','4294967296','4294967297','4294967295','9223372036854775809',
             '-9223372036854775809','1e9999','1e10000','1e-9999','1e-10000',
             '1e2147483647','1e2147483648','1.0e10000','1.0e10001',
             '1'*1023,'1'*1024,'1'*10000,'1'*10001,
             *('1'+'0'*(n-2)+'1' for n in [20,64,65,66,100,1023,1024,10000,10001]),
             '1844674407370955161601','-1844674407370955161601',
             '18446744073709551616.1','18446744073709551616e-18']
    for i,number in enumerate(numbers):
        for key in ['width','frametime','interpolate']:
            item=row(f'number-{key}-{i}',{});item['json']='{"'+key+'":'+number+'}';yield item
        item=row(f'number-index-{i}',{});item['json']='{"frames":[0,'+number+']}';yield item


def main():
    lock,jars,_=base.verify_artifacts()
    build=lab.LOCAL/'rust-sprite-animation-parity';build.mkdir(parents=True,exist_ok=True)
    report_path=build/'report.json';report_path.unlink(missing_ok=True)
    oracle=lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSpriteAnimationOracle.java'
    helper=oracle.with_name('VanillaCuboidModelOracle.java');crate=lab.ROOT/'crates/horizon-assets'
    sources=[oracle,helper,Path(__file__),lab.ROOT/'tools/test_model_baking_parity.py',lab.ROOT/'tools/test_cuboid_model_parity.py',lab.ROOT/'tools/lab.py',lab.ROOT/'tools/test_rust_parity.py',lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',crate/'Cargo.toml',*sorted(crate.rglob('*.rs'))]
    tested=base.source_hashes(sources)
    rows=[]
    with zipfile.ZipFile(jars['client']) as archive:
        for name in sorted(archive.namelist()):
            if not name.startswith('assets/minecraft/textures/') or not name.endswith('.png.mcmeta'):continue
            metadata=json.loads(archive.read(name))
            if 'animation' not in metadata:continue
            width,height=struct.unpack('>II',archive.read(name.removesuffix('.mcmeta'))[16:24])
            rows.append(dict(name=name,json=json.dumps(metadata['animation']),width=width,height=height,ticks=512))
    resources=len(rows);rows.extend(corpus())
    inputs=build/'inputs.jsonl';inputs.write_text(''.join(json.dumps(row)+'\n' for row in rows));fixtures=base.source_hashes([inputs])
    classes=build/'classes';classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+'.log')).open('w') as log:subprocess.run(list(map(str,command)),cwd=lab.ROOT,check=True,timeout=3600,stdout=log,stderr=subprocess.STDOUT)
    dependencies=os.pathsep.join(map(str,[jars['client'],*baking.client_libraries()]))
    run(['javac','--release',lock['minecraft']['java_major'],'-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'),'-cp',dependencies,'-d',classes,oracle,helper],'compile-java')
    run(['cargo','build','--locked','-p','horizon-assets','--example','sprite_animation_fixture'],'compile-rust')
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--no-deps','--format-version','1'],cwd=lab.ROOT))
    binary=Path(metadata['target_directory'])/'debug/examples'/('sprite_animation_fixture.exe' if os.name=='nt' else 'sprite_animation_fixture')
    run([binary,inputs,build/'rust.jsonl'],'rust')
    run(['java','-Xmx512M','-cp',str(classes)+os.pathsep+dependencies,'dev.horizonlab.parity.VanillaSpriteAnimationOracle',inputs,build/'client.jsonl'],'client')
    rejected=decoding.canonicalize(build/'client.jsonl');decoding.canonicalize(build/'rust.jsonl')
    native=[json.loads(line) for line in (build/'client.jsonl').read_text().splitlines()]
    rust=[json.loads(line) for line in (build/'rust.jsonl').read_text().splitlines()]
    for left,right in zip(native,rust,strict=True):
        if left!=right: raise AssertionError('Sprite animation mismatch: '+left['name'])
    matched=base.compare(build/'client.jsonl',build/'rust.jsonl')
    assert all('error' not in row for row in native[:resources]), 'Bundled resource was rejected'
    states=sum(len(row['value']['animation']['samples'])+len(row['value']['animation']['edges']) for row in native if row.get('value',{}).get('animation'))
    base.ensure_sources_unchanged(sources,tested);base.ensure_sources_unchanged([inputs],fixtures)
    report={'schema':1,'minecraft':lock['minecraft']['version'],'manifest_sha1':lock['minecraft']['manifest_sha1'],'platform':platform.platform(),'java':subprocess.check_output(['java','--version'],text=True).splitlines()[0],'rustc':subprocess.check_output(['rustc','--version'],text=True).strip(),'resource_animations':resources,'native_tick_states':states,'fixtures':len(rows),'rejected':rejected,'sources_sha256':tested,'fixtures_sha256':fixtures,'rust_output_sha256':lab.digest(build/'rust.jsonl','sha256'),'references':[{'side':'client','jar_sha256':lab.digest(jars['client'],'sha256'),'output_sha256':lab.digest(build/'client.jsonl','sha256'),'matched_output_lines':matched}],'scope':'Actual AnimationMetadataSection JsonOps codec, frame-size calculation, SpriteContents sequence construction and AnimationState ticks. All bundled animated texture metadata plus synthetic numeric, null, invalid, repeated and out-of-image frame cases. Valid image grids only for SpriteContents construction. No resource-loader fallback, image decoding, GPU interpolation pixels, atlas packing or gameplay parity claim.'}
    lab.write_json(report_path,report);print(f'Native sprite animation: {matched:,} fixtures match ({rejected:,} rejected); {report_path}',flush=True)


if __name__=='__main__':main()
