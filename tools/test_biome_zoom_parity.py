#!/usr/bin/env python3
"""Compare biome zoom, quart lookup and seed obfuscation with pinned vanilla."""
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


def corpus():
    rng=random.Random(0x484f52495a4f4e42494f4d45)
    seeds=[-2**63,-2**63+1,-987654321,-1,0,1,987654321,2**63-1]
    for seed in seeds+[rng.randrange(-2**63,2**63) for _ in range(4096)]:
        yield f's {seed}'
    edges=[-2**31,-2**31+1,-2**31+2,-30000000,-17,-16,-5,-4,-3,-2,-1,0,1,2,3,4,5,15,16,17,30000000,2**31-2,2**31-1]
    import itertools
    for seed in seeds:
        for x,y,z in itertools.product(edges,repeat=3):yield f'b {seed} {x} {y} {z}'
        for x,y,z in itertools.product(range(-8,9),repeat=3):yield f'b {seed} {x} {y} {z}'
    for _ in range(32768):
        seed=rng.randrange(-2**63,2**63);x,y,z=[rng.randrange(-2**31,2**31) for _ in range(3)]
        yield f'b {seed} {x} {y} {z}'
    import struct
    float_values=[float(v) for v in edges]+[-0.,0.1,-0.1,3.9999999999999996,-4.000000000000001,1e100,-1e100,float('inf'),-float('inf'),float('nan')]
    bits=[struct.pack('>d',v).hex() for v in float_values]+['0000000000000001','8000000000000001','7ff0000000000001','fff8000000000000']
    for a,b,c in itertools.product(bits,repeat=3):yield f'p {a} {b} {c}'
    for _ in range(32768):yield 'p '+' '.join(f'{rng.getrandbits(64):016x}' for _ in range(3))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--prepare",action="store_true");args=parser.parse_args()
    build=lab.LOCAL/"rust-biome-zoom-parity";build.mkdir(parents=True,exist_ok=True)
    report_path=build/"report.json";report_path.unlink(missing_ok=True)
    if args.prepare:lab.prepare()
    lock,jars,libraries=base.verify_artifacts()
    oracle=lab.ROOT/"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBiomeZoomOracle.java"
    sources=[oracle,Path(__file__),lab.ROOT/"tools/test_worldgen_parity.py",lab.ROOT/"tools/test_rust_parity.py",lab.ROOT/"tools/lab.py",lab.ROOT/"Cargo.toml",lab.ROOT/"Cargo.lock",lab.ROOT/"crates/horizon-core/Cargo.toml",lab.ROOT/"crates/horizon-world/Cargo.toml",
             *sorted((lab.ROOT/"crates/horizon-core/src").rglob("*.rs")),*sorted((lab.ROOT/"crates/horizon-world/src").rglob("*.rs")),lab.ROOT/"crates/horizon-core/examples/biome_zoom_fixture.rs"]
    tested_sources=base.source_hashes(sources)
    fixtures=list(corpus());inputs=build/"inputs.txt";inputs.write_text("\n".join(fixtures)+"\n",encoding="utf-8")
    fixture_paths=[inputs];tested_fixtures=base.source_hashes(fixture_paths);classes=build/"classes";classes.mkdir(exist_ok=True)
    def run(command,name):
        with (build/(name+".log")).open("w",encoding="utf-8") as log:base.run(list(map(str,command)),stdout=log,stderr=subprocess.STDOUT)
    dependency_path=os.pathsep.join(map(str,libraries))
    run(["javac","--release",lock["minecraft"]["java_major"],"-proc:none","-cp",str(jars["server"])+os.pathsep+dependency_path,"-d",classes,oracle],"compile-java")
    run(["cargo","build","--release","-p","horizon-core","--example","biome_zoom_fixture"],"compile-rust")
    metadata=json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version","1"],cwd=lab.ROOT,text=True))
    binary=Path(metadata["target_directory"])/"release/examples"/("biome_zoom_fixture.exe" if os.name=="nt" else "biome_zoom_fixture")
    run([binary,build],"rust")
    import hashlib
    with (build/"rust.txt").open() as output:
        for fixture in fixtures:
            result=output.readline().strip()
            if fixture.startswith('s '):
                seed=int(fixture[2:]);digest=hashlib.sha256(seed.to_bytes(8,'little',signed=True)).digest()
                if result!='s '+str(int.from_bytes(digest[:8],'little',signed=True)):raise AssertionError('seed SHA-256 disagrees with hashlib')
    references=[]
    for side,jar in jars.items():
        reference=build/f"{side}.txt"
        run(["java","-Xmx512M","-cp",os.pathsep.join([str(classes),str(jar),dependency_path]),"dev.horizonlab.parity.VanillaBiomeZoomOracle",side,build],side)
        matched=base.compare(reference,build/"rust.txt")
        references.append({"side":side,"jar_sha256":lab.digest(jar,"sha256"),"output_sha256":lab.digest(reference,"sha256"),"matched_output_lines":matched})
        print(f"{side}: {matched:,} biome-zoom output lines match Rust",flush=True)
    base.ensure_sources_unchanged(sources,tested_sources);base.ensure_sources_unchanged(fixture_paths,tested_fixtures)
    report={"schema":1,"minecraft":lock["minecraft"]["version"],"manifest_sha1":lock["minecraft"]["manifest_sha1"],"platform":platform.platform(),
            "java":subprocess.check_output(["java","--version"],text=True).splitlines()[0],"rustc":subprocess.check_output(["rustc","--version"],text=True).strip(),
            "fixtures":len(fixtures),"seed_hashes":sum(f.startswith("s ") for f in fixtures),"block_queries":sum(f.startswith("b ") for f in fixtures),"position_queries":sum(f.startswith("p ") for f in fixtures),
            "sources_sha256":tested_sources,"fixtures_sha256":tested_fixtures,"rust_output_sha256":lab.digest(build/"rust.txt","sha256"),"references":references,
            "scope":"Actual BiomeManager seed obfuscation, block-position eight-corner jitter selection, direct block/double-to-quart queries, and source replacement preserving the zoom seed. Captured resolver coordinates are compared, including dense local grids, signed coordinate overflow, negative boundaries, distant blocks, long-seed boundaries, random seeds/positions, double infinities/NaNs/subnormals and saturating conversion. SHA-256 seed transforms are also compared against Python hashlib independently. Biome storage/source generation, blending, chunk integration and gameplay remain separate work."}

    lab.write_json(report_path,report);print(f"Scoped biome-zoom parity passed: {report_path}")


if __name__=="__main__":main()
