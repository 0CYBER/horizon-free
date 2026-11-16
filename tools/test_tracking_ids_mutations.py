"""Exclusive-checkout runtime mutation proof; set HORIZON_TEST_RESOURCES first.

Each case omits one production call (not a test/model). Compile failures do not
count as kills. Originals are restored even on interruption. Logs stay in target.
"""
import os
from pathlib import Path
import re
import signal
import subprocess


def terminate(signum, _frame):
    raise SystemExit(128 + signum)


signal.signal(signal.SIGTERM, terminate)
if not os.environ.get('HORIZON_TEST_RESOURCES'):
    raise SystemExit('Set HORIZON_TEST_RESOURCES')
lan = Path('crates/horizon-client/src/game/network/lan_entities.rs')
dragon = Path('crates/horizon-client/src/game/mobs/dragon.rs')
originals = {p: p.read_text() for p in (lan, dragon)}
subprocess.run(['git', 'diff', 'HEAD', '--exit-code', '--', str(lan), str(dragon)], check=True)
logs = Path('target/tracking-id-proof')
logs.mkdir(parents=True, exist_ok=True)
command = ['cargo', 'test', '-p', 'horizon-client', '--lib']
def check_production(label):
    for test in ['tracking_ids_', 'late_dragon_ids_']:
        result = subprocess.run(command + [test, '--', '--ignored'], capture_output=True, text=True)
        (logs / (label + '-' + test + '.log')).write_text(result.stdout + result.stderr)
        assert result.returncode == 0 and '0 passed' not in result.stdout, result.stdout + result.stderr


cases = []
start = originals[lan].index('    pub(super) fn lan_assign_ids(')
end = originals[lan].index('\n    }', start)
body = originals[lan][start:end]
calls = list(re.finditer(r'(?:super::entity::assign_stored_id|assign_entity_id)\([^;]+\);', body))
assert len(calls) == 12, len(calls)
names = ['tnt', 'falling', 'projectile', 'trident', 'rocket', 'crystal',
         'bolt', 'spawner', 'potion', 'eye', 'cloud', 'fireball']
for name, call in zip(names, calls):
    cases.append((name, lan, start + call.start(), start + call.end(), ';',
                  'tracking_ids_preserve_all_family_interleaving'))
for name, expr in [('falling', 'Some(&f.attachment_lifetime)'),
                   ('projectile', 'Some(&p.attachment_lifetime)')]:
    offset = originals[lan].index(expr, start)
    cases.append((name + '-slot', lan, offset, offset + len(expr), 'None',
                  'tracking_ids_refuse_bound_zero_owners'))
start = originals[dragon].index('    pub(super) fn assign_dragon_and_crystal_ids(')
end = originals[dragon].index('\n    }', start)
body = originals[dragon][start:end]
for name, expr in [('dragon', 'super::entity::assign_stored_id(&mut self.next_entity, &mut d.id, None);'),
                   ('parts', 'self.next_entity += 8;'),
                   ('late-crystal', 'super::entity::assign_stored_id(&mut self.next_entity, &mut c.id, None);')]:
    offset = originals[dragon].index(expr, start, end)
    cases.append((name, dragon, offset, offset + len(expr), ';',
                  'late_dragon_ids_reserve_parts_before_crystals'))
check_production('baseline')
try:
    for name, path, start, end, replacement, test in cases:
        path.write_text(originals[path][:start] + replacement + originals[path][end:])
        result = subprocess.run(command + [test, '--', '--ignored', '--nocapture'],
                                capture_output=True, text=True)
        output = result.stdout + result.stderr
        (logs / (name + '.log')).write_text(output)
        if result.returncode == 0 or 'test result: FAILED' not in output:
            raise SystemExit(f'{name}: no runtime kill; see log')
        print(f'{name}: runtime kill', flush=True)
        path.write_text(originals[path])
finally:
    for path, content in originals.items():
        path.write_text(content)
check_production('restored')
print(f'{len(cases)} runtime kills; production restored and focused tests passed')
