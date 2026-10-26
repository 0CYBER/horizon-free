#!/usr/bin/env python3
"""Compare registry-bound biome-source codecs and decoded runtime behavior."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab,test_rust_parity as base,test_worldgen_parity as worldgen,test_biome_temperature_parity as temperature,test_material_condition_parity as material
except ImportError:
    import lab
    import test_rust_parity as base
    import test_worldgen_parity as worldgen
    import test_biome_temperature_parity as temperature
    import test_material_condition_parity as material


def corpus(bundle):
    encode=lambda value:json.dumps(value,separators=(',',':'))
    for value in [None,False,0,'fixed',[],{}, {'type':'missing'}, {'type':None}]:yield encode(value)
    for name in sorted(bundle['biomes']):
        yield encode({'type':'fixed','biome':name})
        yield encode({'type':'checkerboard','biomes':name})
    for name in sorted(bundle['biome_tags']):yield encode({'type':'checkerboard','biomes':'#'+name,'scale':3})
    for value in [None,False,0,[],{},'',':','missing','Minecraft:plains','#is_overworld',['plains'],['plains','desert','plains'],['#is_overworld']]:
        yield encode({'type':'fixed','biome':value})
        yield encode({'type':'checkerboard','biomes':value})
    for token in ['null','false','"2"','{}','[]','-1','0','1','2','30','31','32','62','63','-0.0','2.9','-0.9','4294967296','4294967298','2147483648','-4294967296','1e100','-1e100','1e-100']:
        yield '{"type":"checkerboard","biomes":["plains","desert"],"scale":'+token+'}'
    yield '{"type":"checkerboard","biomes":"plains","scale":-1,"scale":2}'
    yield '{"type":":fixed","biome":":plains","ignored":true}'
    yield '{"type":"fixed","biome":"missing","biome":"plains"}'
    for value in [{},{'biome':'missing'},{'preset':None}]:yield encode({'type':'the_end',**value})
    point={name:0 for name in ['temperature','humidity','continentalness','erosion','depth','weirdness','offset']}
    good={'parameters':point,'biome':'plains'}
    for preset in [None,False,0,[],{},'overworld','nether',':overworld','missing',{'preset':'overworld'},{'preset':'nether'},{'preset':'missing'},{'preset':None}]:
        yield encode({'type':'multi_noise','preset':preset})
        yield encode({'type':'multi_noise','biomes':[good],'preset':preset})
        yield encode({'type':'multi_noise','biomes':[],'preset':preset})
        yield encode({'type':'multi_noise','biomes':[{'biome':'missing','parameters':point}],'preset':preset})
    for entries in [[],[good],[good,good],[good,{'parameters':point,'biome':'desert'}],[{}],[{'parameters':point}],[{'biome':'plains'}],None,'plains',0]:yield encode({'type':'multi_noise','biomes':entries})
    for field in point:
        for value in [None,False,'0',[],{},[-1,1],{'min':-1,'max':1},[1,-1],[-0.0,0.0],[0.0,-0.0],-2,-2.0001,2,2.0001,-0.0,0.00009999,0.123456789,1e100]:
            changed={**point,field:value}
            yield encode({'type':'multi_noise','biomes':[{'parameters':changed,'biome':'plains'}]})
    for name in point:
        changed={key:value for key,value in point.items() if key!=name}
        yield encode({'type':'multi_noise','biomes':[{'parameters':changed,'biome':'plains'}]})
    yield encode({'type':'multi_noise','biomes':[{'parameters':{**point,'temperature':[-0.1111,0.2222],'offset':0.012345},'biome':'desert'}]})


    # Cover every quantized climate value in [-2,2], including interval encoding
    # and every nonnegative offset increment. Input floats are parsed by each codec.
    for step in range(-20000,20001):
        changed={**point,'temperature':step/10000,'humidity':[-2,step/10000],'offset':abs(step)%10001/10000}
        yield encode({'type':'multi_noise','biomes':[{'parameters':changed,'biome':'plains'}]})


def positions():
    return [[0,0,0],[256,0,0],[260,-1,0],[-257,8,1],[7500000,79,-7500000],[-2**31,2**31-1,-1],[2**31-1,-2**31,17]]


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-biome-source-codec-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBiomeSourceCodecOracle.java"
    exporter=oracle.with_name("VanillaBiomeExport.java")
    helpers=[oracle.with_name(name+".java") for name in ["VanillaMaterialConditionOracle","VanillaAnchorOracle","VanillaBiomeTemperatureOracle"]]
    sources=[oracle,exporter,*helpers,lab.ROOT/"tools/test_biome_temperature_parity.py",lab.ROOT/"tools/test_material_condition_parity.py",Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/biome_source_codec_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,exporter,*helpers],"compile-java")
    bundle={}
    for field,getter in [('biomes',temperature.biome_resources),('biome_tags',material.biome_tag_resources)]:
        bundle[field]=getter(jars['server'])
        if not bundle[field] or bundle[field]!=getter(jars['client']):raise ValueError('reference '+field+' resources differ')
        (build/(field+'.json')).write_text(json.dumps(bundle[field],separators=(',',':')),encoding='utf-8')
    exports=[]
    for side,jar in jars.items():
        path=build/f'presets-{side}.json';exports.append(path)
        run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaBiomeExport',path],side+'-export')
        presets=json.loads(path.read_text())
        if not presets or ('presets' in bundle and presets!=bundle['presets']):raise ValueError('reference biome preset tables differ')
        bundle['presets']=presets
    (build/'presets.json').write_text(json.dumps(bundle['presets'],separators=(',',':')),encoding='utf-8')
    fixtures=list(corpus(bundle));inputs=build/'inputs.txt';inputs.write_text('\n'.join(fixtures)+'\n',encoding='utf-8')
    (build/'positions.json').write_text(json.dumps(positions(),separators=(',',':')),encoding='utf-8')
    fixture_paths=[inputs,*exports,build/'positions.json',*[build/(name+'.json') for name in bundle]];tested_fixtures=base.source_hashes(fixture_paths)
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"biome_source_codec_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBiomeSourceCodecOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} biome-source codec output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"query_positions":len(positions()),"climate_grid_cases":40001,"biomes":len(bundle['biomes']),"biome_tags":len(bundle['biome_tags']),

            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual BiomeSource.CODEC parsing and encoding for fixed, checkerboard, End and multi-noise sources bound to pinned biome/tag registries and preset holders. Named biome holders, direct/tag holder sets, canonical identifiers, scale defaults/bounds/numeric narrowing, inline climate parameter lists, named/inline preset holders, inline-list preference/fallback, malformed fields and duplicate keys are compared. Canonical serialized JSON is compared as sorted structures with numeric double-bit values; possible-biome sets are sorted and compared, followed by source queries against zero climate/erosion. Rust imports independently authenticated preset tables. Full inline biome definitions, live registry ownership/reloads and complete source/world generation remain outside this scoped codec comparison."}

    lab.write_json(report_path,report);print(f"Scoped biome-source codec parity passed: {report_path}")


if __name__=="__main__":main()
