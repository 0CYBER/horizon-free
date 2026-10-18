"""Compare every terrain/decorated chunk and the extra diagnostic metadata exactly."""
import subprocess,os,json,pathlib,hashlib,filecmp
root=pathlib.Path(__file__).resolve().parent
work=pathlib.Path('/srv/bob-data/horizon-noise-aquifer-e477fa56')
resources='/srv/bob-data/horizon/.local/horizon-client/26.3'
rows=[]
for dim in ['overworld','the_nether','the_end']:
 for threads in [1,4]:
  for seed,cx,cz in [(12345,0,0),(-1,32,-32)]:
   label=f'{dim}-t{threads}-s{seed}-c{cx}_{cz}'
   outputs={}
   for version in ['baseline','candidate']:
    path=work/('snap-'+label+'-'+version)
    env=dict(os.environ,HORIZON_PROFILE_WARM='1',HORIZON_SNAPSHOT_PATH=str(path))
    cmd=[str(work/('snapshot-'+version)),resources,'--seed',str(seed),'--size','9','--threads',str(threads),'--dimension',dim,'--center',str(cx),str(cz)]
    result=subprocess.run(cmd,env=env,text=True,capture_output=True,check=True)
    (root/('parity-'+label+'-'+version+'.log')).write_text(result.stdout+result.stderr)
    outputs[version]={phase:pathlib.Path(str(path)+'-'+phase+'.bin') for phase in ['terrain','decorated']}
   for phase in ['terrain','decorated']:
    a,b=(outputs[v][phase] for v in ['baseline','candidate'])
    equal=filecmp.cmp(a,b,shallow=False)
    row=dict(dimension=dim,threads=threads,seed=seed,center=[cx,cz],phase=phase,terrain_chunks=81,decorated_chunks=49 if phase=='decorated' else 0,baseline_path=str(a),candidate_path=str(b),baseline_bytes=a.stat().st_size,candidate_bytes=b.stat().st_size,equal=equal)
    for version,p in [('baseline',a),('candidate',b)]:
     with p.open('rb') as f:row[version+'_sha256']=hashlib.file_digest(f,'sha256').hexdigest()
    rows.append(row)
    (root/'chunk-parity.json').write_text(json.dumps(rows,indent=2)+'\n')
    print(label,phase,'MATCH' if equal else 'MISMATCH',flush=True)
    if not equal:raise SystemExit('Chunk differential failed; snapshots retained')
