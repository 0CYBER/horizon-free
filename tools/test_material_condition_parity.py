#!/usr/bin/env python3
"""Compare numeric material conditions and cache transitions with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import zipfile
try:
    from . import lab,test_rust_parity as base,test_worldgen_parity as worldgen,test_biome_temperature_parity as biome
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_biome_temperature_parity as biome


def corpus():
    rng=random.Random(0x484f52495a4f4e434f4e444954494f4e)
    edges=[-2**31,-2032,-64,-21,-20,-5,-4,-3,-1,0,1,3,4,5,20,21,63,2031,2**31-1]
    conditions=[["hole"],["steep"]]
    for multiplier in [-2**31,-20,-1,0,1,20,2**31-1]:
        for add in [False,True]:
            for kind in range(4):conditions.append(["y",[kind,[-64,0,63,2**31-1][kind]],multiplier,add])
            conditions.append(["water",edges[(multiplier+3)%len(edges)],multiplier,add])
    for secondary in [-2**31,-100,-1,0,1,2,12,2**31-1]:
        for add in [False,True]:
            for ceiling in [False,True]:conditions.append(["stone",edges[(secondary+7)%len(edges)],add,secondary,ceiling])
    for name in ["bedrock_floor","minecraft:bedrock_floor",":bedrock_roof","horizon:test",""]:
        for low,high in [([0,-64],[0,64]),([1,0],[1,5]),([2,5],[2,0]),([3,-5],[3,5]),([0,0],[0,0]),([0,64],[0,-64]),([0,-2**31],[0,2**31-1])]:
            conditions.append(["gradient",name,low,high])
    conditions += [["not",["hole"]],["not",["not",["stone",0,True,12,False]]]]
    secondary_bits=[0,0x80000000,1,0x80000001,0x3f000000,0xbf000000,0x3f800000,0xbf800000,0x40000000,0xc0000000,0x7f7fffff,0xff7fffff,0x7f800000,0xff800000,0x7fc00000]+[rng.getrandbits(32) for _ in range(81)]
    seeds=[0,1,-1,-2**63,2**63-1]+[rng.randrange(-2**63,2**63) for _ in range(19)]
    for fixture,seed in enumerate(seeds):
        ops=[["eval"],["eval"],["secondary"]]
        for i,bits in enumerate(secondary_bits):
            depth=edges[(i+fixture)%len(edges)]
            x,z=[rng.choice(edges) if i%3==0 else rng.randrange(-30000000,30000001) for _ in range(2)]
            above,below,water=[rng.choice(edges) for _ in range(3)]
            y=[-65,-64,-63,-1,0,1,62,63,64,65][i%10] if i%2 else rng.choice(edges)
            ops += [["input",depth,f"{bits:x}"],["xz",x,z,edges[i%len(edges)],edges[(i+9)%len(edges)]],["y",above,below,water,y],["eval"],["eval"]]
            # Changing source values alone does not invalidate cached results.
            ops += [["input",depth^1,f"{secondary_bits[(i+7)%len(secondary_bits)]:x}"],["secondary"],["y",below,above,water,y],["eval"],["secondary"]]
            if i%12==0:
                ops += [["add",["not",["gradient","horizon:late",[0,-64],[0,64]]]],["eval"]]
        for legacy in [False,True]:
            yield {"seed":seed,"legacy":legacy,"context":[[-64,384,63],[0,256,32],[-2032,4064,0],[2**31-1,-2**31,-2**31]][fixture%4],"conditions":conditions,"ops":ops}


def condition_resources(jar):
    prefix="data/minecraft/worldgen/material_condition/"
    with zipfile.ZipFile(jar) as archive:
        return {"minecraft:"+name[len(prefix):-5]:archive.read(name).decode("utf-8") for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith(".json")}


def codec_corpus(definitions):
    templates=[
        {"type":"hole"},{"type":"steep"},{"type":"temperature"},
        {"type":"biome","biome_is":["minecraft:plains"]},
        {"type":"noise_threshold","noise":"surface","min_threshold":-1.0,"max_threshold":1.0,"is_3d":False},
        {"type":"y_above","anchor":{"absolute":0},"surface_depth_multiplier":0,"add_stone_depth":False},
        {"type":"water","offset":0,"surface_depth_multiplier":0,"add_stone_depth":False},
        {"type":"stone_depth","offset":0,"add_surface_depth":False,"secondary_depth_range":0,"surface_type":"floor"},
        {"type":"vertical_gradient","random_name":"bedrock_floor","true_at_and_below":{"above_bottom":0},"false_at_and_above":{"above_bottom":5}},
        {"type":"not","invert":{"type":"hole"}},
    ]
    cases=['null','false','0','[]','{}','"minecraft:unknown"']
    cases += list(definitions.values())+[json.dumps(name) for name in definitions]
    cases += [json.dumps({"type":"not","invert":name}) for name in definitions]
    tokens=['null','true','false','0','1','-1','-21','-20','20','21','2147483647','-2147483648','4294967296','4294967295','0.99','-0.99','1e100','1e1000','-1e1000','1e-100','""','"floor"','"ceiling"','"FLOOR"','"true"','"a:b:c"','[]','{}','{"absolute":0}','{"above_bottom":0,"below_top":0,"relative_to_sea_level":0}']
    for template in templates:
        cases.append(json.dumps(template,separators=(",",":")))
        for key in template:
            cases.append(json.dumps({k:v for k,v in template.items() if k!=key}))
            for token in tokens:
                fields=[json.dumps(k)+":"+(token if k==key else json.dumps(v,separators=(",",":"))) for k,v in template.items()]
                cases.append("{"+",".join(fields)+"}")
        for prefix in ["minecraft:",":","other:","Minecraft:"]:
            cases.append(json.dumps(dict(template,type=prefix+template["type"])))
        cases.append(json.dumps(dict(template,unknown=[1,2,3])))
    cases += ['{"type":"hole","type":"steep"}','{"type":"hole","type":null}']
    base=next(corpus())
    for context in [[-64,384,63],[0,256,32],[-2032,4064,0],[2**31-1,-2**31,-2**31]]:
        for legacy in [False,True]:
            yield {"seed":-1,"legacy":legacy,"context":context,"conditions":[],"json":cases,"ops":base["ops"][:166]}


def rule_corpus(definitions):
    rng=random.Random(0x484f52495a4f4e52554c45)
    base=next(corpus())
    choices=base["conditions"]+[["json",json.dumps(name)] for name in sorted(definitions)]
    def tree(depth):
        kind=rng.randrange(4 if depth else 2)
        if kind==0:return ["block",rng.randrange(10)]
        if kind==1:return ["band"]
        if kind==2:return ["if",rng.choice(choices),tree(depth-1)]
        return ["sequence",[tree(depth-1) for _ in range(rng.randrange(4))]]
    rules=[["block",i] for i in range(10)]+[["band"],["sequence",[]],["sequence",[["block",8]]],["sequence",[["sequence",[]]]]]
    rules += [["if",c,["block",7]] for c in choices]
    rules += [["sequence",[["block",8],["if",["stone",0,False,12,False],["block",7]],["band"]]],
              ["sequence",[["if",["hole"],["block",0]],["if",["not",["hole"]],["band"]]]],
              ["if",["stone",0,True,12,False],["sequence",[["if",["steep"],["block",1]],["block",9]]]]]
    rules += [tree(3) for _ in range(48)]
    edges=[-2**31,-1000,-193,-192,-65,-64,-1,0,1,63,64,191,192,319,2**31-1]
    for fixture in range(12):
        ops=[["apply",0,0,0],["apply",0,0,0]]
        for i in range(64):
            x,y,z=[rng.choice(edges) for _ in range(3)]
            a,b,w=[rng.choice(edges) for _ in range(3)]
            ops += [["input",[-1,0,1,4,12,2**31-1][i%6],["0","3f800000","bf800000","7f800000","7fc00000"][i%5]],
                    ["xz",x,z,[-5,-4,-3,0,3,4,5][i%7],[5,4,3,0,-3,-4,-5][i%7]],["y",a,b,w,y],
                    ["apply",x,y,z],["apply",x,y,z],["apply",z,edges[(i+7)%len(edges)],x]]
            if i%16==0:ops += [["rule",tree(3)]]
        for legacy in [False,True]:
            yield {"seed":[0,1,-1,-2**63,2**63-1,987654321][fixture%6],"legacy":legacy,"context":[-64,384,63],"conditions":[],"rules":rules,"ops":ops}


def noise_corpus(noises):
    import struct
    bits=lambda value:struct.pack('>d',value).hex()
    bounds=[(-1.,1.),(0.,0.),(-0.,0.),(1.,-1.),(float('-inf'),float('inf')),(float('nan'),1.),(-1.,float('nan'))]
    conditions=[]
    for name in ['surface','minecraft:surface',':surface_secondary','minecraft:clay_bands_offset']:
        for is3d in [False,True]:
            for low,high in bounds:conditions.append(['noise',name,bits(low),bits(high),is3d])
    conditions += [['not',c] for c in conditions[:14]]
    random=__import__('random').Random(0x484f52495a4f4e4e4f495345)
    noise_bits=[0,0x80000000,0x3f800000,0xbf800000,0x3f800001,0xbf800001,0x3f7fffff,0xbf7fffff,0x7f800000,0xff800000,0x7fc00000]+[random.getrandbits(32) for _ in range(85)]
    edges=[-2**31,-1000,-64,-1,0,1,63,319,2**31-1]
    for real in [False,True]:
        selected=conditions if not real else [['noise',name,bits(-0.25),bits(0.25),is3d] for name in sorted(noises) for is3d in [False,True]]
        for seed in [0,1,-1,-2**63,2**63-1,87654321]:
            operations=[['eval'],['eval']]
            for i,b in enumerate(noise_bits):
                x,y,z=[random.choice(edges) for _ in range(3)]
                operations += [['input',0,f'{b:x}'],['xz',x,z,0,0],['y',0,0,0,y],['eval'],['eval'],['input',0,f'{noise_bits[(i+7)%len(noise_bits)]:x}'],['eval'],['y',0,0,0,y],['eval'],['xz',x,z,0,0],['eval']]
                if i%16==0:operations += [['add',selected[i%len(selected)]],['eval']]
            for legacy in [False,True]:
                yield {'seed':seed,'legacy':legacy,'context':[-64,384,63],'conditions':selected,'real_noise':real,'ops':operations}


def biome_corpus(biomes):
    names=sorted(biomes)
    conditions=[['temperature'],['not',['temperature']],['biome',[]],['biome',names]]
    conditions += [['biome',[name]] for name in names]
    conditions += [['not',['biome',[name]]] for name in names[::7]]
    conditions += [['biome',['minecraft:plains','minecraft:plains','minecraft:desert']]]
    choices=[None,[],['minecraft:plains'],['minecraft:desert'],['minecraft:plains','minecraft:desert'],names]
    ops=[['eval'],['eval']]
    for i,name in enumerate(names):
        ops += [['biome',name],['xz',i*31,i*-17,(i%9)-4,4-(i%9)],['eval'],['y',0,0,0,[-64,0,63,80,81,120,319][i%7]],['eval'],['eval']]
        if i%9==0:ops += [['add',['biome',[name]]],['eval']]
    # Errors after a previous result preserve that result on a same-revision
    # retry. Null biome results are queried again by uncached conditions.
    ops += [['biome','#throw'],['y',0,0,0,0],['eval'],['eval'],['add',['biome',['minecraft:plains']]],['eval'],['eval'],
            ['biome','minecraft:plains'],['eval'],['y',0,0,0,0],['eval'],['biome',None],['y',0,0,0,81],['eval'],['eval'],
            ['biome','minecraft:desert'],['xz',7,9,0,0],['eval'],['y',0,0,0,81],['eval']]
    for possible in choices:
        for sea in [63,-1000,2**31-1]:
            yield {'seed':0,'legacy':False,'context':[-64,384,sea],'possible_biomes':possible,'conditions':conditions,'ops':ops}


def biome_tag_resources(jar):
    prefix='data/minecraft/tags/worldgen/biome/'
    with zipfile.ZipFile(jar) as archive:
        return {'minecraft:'+name[len(prefix):-5]:archive.read(name).decode('utf-8') for name in sorted(archive.namelist()) if name.startswith(prefix) and name.endswith('.json')}


def biome_tag_corpus(biomes,tags):
    conditions=[['json',json.dumps({'type':'biome','biome_is':'#'+name})] for name in sorted(tags)]
    conditions += [['not',c] for c in conditions[:8]]
    invalid=[{'type':'biome','biome_is':value} for value in ['#minecraft:unknown','#',['#minecraft:is_badlands'],'##minecraft:is_badlands']]
    aliases=[{'type':'biome','biome_is':'#'+name.removeprefix('minecraft:')} for name in sorted(tags)]
    rules=[['sequence',[['if',c,['block',7]],['block',8]]] for c in conditions]
    for possible in [None,[],['minecraft:plains'],['minecraft:badlands'],sorted(biomes)]:
        ops=[['eval'],['eval']]
        for i,name in enumerate(sorted(biomes)):
            ops += [['biome',name],['xz',i,i*-3,0,0],['y',0,0,0,i],['eval'],['apply',i,i,i*-3]]
        ops += [['biome','#throw'],['y',0,0,0,80],['eval'],['eval'],['biome',None],['y',0,0,0,81],['eval'],['eval'],['biome','minecraft:plains'],['eval']]
        yield {'seed':0,'legacy':False,'context':[-64,384,63],'possible_biomes':possible,'conditions':conditions,'json':[json.dumps(v) for v in [*invalid,*aliases]],'rules':rules,'ops':ops}


def preliminary_corpus():
    def wrap(v): return (v+2**31)%2**32-2**31
    volumes=[[4,3,5,0,-64,0,1,1,1],[5,7,3,-17,32,19,4,8,4],
             [1,1,1,1024,0,-1024,1,1,1],[4,2,4,2**31-2,1,-2**31,3,2,5]]
    bit_values=['00000000','80000000','3f000000','bf000000','3f7fffff','bf800001','42810000',
                'c2810000','4effffff','4f000000','cf000000','cf000001','7f800000','ff800000','7fc00000','00000001','80000001']
    conditions=[['above'],['not',['above']],['not',['not',['above']]]]
    rules=[['if',['above'],['block',7]],['sequence',[['if',['not',['above']],['block',8]],['block',9]]]]
    for volume in volumes:
        sx,_,sz,x,_,z,_,_,_=volume
        for failure in [-1,0,1,3,19]:
            for first in ['minimum','eval','apply']:
                # Inside-plane allocation and failed retries, or initial scalar
                # lookup before the first allocation, vary independently.
                initial=[first] if first!='apply' else ['apply',0,200,0]
                ops=[['density','42810000',failure],['input',3,'0'],['xz',x,z,0,0],initial,['minimum'],['eval'],['apply',7,11,13]]
                ops += [['density','43400000',-1],['minimum'],['eval'],['y',0,0,0,80],['minimum'],['eval']]
                for iz in range(sz):
                    for ix in range(sx):
                        ops += [['xz',wrap(x+ix),wrap(z+iz),0,0],['minimum'],['y',0,0,0,(ix+iz)*31-8],['eval'],['apply',-17,90,31]]
                for i,bits in enumerate(bit_values):
                    depth=[-2**31,-9,0,3,8,2**31-1][i%6]
                    # Scalar fallback: floor/conversion, wrapping, cache reuse,
                    # failure after a previous successful minimum, and recovery.
                    ox,oz=wrap(x-1-i),wrap(z-1-i)
                    ops += [['density',bits,-1],['input',depth,'0'],['xz',ox,oz,0,0],['minimum']]
                    for y in [-2**31,-65,-8,-1,0,56,64,65,2**31-1]:
                        ops += [['y',0,0,0,y],['eval'],['apply',0,0,0]]
                    ops += [['density','0',0],['minimum'],['xz',ox,oz,0,0],['eval'],['minimum'],['eval'],['density',bits,-1],['minimum'],['xz',ox,oz,0,0],['minimum']]
                ops += [['add',['above']],['eval']]
                yield {'seed':failure,'legacy':first=='eval','context':[-64,384,63], 'volume':volume,'conditions':conditions,'rules':rules,
                       'json':['{"type":"above_preliminary_surface"}','{"type":"minecraft:above_preliminary_surface","ignored":null}'], 'ops':ops}
    # A scalar lookup outside the plane must not allocate or populate it.
    yield {'seed':0,'legacy':False,'context':[-64,384,63],'volume':[2,1,2,10,0,10,1,1,1],
           'conditions':conditions,'rules':rules,'ops':[['density','42810000',-1],['minimum'],['density','43400000',-1],['xz',10,10,0,0],['minimum'],['xz',0,0,0,0],['minimum']]}


def ore_corpus():
    def wrap(v): return (v+2**31)%2**32-2**31
    rng=random.Random(0x484f52495a4f4e4f5245)
    bits=['0','80000000','00000001','3e4ccccd','3f000000','3f333333','3f7fffff','3f800000',
          '3f800001','bf800000','7f800000','ff800000','7fc00000']
    volumes=[[3,4,2,-1,-2,5,1,1,1],[3,2,3,-10,-5,-15,4,8,4],[2,2,2,2**31-1,-2**31,2**31-1,2,3,2]]
    chances=['0','3ca3d70a','3f000000','3f800000','bf800000','7f800000','7fc00000']
    rules=[['ore',chance] for chance in chances]
    rules += [['sequence',[['block',0],['ore','3f800000']]],['if',['not',['hole']],['ore','0']]]
    for volume in volumes:
        sx,sy,sz,x,y,z,dx,dy,dz=volume
        positions=[[wrap(x+ix*dx),wrap(y+iy*dy),wrap(z+iz*dz)] for iz in range(sz) for ix in range(sx) for iy in range(sy)]
        positions += [[wrap(x-1),y,z],[x,wrap(y-1),z],[x,y,wrap(z-1)],[wrap(x+1),wrap(y+1),wrap(z+1)]]
        ops=[['apply',0,0,0]]
        for i in range(180):
            if i<len(positions):cx,cy,cz=positions[i]
            elif i%3==0:cx,cy,cz=positions[i%len(positions)]
            else:cx,cy,cz=[rng.choice([-2**31,2**31-1,-65,-17,-1,0,1,7,63,1000]) for _ in range(3)]
            for channel in range(3):ops.append(['ore_input',channel,bits[(i*(channel+1)+channel)%len(bits)],-1])
            ops += [['input',i%3,'0'],['xz',cx,cz,0,0],['y',0,0,0,cy],['apply',cx,cy,cz],['apply',cx,cy,cz],['apply',wrap(cx+17),wrap(cy-33),wrap(cz+91)]]
            if i%45==0:ops += [['rule',['ore',chances[(i//45)%len(chances)]]],['apply',cx,cy,cz]]
        for values in [['3f800000','3f800000','bf800000'],['3f333333','3e4ccccd','bf800000'],['0','3f800000','bf800000'],['7fc00000','7fc00000','7fc00000']]:
            for legacy in [False,True]:
                for seed in [0,-2**63]:
                    yield {'seed':seed,'legacy':legacy,'context':[-64,384,63],'conditions':[], 'volume':volume, 'ore_values':values,'rules':rules,'ops':ops}
    for channel in range(3):
        for remaining in [0,1,7]:
            ops=[['ore_input',channel,'3f800000',remaining],['rule',['sequence',[['block',0],['ore','3f800000']]]],
                 ['ore_input',channel,'bf800000' if channel==2 else '3f800000',-1],['rule',['ore','3f800000']],
                 ['xz',1000,1000,0,0],['y',0,0,0,1000],['apply',1000,1000,1000],
                 ['ore_input',channel,'3f800000',0],['apply',1000,1000,1000],['apply',1000,1000,1000],
                 ['xz',0,0,0,0],['y',0,0,0,0],['apply',0,0,0],['ore_input',channel,'bf800000' if channel==2 else '3f800000',-1],['apply',0,0,0]]
            yield {'seed':0,'legacy':False,'context':[-64,384,63],'conditions':[],'volume':[2,2,2,0,0,0,1,1,1], 'ore_values':['3f800000','3f800000','bf800000'],'rules':[],'ops':ops}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-material-condition-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    # The shared fixture binary has an optional bundled-library mode.
    (build/"rules.json").unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    resources=worldgen.noise_resources(jars["server"])
    if not resources or resources!=worldgen.noise_resources(jars["client"]):raise ValueError("client/server noise resources differ")
    prefix="data/minecraft/worldgen/noise/"
    noises={"minecraft:"+name[len(prefix):-5]:text for name,text in resources.items()}
    (build/"noises.json").write_text(json.dumps(noises,separators=(",",":")),encoding="utf-8")
    biomes=biome.biome_resources(jars["server"])
    if not biomes or biomes!=biome.biome_resources(jars["client"]):raise ValueError("client/server biome definitions differ")
    (build/"biomes.json").write_text(json.dumps(biomes,separators=(",",":")),encoding="utf-8")
    biome_tags=biome_tag_resources(jars["server"])
    if not biome_tags or biome_tags!=biome_tag_resources(jars["client"]):raise ValueError("client/server biome tags differ")
    biome_tag_count=len(biome_tags)
    biome_tags.update({"horizon:empty":'{"values":[]}',"horizon:chain":'{"values":["#minecraft:is_badlands","minecraft:plains",{"id":"minecraft:unknown","required":false}]}',"horizon:duplicates":'{"values":["minecraft:plains","minecraft:plains"]}'})
    (build/"biome_tags.json").write_text(json.dumps(biome_tags,separators=(",",":")),encoding="utf-8")
    conditions=condition_resources(jars["server"])
    if not conditions or conditions!=condition_resources(jars["client"]):raise ValueError("client/server material-condition resources differ")
    resource_count=len(conditions)
    conditions.update({"horizon:aa_forward":'{"type":"not","invert":"horizon:zz_leaf"}',"horizon:zz_leaf":'{"type":"hole"}',"horizon:chain":'{"type":"not","invert":"horizon:aa_forward"}',"horizon:":'{"type":"steep"}'})
    (build/"conditions.json").write_text(json.dumps(conditions,separators=(",",":")),encoding="utf-8")
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMaterialConditionOracle.java"
    anchor_oracle=oracle.with_name("VanillaAnchorOracle.java")
    biome_oracle=oracle.with_name("VanillaBiomeTemperatureOracle.java")
    sources=[oracle,anchor_oracle,biome_oracle,lab.ROOT/"tools/test_biome_temperature_parity.py",Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/material_condition_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=[*corpus(),*codec_corpus(conditions),*rule_corpus(conditions),*noise_corpus(noises),*biome_corpus(biomes),*biome_tag_corpus(biomes,biome_tags),*preliminary_corpus(),*ore_corpus()];inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs,build/"noises.json",build/"conditions.json",build/"biomes.json",build/"biome_tags.json"];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,anchor_oracle,biome_oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"material_condition_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaMaterialConditionOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} material-condition output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"initial_conditions":sum(len(f["conditions"]) for f in fixtures),"context_operations":sum(len(f["ops"]) for f in fixtures),"noise_resources":len(noises),"condition_resources":resource_count,"biome_resources":len(biomes),"biome_tag_resources":biome_tag_count,"biome_fixtures":sum("possible_biomes" in f for f in fixtures),"preliminary_fixtures":len(list(preliminary_corpus())),"ore_fixtures":sum("ore_values" in f for f in fixtures),"noise_fixtures":sum("real_noise" in f for f in fixtures),"initial_rules":sum(len(f.get("rules",[])) for f in fixtures),"codec_inputs":sum(len(f.get("json",[])) for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual runtime constructors and compiled evaluators for hole, steep, Y, water, stone-depth, vertical-gradient, noise-threshold, biome-membership, temperature and negated conditions, with actual MaterialRuleContext updates and secondary-noise caching. Both world random algorithms and named positional gradient factories, boundary/overflow arithmetic, floor/ceiling selection, reversed/equal gradient bounds, late compilation and repeated/changed context evaluations. Context surfaceDepth is injected after native updateXZ and the secondary sampler is a traced constant-noise override; these supplied inputs isolate condition behavior, with full MaterialSystem surface inputs checked separately. Direct and holder condition codecs cover all eight bundled definitions, canonical encoding, forward and nested references, default namespaces, numeric wrapping/truncation, required fields, invalid types and nested anchor alternatives. Cycles and bounded resource limits are typed Rust errors, not compared to native stack overflow. Actual BlockRule, SequenceRule, ConditionRule and BandlandsRule compilation/application additionally cover ordered short circuiting, nested and empty/singleton sequences, independent context versus application coordinates, late compilation and seeded band results/failures. Noise-threshold conditions additionally cover shared/separate XZ and XYZ cached suppliers, all bundled named noise resources under both random modes, traced changing source inputs, repeated coordinates/updates, late binding, float promotion, NaN/infinite thresholds and values. Encoded JSON numbers are compared as canonical double bits, including nonfinite Gson output tokens. Biome membership and temperature conditions additionally cover all bundled biome climate settings, possible-biome constant folding (including empty sets), context biome caching and XZ/Y invalidation, shared temperature caches across contexts, null biome results, query failures and lazy-condition retry behavior. Named biome-tag holders additionally cover every bundled biome tag, native TagLoader resolution/binding versus Rust TagDefinitions, encoded tag identity, namespace aliases, empty/duplicate/nested/optional entries, membership and possible-biome folding, predicate errors and conditional-rule application. Preliminary-surface conditions additionally cover lazy unit-spaced XZ plane sampling versus scalar fallback, traced sample order, ignored source-volume strides/Y extent, column minimum caching, block-height changes, partial density failures and retained buffers, same-column failure retries, floor/nonfinite conversions and wrapping arithmetic. Provider error occurrence is compared, not exception class or message. Preliminary planes are bounded to 1,048,576 samples in Rust. Ore-vein rules additionally cover eager density/richness volumes, lazy gap sampling, exact strided lattice hits and scalar misses, shared context versus independent application/random coordinates, both positional random modes, ordered draws and short circuiting, mutable/NaN/infinite sources, raw-ore chances, failed compilation and provider retries. Rust requires precompiled density roots for the same world context; live registry/tag reloads, rule codecs, registry-owner validation, arbitrary provider failures and full surface-stage application remain pending."}
    lab.write_json(report_path,report);print(f"Scoped material-condition parity passed: {report_path}")


if __name__=="__main__":main()
