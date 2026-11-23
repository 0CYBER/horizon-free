"""Slices of tools/parity_fuzz.py as one sharded harness, for a hosted run on a `parity-probe/*` branch (#20446).

parity-hosted.yml runs harnesses by name and keeps only the log of one that did not pass. So a probe branch (never
main) adds `tools/test_fuzz_probe_parity.py`, which hands its slices to `run`: each shard runs one, prints the fuzzer's
output and its `found.json` and `report.json`, and fails. docs/parity-workflow.md, "Fuzzing scenarios", has the stub.

    python3 tools/parity_fuzz_probe.py merge LOGS...   # the shards' downloaded logs as one table of groups
"""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# tools/parity.py stops a harness after `--timeout` (2400 s hosted), the game's build included; what the fuzzer
# found by then is in found.json, written before it shrinks.
LIMIT = 1800


def run(slices):
    """Run this shard's slice (PARITY_SHARD=i/n; `slices[i - 1]` is parity_fuzz.py's arguments) and exit 1."""
    index = int(os.environ.get('PARITY_SHARD', '1/1').split('/')[0])
    work = ROOT / '.local' / 'parity-fuzz-probe' / f'shard-{index}'
    kept = work / 'probe-output.txt'
    if kept.exists():
        # tools/parity.py runs a harness that failed for the first time once more: the same output, not a second run.
        print(kept.read_text(encoding='utf-8'))
        sys.exit(1)
    work.mkdir(parents=True, exist_ok=True)
    command = [sys.executable, str(ROOT / 'tools/parity_fuzz.py'), *slices[index - 1], '--work-dir', str(work)]
    try:
        result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, encoding='utf-8',
                                errors='replace', timeout=LIMIT)
        text = result.stdout + f'\nparity_fuzz exited {result.returncode}\n'
    except subprocess.TimeoutExpired as stopped:
        output = stopped.stdout or ''
        text = (output if isinstance(output, str) else output.decode('utf-8', 'replace')) \
            + f'\nparity_fuzz stopped after {LIMIT} s\n'
    for name in ('found.json', 'report.json'):
        if (work / name).exists():
            text += f'\n=== {name} ===\n{(work / name).read_text(encoding="utf-8")}\n'
    logs = sorted(work.glob('batch-*/harness.log'))
    if logs and 'found.json' not in text:  # nothing compared: the last batch's log says why
        text += f'\n=== {logs[-1].relative_to(work)} ===\n{logs[-1].read_text(encoding="utf-8", errors="replace")[-6000:]}\n'
    kept.write_text(text, encoding='utf-8')
    print(text)
    sys.exit(1)


def section(text, name):
    """The JSON a shard's log holds under `=== name ===`, or None."""
    start = text.find(f'=== {name} ===')
    if start < 0:
        return None
    try:
        return json.JSONDecoder().raw_decode(text[start:].split('\n', 1)[1].lstrip())[0]
    except (ValueError, IndexError):
        return None


def cluster(signature, where):
    """A coarser key than the signature, which names the first differing field with its ids and indices: the family,
    the step, the field without them and how it differs (only one engine has it, the tag type, the value)."""
    family, subject, _, field = (signature.split('|', 3) + ['', '', ''])[:4]
    kind = 'value'
    first = where[2][0] if len(where) > 2 and where[2] and isinstance(where[2][0], list) else None
    if first and len(first) > 2:
        field, vanilla, horizon = first[:3]
        number = re.compile(r'(-?[\d.]+?)(?:\.0)?[bsLfd]?')
        if vanilla is None or horizon is None:
            kind = 'only horizon' if vanilla is None else 'only vanilla'
        elif isinstance(vanilla, str) and isinstance(horizon, str) and number.fullmatch(vanilla)                 and number.fullmatch(horizon) and number.fullmatch(vanilla)[1] == number.fullmatch(horizon)[1]:
            kind = 'tag type'
    return f'{family}|{subject}|{re.sub(r"[\[(]\d+[\])]", "[]", str(field))}|{kind}'


def smoke_passed(text):
    """A `--smoke` slice that matched: it writes no found.json and is no lost shard."""
    return 'smoke' in text and 'parity_fuzz exited 0' in text


def merge(paths):
    """The shards' groups as one: per signature its seeds and one seed's first difference. A log without found.json
    is a shard that compared nothing (it hung or crashed): lost, not clean."""
    groups, clusters, totals, lost = {}, {}, {'scenarios': 0, 'invalid': 0, 'refused': 0}, []
    # The artifact holds a shard that failed twice as `NAME.first-run.log` and the retry's `NAME.log`. One counts:
    # the retry when it has a result (the first run may have stopped before its own), else the first run. A smoke
    # slice that passed is a result too.
    shards = {}
    for path in sorted(paths, key=lambda p: str(p).endswith('.first-run.log')):
        shards.setdefault(re.sub(r'(\.first-run)?\.log$', '', Path(path).name), []).append(path)
    for logs in shards.values():
        texts = [Path(path).read_text(encoding='utf-8', errors='replace') for path in logs]
        path, text = next(((p, t) for p, t in zip(logs, texts)
                           if section(t, 'found.json') is not None or smoke_passed(t)),
                          (logs[-1], texts[-1]))
        found = section(text, 'found.json')
        if found is None:
            if not smoke_passed(text):
                lost.append(Path(path).name)
            continue
        for key in totals:
            totals[key] += found[key] if key == 'scenarios' else len(found[key])
        for group in found['groups']:
            entry = groups.setdefault(group['signature'], {'seeds': [], 'where': group['where'][0]})
            entry['seeds'] += group['seeds']
            for where in group['where']:
                clusters.setdefault(cluster(group['signature'], where), []).append(where)
    return {'totals': totals, 'lost': lost, 'clusters': dict(sorted(clusters.items(), key=lambda item: -len(item[1]))),
            'groups': dict(sorted(groups.items(), key=lambda item: -len(item[1]['seeds'])))}


if __name__ == '__main__':
    if len(sys.argv) < 3 or sys.argv[1] != 'merge':
        sys.exit(__doc__)
    merged = merge(sys.argv[2:])
    print(json.dumps({'totals': merged['totals'], 'lost': merged['lost'], 'distinct': len(merged['groups'])}))
    print('clusters (family, step, field, how it differs): seeds, then one seed with its command, vanilla, Horizon')
    for key, members in merged['clusters'].items():
        print(f'{len(members):5d}  {key}  seeds {sorted({m[0] for m in members})[:6]}')
        print(f'         {json.dumps(members[0])[:400]}')
    print('signatures')
    for signature, entry in merged['groups'].items():
        print(f'{len(entry["seeds"]):5d}  {signature}  seeds {sorted(entry["seeds"])[:6]}')
        print(f'         {json.dumps(entry["where"])[:400]}')
