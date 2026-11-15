"""Runtime transfer proof in an exclusive checkout; set HORIZON_TEST_RESOURCES.

Compile errors are not kills. Each mutant changes one production site; originals
are restored even on interruption. Logs are private build output under target.
"""
import argparse
import os
from pathlib import Path
import signal
import subprocess


def terminate(signum, _frame):
    raise SystemExit(128 + signum)


signal.signal(signal.SIGTERM, terminate)
if not os.environ.get('HORIZON_TEST_RESOURCES'):
    raise SystemExit('Set HORIZON_TEST_RESOURCES')
join = Path('crates/horizon-client/src/join_seq.rs')
travel = Path('crates/horizon-client/src/game/world/server_travel.rs')
levels = Path('crates/horizon-client/src/game/world/levels.rs')
originals = {p: p.read_text() for p in (join, travel, levels)}
logs = Path('target/transfer-lifetime-proof')
logs.mkdir(parents=True, exist_ok=True)
command = ['cargo', 'test', '-p', 'horizon-client', '--lib', '--no-default-features']
removal = 'transfer_removal_retires_before_returning_the_vehicle'
arrival = 'transfer_arrival_rebinds_only_bound_owners_with_the_same_id'
cases = [
    ('omit-removal-retire', join, 'entity.attachment_lifetime().retire();', ';', removal),
    ('wrong-arrival-epoch', levels, 'level.epoch(), id',
     'mod_api::WorldEpoch::new(level.epoch().get() + 1), id', arrival),
    ('wrong-destination-index', levels, 'self.attachment_indexes.entry(self.dim)',
     'self.attachment_indexes.entry(source)', arrival),
    ('keep-source-join', travel,
     'v.set_join_seq(horizon_client::join_seq::JoinSeq::UNJOINED);', ';', arrival),
]
for name, family in [('b', 'boats'), ('c', 'carts'), ('m', 'mobs')]:
    line = f'self.rebind_transferred_entity(source, id, &mut {name});'
    cases.append((f'omit-arrival-{name}', travel, line, ';', arrival))
    line = f'self.{family}.push({name});'
    cases.append((f'clone-arrival-{name}', travel, line,
                  f'self.{family}.push({name}.clone());', arrival))


def run(label, test, ignored=False, kill=False):
    args = command + [test, '--', '--nocapture'] + (['--ignored'] if ignored else [])
    result = subprocess.run(args, capture_output=True, text=True)
    output = result.stdout + result.stderr
    (logs / (label + '.log')).write_text(output)
    if kill:
        assert result.returncode != 0 and 'test result: FAILED' in output, output
    else:
        assert result.returncode == 0 and '1 passed' in output, output


def baseline(label):
    run(label + '-removal', removal)
    run(label + '-arrival', arrival, ignored=True)


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--only', choices=[case[0] for case in cases])
selected = parser.parse_args().only
if selected:
    cases = [case for case in cases if case[0] == selected]
baseline('baseline')
try:
    for label, path, old, new, test in cases:
        content = originals[path]
        # The epoch expression also appears in other helpers: target this method only.
        start = content.index('pub(super) fn rebind_transferred_entity') if path == levels else 0
        offset = content.index(old, start)
        path.write_text(content[:offset] + new + content[offset + len(old):])
        run(label, test, ignored=test == arrival, kill=True)
        print(label + ': runtime kill', flush=True)
        path.write_text(content)
finally:
    for path, content in originals.items():
        path.write_text(content)
baseline('restored')
print(f'{len(cases)} runtime kills; restored tests passed')
