"""Run the repository's complete terrain-fill parity harness in isolated storage.

Only the Rust build profile/path changes from release to fast. Oracle, corpus,
source authentication and exact comparison logic remain the repository's own.
"""
import pathlib,sys,json,hashlib
root=pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(root/'tools'))
import lab
private=pathlib.Path(__file__).parent/'private-parity'
private.mkdir(exist_ok=True)
if not (private/'minecraft').exists():
 (private/'minecraft').symlink_to('/srv/bob-data/horizon/.local/minecraft',target_is_directory=True)
lab.LOCAL=private
script=root/'tools/test_terrain_parity.py'
source=script.read_text()
old='["cargo","build","--release","-p","horizon-world","--example","terrain_fixture"]'
assert source.count(old)==1
source=source.replace(old,'["cargo","build","--profile","fast","-p","horizon-world","--example","terrain_fixture"]')
assert source.count('/"release/examples"/')==1
source=source.replace('/"release/examples"/','/"fast/examples"/')
namespace={'__file__':str(script),'__name__':'horizon_terrain_fast_validation'}
exec(compile(source,str(script),'exec'),namespace)
namespace['main']()
report=json.loads((private/'rust-terrain-parity/report.json').read_text())
report['harness_adjustments']={'profile':'fast','storage':str(private),'runner_sha256':hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),'original_harness_sha256':hashlib.sha256(script.read_bytes()).hexdigest()}
(pathlib.Path(__file__).parent/'terrain-parity.json').write_text(json.dumps(report,indent=2)+'\n')
