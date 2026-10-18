import json,pathlib,statistics,re
root=pathlib.Path(__file__).resolve().parent
rows=[]
for phase in ['gate','matrix']:
 rows+=json.loads((root/(phase+'-raw.json')).read_text())
keys=['dimension','threads','seed','center','warm']
cases=[]
for row in rows:
 case={k:row[k] for k in keys}
 if case not in cases:cases.append(case)
summary=[]
for case in cases:
 group=[r for r in rows if all(r[k]==v for k,v in case.items()) and r['repeat']>=0]
 result=dict(case)
 for metric in ['wall_ms_chunk','combined_ms_chunk','noise','surface','biomes','structures','carvers','terrain_cpu_ms_chunk','peak_rss_kib','process_user_s','process_system_s']:
  med={v:statistics.median(r[metric] for r in group if r['version']==v) for v in ['baseline','candidate']}
  result[metric]={**med,'reduction_percent':100*(med['baseline']-med['candidate'])/med['baseline'] if med['baseline'] else None}
 pairs=[]
 for repeat in range(5):
  b,c=(next(r for r in group if r['repeat']==repeat and r['version']==v) for v in ['baseline','candidate'])
  pairs.append(100*(b['wall_ms_chunk']-c['wall_ms_chunk'])/b['wall_ms_chunk'])
 result['paired_wall_reduction_percent']=pairs
 result['candidate_slower_pairs']=sum(p<0 for p in pairs)
 if case['warm']:
  warmup={}
  for version in ['baseline','candidate']:
   times=[]
   for row in group:
    if row['version']!=version:continue
    filename=f"{row['dimension']}-t{row['threads']}-s{row['seed']}-c{row['center'][0]}_{row['center'][1]}-w1-r{row['repeat']}-{version}.log"
    times.append(float(re.search(r'warmup_ms ([0-9.]+)',(root/filename).read_text())[1]))
   warmup[version]=statistics.median(times)
  result['excluded_initialization_ms']=warmup
 summary.append(result)
(root/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
for s in summary:
 m=s['wall_ms_chunk'];c=s['combined_ms_chunk']
 print(s['dimension'],s['threads'],s['seed'],s['center'],'initialized' if s['warm'] else 'cold',f"{m['baseline']:.6f} -> {m['candidate']:.6f} ({m['reduction_percent']:.3f}%)",f"combined {c['baseline']:.6f} -> {c['candidate']:.6f} ({c['reduction_percent']:.3f}%)",'slower pairs',s['candidate_slower_pairs'])
