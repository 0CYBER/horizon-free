#!/usr/bin/env python3
"""Compare scalar value-provider decoding and seeded samples with pinned vanilla."""
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
    for kind in ['float','int']:
        for token in ['null','false','true','[]','{}','"0"','0','-0.0','1','-1','0.123456789','2.9','-2.9','4294967296','-4294967297','1e100','-1e100','1e-100','-1e-100']:
            yield kind,token
            yield kind,'{"type":"constant","value":'+token+'}'
        for value in [{},{'type':'missing'},{'type':None},{'type':'constant'},{'type':'Constant','value':0},{'type':':constant','value':17}]:yield kind,encode(value)
    for minimum,maximum in [(0,1),(-1,1),(-0.,0.),(1,1),(2,1),(-1e100,1e100),(1e100,1e100),(-1e-100,1e-100),(-0.7,1.3),(-1e30,1e30)]:
        yield 'float',encode({'type':'uniform','min_inclusive':minimum,'max_exclusive':maximum})
        for plateau in [-1,0,0.1,1,3,1e100]:yield 'float',encode({'type':'trapezoid','min':minimum,'max':maximum,'plateau':plateau})
    ranges=[(0,0),(0,1),(-1,1),(1,1),(2,1),(-7,13),(0,14),(0,9),(-2**31,2**31-1),(0,2**31-1),(-2**31,-2**31+100),(2**31-100,2**31-1),(0,4294967297)]
    for minimum,maximum in ranges:
        for kind in ['uniform','biased_to_bottom','very_biased_to_bottom']:
            value={'type':kind,'min_inclusive':minimum,'max_inclusive':maximum};yield 'int',encode(value)
            for low,high in [(0,3),(-100,-50),(50,100),(1,0)]:yield 'int',encode({'type':'clamped','source':value,'min_inclusive':low,'max_inclusive':high})
        for plateau in [-2**31,-10,-1,0,1,3,maximum-minimum,2**31-1]:yield 'int',encode({'type':'trapezoid','min':minimum,'max':maximum,'plateau':plateau})
    for kind in ['float','int']:
        yield kind,'{"type":"constant","value":0,"value":-0.0}'
        for field in ['min_inclusive','max_exclusive' if kind=='float' else 'max_inclusive']:
            for token in ['null','false','"1"','{}','[]']:
                yield kind,'{"type":"uniform","min_inclusive":0,"'+('max_exclusive' if kind=='float' else 'max_inclusive')+'":1,"'+field+'":'+token+'}'


    # Zero-weight entries remain part of declared bounds, but are never sampled.
    constant = (lambda n: {'absolute': n}) if False else (lambda n: n)
    weighted = lambda entries: {'type': 'weighted_list', 'distribution': entries}
    entry = lambda value, weight: {'data': value, 'weight': weight}
    choices = [constant(-7), constant(19), {'type': 'uniform', 'min_inclusive': constant(0), 'max_inclusive': constant(31)}]
    for weights in [(1,), (0,), (0, 1), (1, 0), (1, 1), (1, 62), (1, 63), (1, 64), (0, 3, 7), (2147483647,), (2147483646, 1), (2147483647, 1), (-1, 2), (4294967297, 2), (0.9, 1.9)]:
        value = weighted([entry(choices[i % len(choices)], weight) for i, weight in enumerate(weights)])
        yield 'int', encode(value)
    nested = weighted([entry(constant(5), 1), entry(constant(-9), 4)])
    yield 'int', encode(weighted([entry(nested, 3), entry(choices[-1], 2)]))
    for distribution in [None, {}, [], [entry(constant(0), 0), entry(constant(1), 0)], [entry(None, 0), entry(constant(1), 1)], [{'data': constant(1)}], [{'weight': 1}], [entry(constant(1), '1')], [entry(constant(1), None)]]:
        yield 'int', encode(weighted(distribution))


def corpus():
    definitions_list=list(definitions())
    for legacy in [False,True]:
        for seed in [0,-1,0x123456789abcdef,-2**63,2**63-1]:
            for kind,definition in definitions_list:yield {'kind':kind,'definition':definition,'legacy':legacy,'seed':seed,'samples':64}


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-value-provider-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaValueProviderOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-world/examples/probe/value_provider_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("".join(json.dumps(f,separators=(",",":"))+"\n" for f in fixtures),encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-world","--example","world_probe"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("world_probe.exe" if os.name=="nt" else "world_probe")
    run([binary,"value_provider_fixture",build],"rust")
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaValueProviderOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} value-provider output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"definitions":len(list(definitions())),"samples_per_valid_fixture":64,
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual FloatProviders.CODEC and IntProviders.CODEC decoding, declared ranges, seeded samples and interleaved subsequent nextLong values. Covers constant/uniform/trapezoid floats and constant/uniform/biased-to-bottom/very-biased-to-bottom/trapezoid/clamped/weighted-list integers, both random modes, numeric narrowing, signed zeros, overflow, equal/reversed bounds, negative plateaus, missing fields, duplicate keys and sampling failures. Float NaNs are canonicalized. Weighted lists include zero weights, singleton draws, nested providers, 63/64/65 weight totals, maximum/overflow totals and malformed entries; native constructor rejections count as decode errors. Gaussian providers, encoding, height providers and complete generation remain outside this scope."}

    lab.write_json(report_path,report);print(f"Scoped value-provider parity passed: {report_path}")


if __name__=="__main__":main()
