"""Run from a clean checkout with HORIZON_TEST_RESOURCES pointing to prepared data.

This exclusively owns the three production sources while running: do not build,
edit or switch branches concurrently. Each mutation is restored in finally.
Logs stay locally under target/stored-id-proof; no game resources are copied.
"""
from dataclasses import dataclass
from pathlib import Path
import os
import signal
import subprocess


def terminate(signum, _frame):
    raise SystemExit(128 + signum)


signal.signal(signal.SIGTERM, terminate)
if not os.environ.get("HORIZON_TEST_RESOURCES"):
    raise SystemExit("Set HORIZON_TEST_RESOURCES to prepared game data first")


@dataclass(frozen=True)
class Mutation:
    name: str
    file: str
    before: str
    after: str
    occurrence: int
    test: str

root = Path.cwd()
artifacts = root / 'target' / 'stored-id-proof'
artifacts.mkdir(parents=True, exist_ok=True)
subprocess.run(['git', 'diff', 'HEAD', '--exit-code', '--',
                'crates/horizon-client/src/game/modding/native_entities.rs',
                'crates/horizon-client/src/game/blocks/nether_portal.rs',
                'crates/horizon-client/src/game/entities/entity.rs'], check=True)
native = 'crates/horizon-client/src/game/modding/native_entities.rs'
portal = 'crates/horizon-client/src/game/blocks/nether_portal.rs'
entity = 'crates/horizon-client/src/game/entities/entity.rs'
# Each case changes one actual expression; repeated expression positions are explicit.
cases = [
    Mutation(
        'native-tnt-selection', native,
        '.tnts.get_mut(deferred.0)',
        '.tnts.get_mut(0)',
        0, 'native_roots_keep_the_loaded_id',
    ),
    Mutation(
        'native-rocket-selection', native,
        '.rockets.get_mut(deferred.2)',
        '.rockets.get_mut(0)',
        0, 'native_roots_keep_the_loaded_id',
    ),
    Mutation(
        'native-falling-selection', native,
        '.falling.update(deferred.1,',
        '.falling.update(0,',
        0, 'native_roots_keep_the_loaded_id',
    ),
    Mutation(
        'pending-falling-slot', native,
        '&mut f.id,\n                Some(&f.attachment_lifetime)',
        '&mut f.id,\n                None',
        0, 'pending_assignment_refuses_a_bound_zero',
    ),
    Mutation(
        'pending-projectile-slot', native,
        '&mut p.id,\n                Some(&p.attachment_lifetime)',
        '&mut p.id,\n                None',
        0, 'pending_assignment_refuses_a_bound_zero',
    ),
    Mutation(
        'assigned-bound', entity,
        'debug_assert!(*id != 0 || lifetime.is_none_or(|slot| !slot.is_bound()));',
        'debug_assert!(lifetime.is_none_or(|slot| !slot.is_bound()));',
        0, 'pending_ids_keep_counter_order',
    ),
    Mutation(
        'invalid-falling-root', native,
        '"minecraft:falling_block" if deferred.1 < self.falling.len()',
        '"minecraft:falling_block"',
        0, 'native_roots_keep_the_loaded_id',
    ),
    Mutation(
        'slot-guard', entity,
        'debug_assert!(*id != 0 || lifetime.is_none_or(|slot| !slot.is_bound()));',
        'let _ = lifetime;',
        0, 'pending_assignment_refuses_a_bound_zero',
    ),
    Mutation(
        'allocate-delegate', entity,
        'super::lan_entities::assign_entity_id(next, id)',
        '0',
        0, 'pending_ids_keep_counter_order',
    ),
    Mutation(
        'native-tnt', native,
        'super::entity::assign_stored_id(&mut self.next_entity, &mut e.id, None)',
        '0',
        0, 'native_roots_keep_the_loaded_id',
    ),
    Mutation(
        'native-rocket', native,
        'super::entity::assign_stored_id(&mut self.next_entity, &mut e.id, None)',
        '0',
        1, 'native_roots_keep_the_loaded_id',
    ),
    Mutation(
        'native-falling', native,
        'super::entity::assign_stored_id(\n                            &mut self.next_entity,\n                            &mut f.id,\n                            Some(&f.attachment_lifetime),\n                        )',
        '0',
        0, 'native_roots_keep_the_loaded_id',
    ),
    Mutation(
        'pending-tnt', native,
        'super::entity::assign_stored_id(&mut self.next_entity, &mut t.id, None);',
        'let _ = t;',
        0, 'pending_ids_keep_counter_order',
    ),
    Mutation(
        'pending-trident', native,
        'super::entity::assign_stored_id(&mut self.next_entity, &mut t.id, None);',
        'let _ = t;',
        1, 'pending_ids_keep_counter_order',
    ),
    Mutation(
        'pending-falling', native,
        'super::entity::assign_stored_id(\n                &mut self.next_entity,\n                &mut f.id,\n                Some(&f.attachment_lifetime),\n            );',
        'let _ = f;',
        0, 'pending_ids_keep_counter_order',
    ),
    Mutation(
        'pending-projectile', native,
        'super::entity::assign_stored_id(\n                &mut self.next_entity,\n                &mut p.id,\n                Some(&p.attachment_lifetime),\n            );',
        'let _ = p;',
        0, 'pending_ids_keep_counter_order',
    ),
    Mutation(
        'pending-rocket', native,
        'super::entity::assign_stored_id(&mut self.next_entity, &mut r.id, None);',
        'let _ = r;',
        0, 'pending_ids_keep_counter_order',
    ),
    Mutation(
        'portal-trident', portal,
        'super::entity::assign_stored_id(&mut self.next_entity, &mut t.id, None);',
        'let _ = t;',
        0, 'portal_ids_follow_only',
    ),
    Mutation(
        'portal-rocket', portal,
        'super::entity::assign_stored_id(&mut self.next_entity, &mut r.id, None);',
        'let _ = r;',
        0, 'portal_ids_follow_only',
    ),
]
for case in cases:
    name, before = case.name, case.before
    path = root / case.file
    original = path.read_text()
    found = -1
    for _ in range(case.occurrence + 1):
        found = original.find(before, found + 1)
        assert found >= 0, name
    try:
        path.write_text(original[:found] + case.after + original[found + len(before):])
        result = subprocess.run(['cargo', 'test', '-p', 'horizon-client', '--lib', case.test, '--', '--ignored'], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        output = result.stdout
        (artifacts / f'pr1e1-mutation-{name}.log').write_text(output)
        assert result.returncode == 101 and 'test result: FAILED.' in output and 'panicked at' in output, (name, output[-4000:])
        print(f'{name}: production mutation killed', flush=True)
    finally:
        path.write_text(original)
for test in ('pending_ids_keep_counter_order', 'native_roots_keep_the_loaded_id', 'portal_ids_follow_only', 'pending_assignment_refuses_a_bound_zero', 'joins_follow_insertion_order'):
    result = subprocess.run(['cargo', 'test', '-p', 'horizon-client', '--lib', test, '--', '--ignored'], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (artifacts / f'pr1e1-restored-{test}.log').write_text(result.stdout)
    assert result.returncode == 0, (test, result.stdout[-4000:])
print('Restored production source: all five focused regressions pass', flush=True)
