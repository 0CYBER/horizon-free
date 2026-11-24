"""Slices of tools/parity_fuzz.py as one sharded harness, for a hosted run on a `parity-probe/*` branch (#20446).

parity-hosted.yml runs harnesses by name and keeps only the log of one that did not pass. So a probe branch (never
main) adds `tools/test_fuzz_probe_parity.py`, which hands its slices to `run`: each shard runs one, prints the fuzzer's
output and its `found.json` and `report.json`, and fails. docs/parity-workflow.md, "Fuzzing scenarios", has the stub.

    python3 tools/parity_fuzz_probe.py merge LOGS...   # the shards' downloaded logs as one table of groups

parity-fuzz.yml needs no probe branch: its fuzz jobs run `slices` (slices of one family side by side against a game
built by another job) and its summary merges their `found.json` files.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import stat
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# tools/parity.py stops a harness after `--timeout` (2400 s hosted), the game's build included; what the fuzzer
# found by then is in found.json, written before it shrinks.
LIMIT = 1800
FUZZER = ROOT / 'tools/parity_fuzz.py'


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


def staged(work, *parts, limit=64 << 20):
    """The file `parts` name under a slice's folder `work`, as text: only a regular file, with no link on the way to
    it or as it (the game and the oracle write in that folder, and what is staged is uploaded), and no larger than
    `limit`. None otherwise."""
    source = Path(work)
    for part in ('',) + parts:
        source = source / part
        if source.is_symlink():
            return None
    try:
        handle = os.open(source, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_NONBLOCK', 0))
    except OSError:
        return None
    try:
        status = os.fstat(handle)
        if not stat.S_ISREG(status.st_mode) or status.st_nlink != 1 or status.st_size > limit:
            return None
        with os.fdopen(os.dup(handle), 'rb') as file:
            return file.read(limit + 1)[:limit].decode('utf-8', 'replace')
    finally:
        os.close(handle)


# The files a step speaks to the runner through: the slices step's own, not for the game it starts.
STEP_FILES = ('GITHUB_OUTPUT', 'GITHUB_ENV', 'GITHUB_PATH', 'GITHUB_STATE', 'GITHUB_STEP_SUMMARY')


def emptied(folder):
    """`folder` gone, whatever it was: a link is removed, not followed."""
    if folder.is_symlink() or folder.is_file():
        folder.unlink()
    elif folder.exists():
        shutil.rmtree(folder)  # removes the links it holds, not what they name


def complete(found):
    """Whether `found` is a found.json with all that `merge` and its printing read: a slice whose file lacks any of it
    compared nothing that can be counted."""
    return (isinstance(found, dict) and type(found.get('scenarios')) is int
            and all(isinstance(found.get(key), list) for key in ('invalid', 'refused', 'groups'))
            and all(isinstance(group, dict) and isinstance(group.get('signature'), str)
                    and isinstance(group.get('seeds'), list) and all(type(seed) is int for seed in group['seeds'])
                    and isinstance(group.get('where'), list) and group['where']
                    # One seed's place: the seed, its command, and (what `cluster` reads) its differences, each
                    # a named field with vanilla's value and Horizon's. The values are of any kind: text of a tag,
                    # nothing where one engine has no such field, two lists' lengths, a witness's results.
                    and all(isinstance(where, list) and where and type(where[0]) is int
                            and (len(where) < 3 or isinstance(where[2], list)
                                 and all(isinstance(difference, list) and len(difference) >= 3
                                         and isinstance(difference[0], str) for difference in where[2]))
                            for where in group['where'])
                    for group in found['groups']))


def digest(folder):
    """One digest of every file under `folder`: each file's sha256 and path as `sha256sum ./path` prints them, in the
    paths' byte order (parity-fuzz.yml computes the same over what it is about to upload)."""
    lines = [f'{hashlib.sha256(path.read_bytes()).hexdigest()}  ./{path.relative_to(folder).as_posix()}\n'
             for path in folder.rglob('*') if path.is_file()]
    return hashlib.sha256(''.join(sorted(lines, key=lambda line: line[66:].encode())).encode()).hexdigest()


def slices(args):
    """`args.slices` slices of one family side by side (`args.count` seeds each from `args.seed` on, or one slice for
    each of `args.seeds`), each with a work folder and a game home of its own; what each printed and found goes to
    `args.out`/slice-<first seed>/. Exits 1 when a slice compared nothing.

    `args.out` is removed first and made new once every slice has ended (a slice runs the tested game, which could
    have put anything there), and gets only what `staged` reads: each slice's output, its two reports parsed as JSON
    and written again, and the last harness log's tail of a slice without a result. Once staged, also when a slice
    was lost, the step's output `staged` is the digest of what was staged ($GITHUB_OUTPUT, which no slice is given)."""
    firsts = args.seeds or [args.seed + index * args.count for index in range(args.slices)]
    if not firsts:
        sys.exit('no slice to run: nothing would be fuzzed')
    # `args.out` is removed, with all it holds: never the checkout, a folder above it, or the slices' work folders.
    # Each on its own: .local may be a link to a folder elsewhere, and then the work folders are not in the checkout.
    out, root, works = args.out.resolve(), ROOT.resolve(), (ROOT / '.local' / 'parity-fuzz').resolve()
    if out == root or out in root.parents or out == works or out in works.parents or works in out.parents:
        sys.exit(f'--out {args.out} is the checkout, above it or the slices\' work folder: it would be removed')
    if len(set(firsts)) != len(firsts):
        sys.exit(f'a seed is named twice: {firsts}')  # two slices of one seed would share a work folder
    emptied(args.out)
    running = []
    count = 1 if args.seeds else args.count
    for first in firsts:
        work = ROOT / '.local' / 'parity-fuzz' / f'slice-{first}'
        shutil.rmtree(work, ignore_errors=True)
        (work / 'home').mkdir(parents=True)
        command = [sys.executable, str(FUZZER), args.family, '--seed', str(first),
                   '--count', str(count), '--batch', '25', '--shrink-rounds', str(args.shrink_rounds),
                   '--binary', str(args.binary), '--work-dir', str(work)] + (['--ordinary'] if args.ordinary else [])
        log = (work / 'output.txt').open('w', encoding='utf-8')
        # A session of its own: a slice stopped at the limit takes its harness, game and oracle with it.
        process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
                                   env={**{name: value for name, value in os.environ.items() if name not in STEP_FILES},
                                        'HORIZON_HOME': str(work / 'home')})
        running.append((first, work, log, process))
    deadline, lost, ended = time.monotonic() + args.limit, [], {}
    for first, work, log, process in running:
        try:
            process.wait(max(1.0, deadline - time.monotonic()))
            ended[first] = f'exited {process.returncode}'
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            ended[first] = f'stopped after {args.limit} s'  # what it found by then is in found.json, written before it shrinks
        log.close()
    emptied(args.out)
    args.out.mkdir(parents=True)  # new: nothing a slice could have put there
    for first, work, _, _ in running:
        kept = args.out / f'slice-{first}'
        kept.mkdir()
        (kept / 'output.txt').write_text(staged(work, 'output.txt') or '', encoding='utf-8')
        for name in ('found.json', 'report.json'):
            try:
                report = json.loads(staged(work, name) or '')
            except ValueError:
                continue
            if complete(report) if name == 'found.json' else isinstance(report, dict):
                (kept / name).write_text(json.dumps(report), encoding='utf-8')
        logs = sorted(work.glob('batch-*/harness.log'))
        if not (kept / 'found.json').exists():
            lost.append(first)
            if logs:  # nothing compared: the last batch's log says why
                tail = staged(work, logs[-1].parent.name, 'harness.log') or ''
                (kept / 'harness-tail.txt').write_text(tail[-6000:], encoding='utf-8')
        print(f'slice {first}: {ended[first]}, {len(logs)} harness runs' + ('' if first not in lost else ', NO RESULT'),
              flush=True)
    if os.environ.get('GITHUB_OUTPUT'):
        with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as output:
            output.write(f'staged={digest(args.out)}\n')
    sys.exit(f'{len(lost)} slices compared nothing: {lost}' if lost else 0)


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
        elif (isinstance(vanilla, str) and isinstance(horizon, str) and number.fullmatch(vanilla)
              and number.fullmatch(horizon) and number.fullmatch(vanilla)[1] == number.fullmatch(horizon)[1]):
            kind = 'tag type'
    return f'{family}|{subject}|{re.sub(r"[\[(]\d+[\])]", "[]", str(field))}|{kind}'


def smoke_passed(text):
    """A `--smoke` slice that matched: it writes no found.json and is no lost shard."""
    return 'smoke' in text and 'parity_fuzz exited 0' in text


def merge(paths):
    """The shards' groups as one: per signature its seeds and one seed's first difference. A log without found.json
    is a shard that compared nothing (it hung or crashed): lost, not clean."""
    groups, clusters, totals, lost, counted = {}, {}, {'scenarios': 0, 'invalid': 0, 'refused': 0}, [], 0
    # The artifact holds a shard that failed twice as `NAME.first-run.log` and the retry's `NAME.log`. One counts:
    # the retry when it has a result (the first run may have stopped before its own), else the first run. A smoke
    # slice that passed is a result too.
    shards = {}
    for path in sorted(paths, key=lambda p: str(p).endswith('.first-run.log')):
        own = Path(path).name == 'found.json'  # a slice's file itself (parity-fuzz.yml), named by its folder
        shards.setdefault(str(Path(path).parent) if own else re.sub(r'(\.first-run)?\.log$', '', Path(path).name),
                          []).append(path)
    for logs in shards.values():
        texts = [Path(path).read_text(encoding='utf-8', errors='replace') for path in logs]
        texts = ['=== found.json ===\n' + text if Path(path).name == 'found.json' else text
                 for path, text in zip(logs, texts)]
        path, text = next(((p, t) for p, t in zip(logs, texts)
                           if section(t, 'found.json') is not None or smoke_passed(t)),
                          (logs[-1], texts[-1]))
        found = section(text, 'found.json')
        if not complete(found):  # none, or one that lacks what is counted below
            if not smoke_passed(text):
                lost.append(Path(path).name)
            continue
        counted += 1
        for key in totals:
            totals[key] += found[key] if key == 'scenarios' else len(found[key])
        for group in found['groups']:
            entry = groups.setdefault(group['signature'], {'seeds': [], 'where': group['where'][0]})
            entry['seeds'] += group['seeds']
            for where in group['where']:
                clusters.setdefault(cluster(group['signature'], where), []).append(where)
    return {'totals': totals, 'lost': lost, 'counted': counted, 'clusters': dict(sorted(clusters.items(), key=lambda item: -len(item[1]))),
            'groups': dict(sorted(groups.items(), key=lambda item: -len(item[1]['seeds'])))}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='command', required=True)
    merging = commands.add_parser('merge')
    merging.add_argument('logs', nargs='+', type=Path)
    merging.add_argument('--expect', type=int, help='exit 3, after the table, unless this many shards had a whole result')
    run_slices = commands.add_parser('slices')
    run_slices.add_argument('--family', required=True,
                            choices=['blockdata', 'commands', 'items', 'persist', 'retype', 'shulker', 'spawn'])
    run_slices.add_argument('--seed', type=int, default=1, help='the first seed of the first slice')
    run_slices.add_argument('--count', type=int, default=50, help='seeds per slice')
    run_slices.add_argument('--seeds', type=lambda text: [int(seed) for seed in text.split(',')],
                            help='or these seeds, comma-separated: one slice each (a replay, with shrinking)')
    run_slices.add_argument('--slices', type=int, default=8)
    run_slices.add_argument('--shrink-rounds', type=int, default=0)
    run_slices.add_argument('--ordinary', action='store_true')
    run_slices.add_argument('--binary', type=Path, required=True, help='the built Horizon game')
    run_slices.add_argument('--out', type=Path, required=True)
    run_slices.add_argument('--limit', type=int, default=4200, help='seconds before a slice is stopped')
    args = parser.parse_args()
    if args.command == 'slices':
        slices(args)
    merged = merge(args.logs)
    print(json.dumps({'totals': merged['totals'], 'counted': merged['counted'], 'lost': merged['lost'],
                      'distinct': len(merged['groups'])}))
    print('clusters (family, step, field, how it differs): seeds, then one seed with its command, vanilla, Horizon')
    for key, members in merged['clusters'].items():
        print(f'{len(members):5d}  {key}  seeds {sorted({m[0] for m in members})[:6]}')
        print(f'         {json.dumps(members[0])[:400]}')
    print('signatures')
    for signature, entry in merged['groups'].items():
        print(f'{len(entry["seeds"]):5d}  {signature}  seeds {sorted(entry["seeds"])[:6]}')
        print(f'         {json.dumps(entry["where"])[:400]}')
    if args.expect is not None and merged['counted'] != args.expect:
        sys.exit(3)  # the table above is of a part: a slice is missing or its result is not whole


if __name__ == '__main__':
    main()
