#!/usr/bin/env python3
"""Admission of predicate holders by the real 26.3 and Horizon servers.

Cases (each a fresh dedicated server, a fresh flat world and one enabled world pack `prg`):
- `valid`: `prg:target` is a constant true check; `prg:ref` (`all_of`) references it by id. A control: the server must
  reach Ready and serve commands that evaluate the reference.
- `missing`: the same pack, but `prg:ref` references `prg:missing`, which no pack defines. Nothing uses
  `prg:ref`.
- `cycle`: `prg:ref` references `prg:loop`, which references `prg:ref`.
- `single_valid`, `single_missing`, `single_cycle`: the same three, with `terms` one holder alone instead of a list
  (`HolderSetCodec` reads it as a list of one, at `.terms[0]`). The valid one is an `any_of`, which is true only
  when its one term is evaluated.
- `tag_missing`: `prg:ref`'s `terms` names the predicate tag `#prg:none`, which no pack defines.
- Further cases cover tags, inline/named number providers, loot tables, item modifiers,
  advancements and invalid identifiers. Unknown fields and deferred container loot are controls.
- `--reload` starts with working data, installs each invalid candidate, checks that the old
  predicates and load function survive, then repairs the pack and checks that the repair is live.

Each run keeps the complete server log, an archive of the exact pack bytes and independent facts:
Ready (the `Done (` line), service (an RCON-free console probe), timeout, how the probe ended the
process, the exit code and the reaping. A case that never reaches Ready is never sent a command.
"""
import argparse
import datetime
import hashlib
import io
import json
import shutil
import subprocess
import sys
import tarfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import lab  # noqa: E402
PARITY_MANUAL = "a proof run by hand: --kind java --output DIR, then --kind native --output DIR2 --binary BIN"  # not a harness: tools/parity.py leaves it out

META = '{"pack":{"description":"predicate reference graph","min_format":[121,0],"max_format":[121,0]}}\n'
# 26.3 dispatches a predicate on `type`; a named reference is a holder: an id where a predicate goes.
TARGET = {'type': 'minecraft:all_of', 'terms': []}
CASES = {
    'valid': {'target': TARGET, 'ref': {'type': 'minecraft:all_of', 'terms': ['prg:target']}},
    'missing': {'target': TARGET, 'ref': {'type': 'minecraft:all_of', 'terms': ['prg:missing']}},
    # Two predicates that reference each other; nothing uses either.
    'cycle': {'target': TARGET, 'ref': {'type': 'minecraft:all_of', 'terms': ['prg:loop']},
              'loop': {'type': 'minecraft:inverted', 'term': 'prg:ref'}},
    'single_valid': {'target': TARGET, 'ref': {'type': 'minecraft:any_of', 'terms': 'prg:target'}},
    'single_missing': {'target': TARGET, 'ref': {'type': 'minecraft:all_of', 'terms': 'prg:missing'}},
    'single_cycle': {'target': TARGET, 'ref': {'type': 'minecraft:any_of', 'terms': 'prg:loop'},
                     'loop': {'type': 'minecraft:inverted', 'term': 'prg:ref'}},
    'tag_missing': {'target': TARGET, 'ref': {'type': 'minecraft:all_of', 'terms': '#prg:none'}},
}
VALID_CASES = {'valid', 'single_valid', 'tag_valid', 'tag_nested_valid', 'tag_optional_valid',
               'unrelated_fields', 'deferred_loot_reference'}
EXTRA_FILES = {}


def add_case(name, files, predicate=None):
    """Each fixture starts with a working control predicate, independent of the failure."""
    CASES[name] = {'target': TARGET, 'ref': predicate or {'type': 'minecraft:all_of', 'terms': ['prg:target']}}
    EXTRA_FILES[name] = {f'data/prg/{path}.json': value for path, value in files.items()}


add_case('tag_valid', {'tags/predicate/yes': {'values': ['prg:target']}},
         {'type': 'minecraft:any_of', 'terms': '#prg:yes'})
add_case('tag_nested_valid', {'tags/predicate/yes': {'values': ['#prg:nested']},
                             'tags/predicate/nested': {'values': ['prg:target']}},
         {'type': 'minecraft:any_of', 'terms': '#prg:yes'})
add_case('tag_optional_valid', {'tags/predicate/yes': {'values': [
    {'id': 'prg:absent', 'required': False}, 'prg:target']}},
         {'type': 'minecraft:any_of', 'terms': '#prg:yes'})
add_case('tag_required_missing', {'tags/predicate/yes': {'values': ['prg:missing']}},
         {'type': 'minecraft:all_of', 'terms': '#prg:yes'})
add_case('tag_required_unused', {'tags/predicate/yes': {'values': ['prg:missing']}})
add_case('tag_incomplete_unused', {'tags/predicate/yes': {'values': ['#prg:none', 'prg:missing']}})
# Invalid tag JSON is ignored as a whole, not partly admitted as a required element reference.
add_case('tag_invalid_unused', {'tags/predicate/yes': {'values': ['Not An Id', 'prg:missing']}})
VALID_CASES.add('tag_invalid_unused')
add_case('tag_predicate_cycle', {'tags/predicate/yes': {'values': ['prg:ref']}},
         {'type': 'minecraft:all_of', 'terms': '#prg:yes'})
add_case('invalid_identifier', {}, {'type': 'minecraft:inverted', 'term': 'Not An Id'})
add_case('legacy_reference', {}, {'condition': 'minecraft:reference', 'name': 'prg:missing'})
add_case('removed_reference_type', {}, {'type': 'minecraft:reference', 'name': 'prg:missing'})
add_case('legacy_predicate_list', {}, [{'type': 'minecraft:inverted', 'term': 'prg:missing'}])
add_case('legacy_modifier_list', {'item_modifier/change': [
    {'type': 'minecraft:set_count', 'count': 1, 'condition': 'prg:missing'}]})
add_case('inline_modifier_list_valid', {'loot_table/table': {'modifier': [
    {'type': 'minecraft:set_count', 'count': 1, 'condition': 'prg:target'}]}})
VALID_CASES.add('inline_modifier_list_valid')
add_case('loot_condition_missing', {'loot_table/table': {'pools': [
    {'rolls': 1, 'entries': [], 'condition': 'prg:missing'}]}})
add_case('loot_inline_missing', {'loot_table/table': {'pools': [{'rolls': 1, 'entries': [
    {'type': 'minecraft:loot_table', 'value': {'pools': [
        {'rolls': 1, 'entries': [], 'condition': 'prg:missing'}]}}]}]}})
add_case('modifier_condition_missing', {'item_modifier/change': {
    'type': 'minecraft:set_count', 'count': 1, 'condition': 'prg:missing'}})
add_case('modifier_nested_missing', {'item_modifier/change': {
    'type': 'minecraft:filtered', 'item_filter': {}, 'on_pass': {
        'type': 'minecraft:set_count', 'count': 1, 'condition': 'prg:missing'}}})
add_case('number_condition_missing', {}, {'type': 'minecraft:random_chance', 'chance': {
    'type': 'minecraft:conditional', 'condition': 'prg:missing', 'on_true': 1.0}})
add_case('named_number_condition_missing', {'context_float_provider/chance': {
    'type': 'minecraft:conditional', 'condition': 'prg:missing', 'on_true': 1.0}})
add_case('number_cross_cycle', {'context_float_provider/chance': {
    'type': 'minecraft:conditional', 'condition': 'prg:ref', 'on_true': 1.0}},
    {'type': 'minecraft:float_value_check', 'value': 'prg:chance', 'test': 1.0})
# RandomChance's codec binds chance, but its validate method does not visit it in 26.3.
# Keep this unused: evaluating it would recurse even though Vanilla admits the pack.
add_case('unchecked_chance_cycle', {
    'predicate/unused': {'type': 'minecraft:random_chance', 'chance': 'prg:chance'},
    'context_float_provider/chance': {
        'type': 'minecraft:conditional', 'condition': 'prg:unused', 'on_true': 1.0}})
VALID_CASES.add('unchecked_chance_cycle')
add_case('int_range_missing', {}, {'type': 'minecraft:int_value_check', 'value': 1, 'test': {
    'type': 'minecraft:conditional', 'condition': 'prg:missing', 'on_true': 1}})
add_case('slot_cycle', {'slot_source/source': {'type': 'minecraft:group', 'terms': 'prg:source'}})
add_case('modifier_tag_missing', {'item_modifier/change': {
    'type': 'minecraft:sequence', 'functions': '#prg:none'}})
add_case('uniform_valid', {'context_int_provider/number': {
    'type': 'minecraft:uniform', 'min': 1, 'max': 2}})
VALID_CASES.add('uniform_valid')
add_case('modifier_cycle', {'item_modifier/change': {
    'type': 'minecraft:sequence', 'functions': ['prg:change']}})
add_case('loot_cycle', {'loot_table/table': {'pools': [{'rolls': 1, 'entries': [
    {'type': 'minecraft:loot_table', 'value': 'prg:table'}]}]}})
add_case('advancement_missing', {'advancement/check': {
    'criteria': {'check': {'trigger': 'minecraft:tick', 'conditions': {'player': 'prg:missing'}}}}})
add_case('advancement_nested_missing', {'advancement/check': {
    'criteria': {'check': {'trigger': 'minecraft:bred_animals', 'conditions': {
        'parent': {'type': 'minecraft:inverted', 'term': 'prg:missing'}}}}}})
add_case('unrelated_fields', {}, {'type': 'minecraft:all_of', 'terms': [],
                                'unrelated': {'condition': 'prg:missing'}})
# Setting a container's future loot table binds a holder but does not invoke it during validation.
add_case('deferred_loot_reference', {
    'loot_table/table': {'modifier': {'type': 'minecraft:set_loot_table', 'loot_table_id': 'prg:table'}}})
# Commands for a Ready server; the last one says the marker only when the reference evaluates true.
SERVICE = ['datapack list enabled', 'execute if predicate prg:target run say PRG_TARGET_TRUE',
           'execute if predicate prg:ref run say PRG_REF_TRUE', 'say PRG_SERVICE_END']


def sha(b):
    return hashlib.sha256(b).hexdigest()


def encode(value):
    return (json.dumps(value, separators=(',', ':')) + '\n').encode()


def pack_files(case, generation=None):
    files = {f'data/prg/predicate/{name}.json': encode(value) for name, value in CASES[case].items()}
    files.update({path: encode(value) for path, value in EXTRA_FILES.get(case, {}).items()})
    files['pack.mcmeta'] = META.encode()
    if generation:
        files['data/minecraft/tags/function/load.json'] = encode({'values': ['prg:loaded']})
        files['data/prg/function/loaded.mcfunction'] = f'say PRG_LOADED_{generation}\n'.encode()
    return files


def replace_pack(pack, previous, current):
    for rel in previous.keys() - current.keys():
        (pack / rel).unlink()
    for rel, data in current.items():
        (pack / rel).parent.mkdir(parents=True, exist_ok=True)
        (pack / rel).write_bytes(data)


def reload_probe(process, lines, pack, out, case, initial):
    """Observe completion through the load function or reload failure, never an arbitrary delay."""
    def exchange(commands, finished):
        offset = len(lines)
        process.stdin.write('\n'.join(commands) + '\n')
        process.stdin.flush()
        deadline = time.monotonic() + 60
        while process.poll() is None and time.monotonic() < deadline:
            replies = ''.join(lines[offset:])
            if finished(replies):
                return replies
            time.sleep(.1)
        raise AssertionError(('reload probe did not complete', commands, ''.join(lines[offset:])))

    candidate = pack_files(case, 'INVALID')
    # A failed reload must not publish even the otherwise valid part of the candidate.
    false = encode({'type': 'minecraft:any_of', 'terms': []})
    candidate['data/prg/predicate/target.json'] = false
    repaired = pack_files('valid', 'REPAIRED')
    repaired['data/prg/predicate/target.json'] = false
    result = {}
    for label, files in [('candidate', candidate), ('repaired', repaired)]:
        with tarfile.open(out / f'{label}.tar', 'w') as archive:
            for path, data in files.items():
                entry = tarfile.TarInfo(path)
                entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
    replace_pack(pack, initial, candidate)
    result['rejected_reply'] = exchange(['reload'], lambda text:
        'PRG_LOADED_INVALID' in text or (
            ('Failed to execute reload' in text or 'Reload failed' in text)
            and all(diagnostic in text for diagnostic in DIAGNOSTICS[case])))
    result['retained_reply'] = exchange(['function prg:loaded', *SERVICE],
                                        lambda text: '[Server] PRG_SERVICE_END' in text)
    replace_pack(pack, candidate, repaired)
    result['repaired_reply'] = exchange(['reload'], lambda text: '[Server] PRG_LOADED_REPAIRED' in text)
    result['repaired_service'] = exchange([
        'execute unless predicate prg:target run say PRG_REPAIRED_TARGET_FALSE',
        'execute unless predicate prg:ref run say PRG_REPAIRED_REF_FALSE',
        'say PRG_REPAIR_END',
    ], lambda text: '[Server] PRG_REPAIR_END' in text)
    # Restore the original bytes after observation so the sealed fixture checks remain meaningful.
    replace_pack(pack, repaired, initial)
    return result


def run(kind, case, parent, binary, timeout, reload=False):
    out = parent / f'{kind}-{case}'
    out.mkdir(parents=True, exist_ok=False)
    game = out / 'game'
    pack = game / 'world/datapacks/prg'
    files = pack_files('valid', 'OLD') if reload else pack_files(case)
    replace_pack(pack, {}, files)
    with tarfile.open(out / 'inputs.tar', 'w') as archive:
        archive.add(pack, arcname='prg')
    port = lab.free_port()
    props = {'server-ip': '127.0.0.1', 'server-port': port, 'online-mode': 'false', 'level-name': 'world',
             'level-seed': 121, 'level-type': 'minecraft:flat',
             'generator-settings': json.dumps({'layers': [{'block': 'minecraft:bedrock', 'height': 1}],
                                               'biome': 'minecraft:plains', 'features': False, 'lakes': False,
                                               'structure_overrides': []}, separators=(',', ':')),
             'generate-structures': 'false', 'initial-enabled-packs': 'vanilla,file/prg',
             'view-distance': 2, 'simulation-distance': 2, 'max-tick-time': -1,
             'pause-when-empty-seconds': -1, 'enable-rcon': 'false'}
    (game / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (game / 'eula.txt').write_text('eula=true\n')
    java = shutil.which('java')
    command = ([java, '-Xms256M', '-Xmx2G', '-jar', str(lab.GAME / 'jars/server.jar'), '--nogui']
               if kind == 'java' else
               [str(binary), '--resources', str(lab.RESOURCES), '--dedicated', '--nogui'])
    env = lab.game_env(HORIZON_HOME=out / 'homes/horizon', HORIZON_LAUNCHER_HOME=out / 'homes/launcher',
                       XDG_DATA_HOME=out / 'homes/xdg-data', XDG_CONFIG_HOME=out / 'homes/xdg-config')
    start = time.monotonic()
    facts = dict(start_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), kind=kind, case=case,
                 command=command, timeout_seconds=timeout,
                 inputs={rel: sha(data) for rel, data in files.items()},
                 inputs_tar_sha256=sha((out / 'inputs.tar').read_bytes()),
                 server_properties_sha256=sha((game / 'server.properties').read_bytes()),
                 reload=reload, ready=False, service=False, timed_out=False, probe_termination=[], return_code=None,
                 reaped=False, ready_seconds=None, exit_seconds=None, console_requests=[],
                 console_reply_lines=[])
    if kind == 'java':
        facts['java_version'] = subprocess.run([java, '-version'], capture_output=True, text=True).stderr
    lines = []
    log = (out / 'server.log').open('w')
    p = subprocess.Popen(command, cwd=game, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='replace',
                         start_new_session=True)

    def pump():
        for line in p.stdout:
            lines.append(line)
            log.write(line)
            log.flush()
    thread = threading.Thread(target=pump)
    thread.start()
    try:
        while time.monotonic() - start < timeout and p.poll() is None:
            if any('Done (' in line and 'For help' in line for line in lines):
                facts['ready'] = True
                facts['ready_seconds'] = time.monotonic() - start
                break
            time.sleep(.1)
        if not facts['ready'] and p.poll() is None:
            facts['timed_out'] = True
        if facts['ready']:
            offset = len(lines)
            facts['console_requests'] = SERVICE
            for text in SERVICE:
                p.stdin.write(text + '\n')
            p.stdin.flush()
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline and p.poll() is None:
                if any(line.rstrip().endswith('[Server] PRG_SERVICE_END') for line in lines[offset:]):
                    facts['service'] = True
                    break
                time.sleep(.1)
            time.sleep(.5)
            facts['console_reply_lines'] = lines[offset:].copy()
            if reload:
                try:
                    facts['reload_observation'] = reload_probe(p, lines, pack, out, case, files)
                finally:
                    # Keep failure artifacts too if a process crashes or never acknowledges reload.
                    current = {str(path.relative_to(pack)): None for path in pack.rglob('*') if path.is_file()}
                    replace_pack(pack, current, files)
    except Exception as e:
        facts['probe_error'] = repr(e)
    finally:
        if p.poll() is None:
            facts['probe_termination'].append(dict(action='console stop', seconds=time.monotonic() - start))
            try:
                p.stdin.write('stop\n')
                p.stdin.flush()
                p.wait(timeout=60)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                facts['probe_termination'].append(dict(action='kill', seconds=time.monotonic() - start))
                p.kill()
                p.wait(timeout=10)
        else:
            p.wait()
        facts['return_code'] = p.returncode
        facts['reaped'] = True
        facts['exit_seconds'] = time.monotonic() - start
        thread.join(timeout=10)
        assert not thread.is_alive()
        log.close()
        facts['log_sha256'] = sha((out / 'server.log').read_bytes())
        for rel, data in files.items():
            assert (pack / rel).read_bytes() == data, rel
        (out / 'outcome.json').write_text(json.dumps(facts, indent=2) + '\n')
    print(kind, case, json.dumps({k: facts[k] for k in ['ready', 'service', 'timed_out', 'return_code', 'reaped',
                                                       'ready_seconds', 'exit_seconds']}), flush=True)
    return facts


def run_integrated(case, parent, binary, timeout):
    """Exercise single player's real integrated-server startup, also used before opening LAN."""
    out = parent / f'native-{case}'
    out.mkdir(parents=True, exist_ok=False)
    game = out / 'game'
    pack = game / 'saves/prg/datapacks/prg'
    files = pack_files(case)
    replace_pack(pack, {}, files)
    script = out / 'commands.jsonl'
    script.write_bytes(b''.join(encode({'op': 'cmd', 'cmd': command, 'source': 'console'})
                                for command in SERVICE))
    result = out / 'commands-out.jsonl'
    env = lab.game_env(
        HORIZON_HOME=out / 'homes/horizon', HORIZON_LAUNCHER_HOME=out / 'homes/launcher',
        XDG_DATA_HOME=out / 'homes/xdg-data', XDG_CONFIG_HOME=out / 'homes/xdg-config',
        HORIZON_COMMAND_SCRIPT=script, HORIZON_COMMAND_OUTPUT=result,
        HORIZON_API_DISCOVERY='http://127.0.0.1:9/', HORIZON_PLAYER_NAME='PredicateProbe',
        HORIZON_HARNESS_LOADED_FIXTURE='1', HORIZON_HARNESS_TIMEOUT=str(int(timeout)))
    command = [str(binary), '--resources', str(lab.RESOURCES),
               '--world', 'prg', '--seed', '121', '--radius', '2', '--headless']
    with (out / 'server.log').open('w', encoding='utf-8') as log:
        completed = subprocess.run(command, cwd=game, env=env, stdout=log, stderr=subprocess.STDOUT,
                                   timeout=timeout)
    facts = {'kind': 'native', 'case': case, 'integrated': True, 'command': command,
             'return_code': completed.returncode, 'inputs': {path: sha(data) for path, data in files.items()},
             'log_sha256': sha((out / 'server.log').read_bytes()),
             'service': result.read_text(encoding='utf-8') if result.exists() else None}
    (out / 'outcome.json').write_text(json.dumps(facts, indent=2) + '\n')
    print('integrated', case, 'exit', completed.returncode, 'served', result.exists(), flush=True)
    return facts


# The Java 26.3 diagnostics each invalid case must log (as substrings; their order and log prefixes are
# not part of the contract).
REFUSAL = ("Failed to load datapacks, can't proceed with server load. "
           'You can either fix your datapacks or reset to vanilla with --safeMode')
DIAGNOSTICS = {
    'missing': ['Unbound values in registry ResourceKey[minecraft:root / minecraft:predicate]: [prg:missing]'],
    'cycle': ['Found loot table element validation problem in {prg:loop@minecraft:predicate}.term'
              '->{prg:ref@minecraft:predicate}.terms[0]: prg:loop of type minecraft:predicate is recursively called',
              'Found loot table element validation problem in {prg:ref@minecraft:predicate}.terms[0]'
              '->{prg:loop@minecraft:predicate}.term: prg:ref of type minecraft:predicate is recursively called'],
    'tag_missing': ['Unbound tags in registry ResourceKey[minecraft:root / minecraft:predicate]: [prg:none]'],
}
# A lone holder is reported as the list form's first element.
DIAGNOSTICS['single_missing'] = DIAGNOSTICS['missing']
DIAGNOSTICS['single_cycle'] = DIAGNOSTICS['cycle']
for _case in ('loot_condition_missing', 'loot_inline_missing', 'modifier_condition_missing',
              'modifier_nested_missing', 'number_condition_missing', 'named_number_condition_missing',
              'advancement_missing', 'advancement_nested_missing', 'int_range_missing'):
    DIAGNOSTICS[_case] = DIAGNOSTICS['missing']
DIAGNOSTICS['tag_required_missing'] = DIAGNOSTICS['missing']
DIAGNOSTICS['tag_required_unused'] = DIAGNOSTICS['missing']
DIAGNOSTICS['tag_incomplete_unused'] = DIAGNOSTICS['missing']
DIAGNOSTICS['tag_predicate_cycle'] = ['of type minecraft:predicate is recursively called']
DIAGNOSTICS['invalid_identifier'] = ['Not An Id']
DIAGNOSTICS['legacy_reference'] = ['No key type']
DIAGNOSTICS['removed_reference_type'] = ['minecraft:reference']
DIAGNOSTICS['legacy_predicate_list'] = ['prg:missing']
DIAGNOSTICS['legacy_modifier_list'] = ['prg:missing']
DIAGNOSTICS['modifier_cycle'] = ['prg:change of type minecraft:item_modifier is recursively called']
DIAGNOSTICS['loot_cycle'] = ['prg:table of type minecraft:loot_table is recursively called']
DIAGNOSTICS['number_cross_cycle'] = ['of type minecraft:predicate is recursively called',
                                    'of type minecraft:context_float_provider is recursively called']
DIAGNOSTICS['slot_cycle'] = ['prg:source of type minecraft:slot_source is recursively called']
DIAGNOSTICS['modifier_tag_missing'] = [
    'Unbound tags in registry ResourceKey[minecraft:root / minecraft:item_modifier]: [prg:none]']


def check(folder, facts):
    """The contract: a valid world serves the reference; an invalid one ends by itself before Ready."""
    expected = pack_files('valid', 'OLD') if facts.get('reload') else pack_files(facts['case'])
    assert facts['inputs'] == {path: sha(data) for path, data in expected.items()}, 'fixture changed since run'
    log = (folder / 'server.log').read_bytes()
    assert sha(log) == facts['log_sha256'], 'sealed log changed'
    if facts.get('integrated'):
        if facts['case'] in VALID_CASES:
            assert facts['return_code'] == 0 and facts['service'], facts
            for marker in ('PRG_TARGET_TRUE', 'PRG_REF_TRUE', 'PRG_SERVICE_END'):
                assert marker in facts['service'], facts['service']
        else:
            assert facts['return_code'] != 0 and facts['service'] is None, facts
            for diagnostic in [*DIAGNOSTICS[facts['case']], REFUSAL]:
                assert diagnostic in log.decode('utf-8', 'replace'), diagnostic
        return
    assert facts['reaped'] and not facts['timed_out'] and not facts.get('probe_error'), facts
    replies = ''.join(facts['console_reply_lines'])
    if facts.get('reload'):
        assert facts['ready'] and facts['service'] and facts['return_code'] == 0, facts
        observed = facts['reload_observation']
        assert 'PRG_LOADED_INVALID' not in observed['rejected_reply'], observed['rejected_reply']
        for diagnostic in DIAGNOSTICS[facts['case']]:
            assert diagnostic in observed['rejected_reply'], diagnostic
        for marker in ('[Server] PRG_TARGET_TRUE', '[Server] PRG_REF_TRUE', '[Server] PRG_LOADED_OLD'):
            assert marker in observed['retained_reply'], observed['retained_reply']
        for marker in ('[Server] PRG_REPAIRED_TARGET_FALSE', '[Server] PRG_REPAIRED_REF_FALSE'):
            assert marker in observed['repaired_service'], observed['repaired_service']
        return
    if facts['case'] in VALID_CASES:
        assert facts['ready'] and facts['service'], facts
        for marker in ('[Server] PRG_TARGET_TRUE', '[Server] PRG_REF_TRUE', 'file/prg (world)'):
            assert marker in replies, (marker, replies)
        return
    text = log.decode('utf-8', 'replace')
    assert not facts['ready'] and not facts['service'], 'an invalid world reached Ready'
    assert not facts['probe_termination'] and not facts['console_requests'], 'the probe ended or drove it'
    assert facts['return_code'] == 0, facts['return_code']
    assert 'Done (' not in text
    for line in [*DIAGNOSTICS[facts['case']], REFUSAL]:
        assert line in text, line


def main():
    a = argparse.ArgumentParser()
    a.add_argument('--kind', choices=['java', 'native'], required=True)
    a.add_argument('--output', type=Path, required=True)
    a.add_argument('--binary', type=Path)
    a.add_argument('--timeout', type=float, default=240)
    a.add_argument('--cases', default=','.join(CASES))
    a.add_argument('--verify-only', action='store_true', help='check sealed outcomes without running')
    a.add_argument('--reload', action='store_true', help='reject invalid candidates and retain live data')
    a.add_argument('--integrated', action='store_true', help='native single player/LAN startup via the real server')
    o = a.parse_args()
    if o.integrated and (o.kind != 'native' or o.reload):
        a.error('--integrated requires --kind native and cannot be combined with --reload')
    failures = []
    for case in o.cases.split(','):
        if o.reload and case in VALID_CASES:
            continue
        folder = o.output.resolve() / f'{o.kind}-{case}'
        if o.verify_only:
            facts = json.loads((folder / 'outcome.json').read_text())
        elif o.integrated:
            facts = run_integrated(case, o.output.resolve(), o.binary, o.timeout)
        else:
            facts = run(o.kind, case, o.output.resolve(), o.binary, o.timeout, o.reload)
        try:
            check(folder, facts)
            print(o.kind, case, 'contract: ok')
        except AssertionError as e:
            failures.append(case)
            print(o.kind, case, 'contract: FAILED', str(e)[:300])
    return 1 if failures else 0


if __name__ == '__main__':
    sys.exit(main())
