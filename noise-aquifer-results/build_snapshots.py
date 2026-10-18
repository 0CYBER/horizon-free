"""Build diagnostic snapshots for both revisions, restoring candidate sources on exit."""
import os,pathlib,subprocess,shutil
root=pathlib.Path(__file__).resolve().parents[1]
results=pathlib.Path(__file__).parent
work=pathlib.Path('/srv/bob-data/horizon-noise-aquifer-e477fa56')
target=pathlib.Path('/srv/bob-data/horizon-stage3d-target')
env=dict(os.environ,CARGO_TARGET_DIR=str(target))
arithmetic=root/'crates/horizon-core/src/density/arithmetic.rs'
candidate=arithmetic.read_bytes()
(work/'candidate-arithmetic.rs').write_bytes(candidate)
def build(args,log):
 with (results/log).open('w') as f:subprocess.run(['cargo','build','--profile','fast',*args],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
try:
 build(['-p','horizon-world','--example','chunkgen_snapshot'],'snapshot-candidate-build.log')
 shutil.copy2(target/'fast/examples/chunkgen_snapshot',work/'snapshot-candidate')
 build(['-p','horizon-core','--bin','horizon-density-parity'],'density-build.log')
 shutil.copy2(target/'fast/horizon-density-parity',work/'horizon-density-parity')
 arithmetic.write_bytes(subprocess.check_output(['git','show','e4f8154cb4115aa1ee622381d9b12387cdf515d4:crates/horizon-core/src/density/arithmetic.rs'],cwd=root))
 build(['-p','horizon-world','--example','chunkgen_snapshot'],'snapshot-baseline-build.log')
 shutil.copy2(target/'fast/examples/chunkgen_snapshot',work/'snapshot-baseline')
finally:
 arithmetic.write_bytes(candidate)
