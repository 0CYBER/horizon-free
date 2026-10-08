#!/usr/bin/env python3
"""Measure held-live attribute storage RSS on Linux; not a frame-rate benchmark."""
import argparse,json,os,platform,statistics,subprocess
from pathlib import Path
try:
    from . import lab,test_rust_parity as base
except ImportError:
    import lab,test_rust_parity as base

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--phase',required=True,choices=['before','after'])
    parser.add_argument('--catalog',type=Path,default=lab.LOCAL/'rust-entity-attribute-parity/catalog.json')
    args=parser.parse_args()
    if platform.system()!='Linux':raise SystemExit('This measurement uses Linux /proc status.')
    build=lab.LOCAL/'rust-attribute-memory';build.mkdir(exist_ok=True,parents=True)
    sources=[Path(__file__),lab.ROOT/'Cargo.toml',lab.ROOT/'Cargo.lock',lab.ROOT/'crates/horizon-world/src/attributes/entity.rs',lab.ROOT/'crates/horizon-world/src/attributes/mod.rs',lab.ROOT/'crates/horizon-world/examples/attribute_memory.rs']
    hashes=base.source_hashes(sources);catalog_hash=lab.digest(args.catalog,'sha256')
    with (build/'compile.log').open('w') as log:base.run(['cargo','build','--release','-p','horizon-world','--example','attribute_memory'],stdout=log,stderr=subprocess.STDOUT)
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'],cwd=lab.ROOT,text=True))
    binary=Path(metadata['target_directory'])/'release/examples/attribute_memory'
    results=[]
    for mode in ['defaults','active']:
        samples=[]
        for _ in range(5):
            process=subprocess.Popen([str(binary),str(args.catalog),'20000',mode],cwd=lab.ROOT,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                ready=json.loads(process.stdout.readline())
                if ready!={'entities':20000,'active_per_entity':4 if mode=='active' else 0,'health_sum':400000.0}:raise AssertionError(ready)
                status={line.split(':',1)[0]:line.split(':',1)[1].strip() for line in Path(f'/proc/{process.pid}/status').read_text().splitlines() if ':' in line}
                samples.append({key:int(status[key].split()[0]) for key in ['VmRSS','VmHWM']})
                _,err=process.communicate('\n',timeout=30)
                if process.returncode:raise RuntimeError(err)
            finally:
                if process.poll() is None:process.kill();process.wait()
        results.append({'mode':mode,'entities':20000,'active_per_entity':4 if mode=='active' else 0,'samples_kib':samples,'median_rss_kib':statistics.median(s['VmRSS'] for s in samples),'median_peak_kib':statistics.median(s['VmHWM'] for s in samples)})
    base.ensure_sources_unchanged(sources,hashes)
    if lab.digest(args.catalog,'sha256')!=catalog_hash:raise RuntimeError('Catalog changed during measurement')
    report=dict(schema=1,phase=args.phase,platform=platform.platform(),rustc=subprocess.check_output(['rustc','--version'],text=True).strip(),catalog_sha256=catalog_hash,sources_sha256=hashes,results=results,scope='Five fresh release-process samples per case, 20,000 held-live zombie attribute maps, no instances or four instantiated attributes. RSS/peak includes allocator/runtime/catalog. Identical health-read checksum. Excludes the rest of entity/world state; no timing, rendering or FPS conclusion.')
    lab.write_json(build/(args.phase+'.json'),report)
    print(json.dumps(results,indent=2))
if __name__=='__main__':main()
