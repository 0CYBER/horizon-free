"""Run the existing pinned density corpus with a fast-profile candidate and fresh Java oracles."""
import sys,os,json,pathlib,subprocess,hashlib
root=pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(root/'tools'))
import lab,test_rust_parity as base
work=pathlib.Path('/srv/bob-data/horizon-noise-aquifer-e477fa56/parity')
work.mkdir(parents=True,exist_ok=True)
(work/'minecraft').symlink_to('/srv/bob-data/horizon/.local/minecraft',target_is_directory=True) if not (work/'minecraft').exists() else None
lab.LOCAL=work
lock,jars,libraries=base.verify_artifacts()
shared=pathlib.Path('/srv/bob-data/horizon/.local/rust-density-parity')
prior=json.loads((shared/'report.json').read_text())
inputs=shared/'inputs.txt'
assert lab.digest(inputs,'sha256')==prior['inputs_sha256']
for name in ['mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDensityOracle.java','tools/test_density_parity.py','tools/test_rust_parity.py','tools/lab.py']:
 assert lab.digest(root/name,'sha256')==prior['sources_sha256'][name], name
classes=work/'classes';classes.mkdir(exist_ok=True)
dependency_path=os.pathsep.join(map(str,libraries))
source=root/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaDensityOracle.java'
subprocess.run(['javac','--release',str(lock['minecraft']['java_major']),'-proc:none','-cp',str(jars['server'])+os.pathsep+dependency_path,'-d',str(classes),str(source)],check=True)
binary=pathlib.Path('/srv/bob-data/horizon-noise-aquifer-e477fa56/horizon-density-parity')
rust=work/'density-rust.txt'
with inputs.open('rb') as stdin,rust.open('wb') as stdout:
 subprocess.run([str(binary)],stdin=stdin,stdout=stdout,check=True)
references=[]
for side,jar in jars.items():
 output=work/f'density-{side}.txt'
 subprocess.run(['java','-Xmx512M','-cp',os.pathsep.join([str(classes),str(jar),dependency_path]),'dev.horizonlab.parity.VanillaDensityOracle',str(inputs),str(output)],check=True)
 lines=base.compare(output,rust)
 references.append(dict(side=side,jar_sha256=lab.digest(jar,'sha256'),output_sha256=lab.digest(output,'sha256'),matched_output_lines=lines))
 print(side,lines,'lines match',flush=True)
report=dict(minecraft=lock['minecraft'],profile='fast',binary_sha256=lab.digest(binary,'sha256'),inputs_sha256=lab.digest(inputs,'sha256'),references=references,rust_output_sha256=lab.digest(rust,'sha256'),oracle_source_sha256=lab.digest(source,'sha256'),corpus_script_sha256=lab.digest(root/'tools/test_density_parity.py','sha256'),runner_sha256=lab.digest(pathlib.Path(__file__),'sha256'),sources_sha256=base.source_hashes([root/'crates/horizon-core/src/density/arithmetic.rs',root/'crates/horizon-core/src/density/tests.rs']),scope='Existing corpus authenticated against prior report and unchanged generator/oracle source; candidate and both checksum-pinned Java sides run freshly; density parity, not whole-game parity.')
(pathlib.Path(__file__).parent/'density-parity.json').write_text(json.dumps(report,indent=2)+'\n')
