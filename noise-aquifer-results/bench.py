import subprocess,os,json,pathlib,re,statistics,sys
root=pathlib.Path(__file__).resolve().parent
bins=pathlib.Path('/srv/bob-data/horizon-noise-aquifer-e477fa56')
resources='/srv/bob-data/horizon/.local/horizon-client/26.3'
phase=sys.argv[1] if len(sys.argv)>1 else 'gate'
cases=[('overworld',1,12345,0,0,True)] if phase=='gate' else [(d,t,12345,0,0,True) for d in ['overworld','the_nether','the_end'] for t in [1,4] if (d,t)!=('overworld',1)]+[('overworld',t,-1,32,-32,True) for t in [1,4]]+[('overworld',1,12345,0,0,False)]
rows=[]
for dim,threads,seed,cx,cz,warm in cases:
 for repeat in range(-1,5):
  order=['baseline','candidate'] if repeat%2==0 else ['candidate','baseline']
  for version in order:
   name=f'{dim}-t{threads}-s{seed}-c{cx}_{cz}-w{int(warm)}-r{repeat}-{version}'
   cmd=[sys.executable,'-c',"import resource,subprocess,sys; r=subprocess.run(sys.argv[1:]); u=resource.getrusage(resource.RUSAGE_CHILDREN); print(f'RESOURCE {u.ru_utime} {u.ru_stime} {u.ru_maxrss}',file=sys.stderr); sys.exit(r.returncode)",str(bins/version),resources,'--seed',str(seed),'--size','16','--threads',str(threads),'--dimension',dim,'--center',str(cx),str(cz)]
   env=dict(os.environ)
   env.pop('HORIZON_PROFILE_SIGNAL',None);env.pop('HORIZON_PROFILE_WARM',None)
   if warm:env['HORIZON_PROFILE_WARM']='1'
   r=subprocess.run(cmd,text=True,capture_output=True,env=env,check=True)
   (root/(name+'.log')).write_text(r.stdout+r.stderr)
   row=json.loads(next(x.split(' ',1)[1] for x in r.stdout.splitlines() if x.startswith('PROFILE_STAGES ')))
   m=re.search(r'decorated (\d+) chunks in ([0-9.]+)(ms|s)',r.stdout)
   deco=float(m[2])*(1 if m[3]=='ms' else 1000)
   # Parent diagnostic prints rounded decoration duration, retain this precision limit.
   row.update(version=version,repeat=repeat,dimension=dim,seed=seed,center=[cx,cz],warm=warm,cmd=cmd,decorated=int(m[1]),decoration_ms=deco,combined_ms_chunk=row['wall_ms_chunk']+deco/row['chunks'])
   cpu=re.search(r'cpu time ([0-9.]+) ms/chunk',r.stdout)
   resource=re.search(r'RESOURCE ([0-9.]+) ([0-9.]+) (\d+)',r.stderr)
   row.update(terrain_cpu_ms_chunk=float(cpu[1]),process_user_s=float(resource[1]),process_system_s=float(resource[2]),peak_rss_kib=int(resource[3]))
   rows.append(row)
   (root/(phase+'-raw.json')).write_text(json.dumps(rows,indent=2)+'\n')
   print(name,round(row['wall_ms_chunk'],6),flush=True)
summary=[]
for case in cases:
 dim,threads,seed,cx,cz,warm=case
 rs=[r for r in rows if (r['dimension'],r['threads'],r['seed'],*r['center'],r['warm'])==case and r['repeat']>=0]
 out=dict(dimension=dim,threads=threads,seed=seed,center=[cx,cz],warm=warm)
 for metric in ['wall_ms_chunk','combined_ms_chunk','noise','surface','terrain_cpu_ms_chunk','peak_rss_kib']:
  med={v:statistics.median(r[metric] for r in rs if r['version']==v) for v in ['baseline','candidate']}
  out[metric]={**med,'reduction_percent':100*(med['baseline']-med['candidate'])/med['baseline']}
 summary.append(out)
(root/(phase+'-summary.json')).write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps(summary,indent=2))
