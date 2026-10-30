#!/usr/bin/env python3
"""Compare anchored height-provider decoding and seeded samples with pinned vanilla."""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab,test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def definitions():
    encode=lambda value:json.dumps(value,separators=(',',':'))
    anchors=[{'absolute':-64},{'absolute':0},{'absolute':1},{'absolute':2031},{'above_bottom':8},{'below_top':1},{'relative_to_sea_level':-8}]
    for value in [None,False,0,[],{},*anchors,{'absolute':2032},{'absolute':0,'above_bottom':1}]:
        yield encode(value);yield encode({'type':'constant','value':value})
    for minimum in anchors:
        for maximum in anchors:
            for kind in ['uniform','biased_to_bottom','very_biased_to_bottom','trapezoid']:
                yield encode({'type':kind,'min_inclusive':minimum,'max_inclusive':maximum})
    for kind in ['biased_to_bottom','very_biased_to_bottom','trapezoid']:
        field='plateau' if kind=='trapezoid' else 'inner'
        for token in ['null','false','"1"','{}','[]','-1','0','1','2','4','14','100','2147483647','-2147483648','4294967297','1e100']:
            yield '{"type":"'+kind+'","min_inclusive":{"absolute":0},"max_inclusive":{"absolute":14},"'+field+'":'+token+'}'
    yield '{"type":"uniform","absolute":5,"min_inclusive":{"absolute":0},"max_inclusive":{"absolute":1}}'
    yield '{"type":"constant","value":{"absolute":0},"value":{"relative_to_sea_level":0}}'
    yield '{"type":"trapezoid","min_inclusive":{"absolute":-7},"max_inclusive":{"absolute":7}}'
    yield '{"type":":uniform","min_inclusive":{"above_bottom":8},"max_inclusive":{"below_top":1}}'


    # Zero-weight entries remain part of declared bounds, but are never sampled.
    constant = (lambda n: {'absolute': n}) if True else (lambda n: n)
    weighted = lambda entries: {'type': 'weighted_list', 'distribution': entries}
    entry = lambda value, weight: {'data': value, 'weight': weight}
    choices = [constant(-7), constant(19), {'type': 'uniform', 'min_inclusive': constant(0), 'max_inclusive': constant(31)}]
    for weights in [(1,), (0,), (0, 1), (1, 0), (1, 1), (1, 62), (1, 63), (1, 64), (0, 3, 7), (2147483647,), (2147483646, 1), (2147483647, 1), (-1, 2), (4294967297, 2), (0.9, 1.9)]:
        value = weighted([entry(choices[i % len(choices)], weight) for i, weight in enumerate(weights)])
        yield encode(value)
    nested = weighted([entry(constant(5), 1), entry(constant(-9), 4)])
    yield encode(weighted([entry(nested, 3), entry(choices[-1], 2)]))
    for distribution in [None, {}, [], [entry(constant(0), 0), entry(constant(1), 0)], [entry(None, 0), entry(constant(1), 1)], [{'data': constant(1)}], [{'weight': 1}], [entry(constant(1), '1')], [entry(constant(1), None)]]:
        yield encode(weighted(distribution))


def corpus():
    definitions_list=list(definitions())
    contexts=[[-64,384,63],[0,128,32],[-2032,4064,0],[2**31-1,-2**31,2**31-1],[-2**31,2**31-1,-2**31]]
    for legacy in [False,True]:
        for seed in [0,-1,0x123456789abcdef]:
            for context in contexts:
                for definition in definitions_list:yield {'definition':definition,'context':context,'legacy':legacy,'seed':seed,'samples':32}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-height-provider-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightProviderOracle.java"
    helper=oracle.with_name("VanillaAnchorOracle.java")
    sources=[oracle,helper,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/height_provider_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle,helper],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"height_provider_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaHeightProviderOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} height-provider output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"definitions":len(list(definitions())),"samples_per_valid_fixture":32,
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual HeightProvider.CODEC decoding and sampled heights with interleaved nextLong values, using actual WorldGenerationContext and all four vertical-anchor kinds. Covers constant, uniform, biased-to-bottom, very-biased-to-bottom trapezoid and weighted-list providers, both random modes, empty/equal ranges, inner/plateau defaults and bounds, numeric narrowing, relative coordinate overflow, malformed fields and anchor-versus-dispatch preference. Warning logs are excluded. Weighted lists include zero weights, singleton draws, nested providers, 63/64/65 weight totals, maximum/overflow totals and malformed entries; native constructor rejections count as decode errors. Encoding, cave/canyon paths and complete generation remain outside this scope."}

    lab.write_json(report_path,report);print(f"Scoped height-provider parity passed: {report_path}")


if __name__=="__main__":main()
