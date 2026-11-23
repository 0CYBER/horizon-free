#!/usr/bin/env python3
"""Differential scenario fuzzer: generated command scripts run in vanilla and Horizon, compared exactly (#12498).

Each scenario comes from a seed: an area far from the player, forceloaded; a setup and an actor (summon with NBT,
a block entity with NBT, a dispenser, a drop, an explosion, a piston); time steps; optionally an unload and reload of
the area's chunk, which saves and loads it through each engine's chunk and entity storage; and reads (`/data get`).
An item stack is a step of its own (`stack`): one item and component patch, built by a command or loaded from NBT.
Every scenario syncs both engines' randoms first (docs/parity-workflow.md, "Randoms that repeat"). A batch runs
through tools/test_command_exec_parity.py (`--groups-file`), each read is compared exactly (SNBT parsed, so a
difference names its field), failures are shrunk while the same field still differs, and grouped by family, subject
type and first differing field. docs/parity-workflow.md, "Fuzzing scenarios", says how to run it.
"""
import argparse
import json
import os
import random
import re
import shutil
import subprocess
import sys
from pathlib import Path

try:
    from . import lab, test_command_exec_parity as harness
except ImportError:
    import lab
    import test_command_exec_parity as harness

ROOT = Path(__file__).resolve().parents[1]

# Far from the player (8.5 230 8.5) and its view distance, above any terrain; one chunk per scenario, three apart.
AREA_X, AREA_Z, FLOOR_Y = 4096, 4096, 300
RELOAD_TICKS = 60  # enough for both engines to save and drop a chunk whose forced ticket went (a guard checks it)

# What a reload's witnesses expect from vanilla: `/execute unless loaded` succeeds once the chunk went, and
# `/execute if entity` reports 0 entities once the entity is unloaded. Vanilla can keep a chunk's entities loaded
# after the chunk reads as unloaded (#16772), so any other vanilla result makes the scenario invalid: nothing was
# saved and loaded to compare. A Horizon result that differs from vanilla's is a divergence.
GUARDS = {'chunk': [[True, 1]], 'entity': [[False, 0]]}


def set_alike(vanilla, horizon):
    """Absolute game times differ by the engines' clocks: only whether each is set (not 0 or -1) is compared."""
    unset = lambda v: v is None or str(v).rstrip('L') in ('0', '-1')  # noqa: E731
    return vanilla is not None and horizon is not None and unset(vanilla) == unset(horizon)


# Differences that are not parity bugs: (regular expression on the normalised path, reason, test on the two values).
# The two engines' worlds start the script at different game times (they settle in a different number of ticks).
ALLOWED = [
    (r'next_weather_age', 'absolute game time (CopperGolem.nextWeatheringTick)', set_alike),
    (r'server_data\.state_updating_resumes_at', 'absolute game time (VaultServerData.pauseStateUpdatingUntil)',
     set_alike),
    (r'LastGossipDecay', 'absolute game time (Villager.maybeDecayGossip)', set_alike),
]

# Differences already filed, by signature (family|subject|live/reloaded|first field). A pattern matches
# the whole signature; the report lists a match under its issue, and only the rest as new. An entry that
# matches nothing in a round is reported as "not observed" (not evidence of a fix), so the table can be
# revisited without pretending a bug was fixed.
KNOWN = [
    # The issue's own example, and the shape a table entry takes. Add the other filed groups
    # (#16100 freezing twice, #16128 crystal Rotation, the reload timing family) from a report's
    # signatures; each entry is a whole-signature regex, and an entry that matches nothing is
    # reported as "not observed", not as fixed.
    (r'persist\|trident\|live\|damage', 14823),
]


def area(slot):
    return AREA_X + 48 * slot, FLOOR_Y, AREA_Z


# ---------------------------------------------------------------------------------------------------------------
# SNBT as /data get prints it (TextComponentTagVisitor): compounds become dicts, lists lists, scalars their text.

TOKEN = re.compile(r'\s*(?:("(?:[^"\\]|\\.)*"|\'(?:[^\'\\]|\\.)*\')|([{}\[\]:,;])|([^\s{}\[\]:,;"\']+))')


def parse_snbt(text):
    tokens = [m.group(1) or m.group(2) or m.group(3) for m in TOKEN.finditer(text)]
    pos = 0

    def value():
        nonlocal pos
        token = tokens[pos]
        pos += 1
        if token == '{':
            out = {}
            while tokens[pos] != '}':
                key = tokens[pos].strip('"\'')
                assert tokens[pos + 1] == ':', text
                pos += 2
                out[key] = value()
                if tokens[pos] == ',':
                    pos += 1
            pos += 1
            return out
        if token == '[':
            out = []
            if pos + 1 < len(tokens) and tokens[pos + 1] == ';':
                out.append(tokens[pos] + ';')
                pos += 2
            while tokens[pos] != ']':
                out.append(value())
                if tokens[pos] == ',':
                    pos += 1
            pos += 1
            return out
        return token

    result = value()
    assert pos == len(tokens), text
    return result


# Lists whose order vanilla doesn't fix, compared sorted by this key: AttributeMap keeps its instances in an
# Object2ObjectOpenHashMap keyed by Holder.Reference, which hashes by identity, so the saved order changes run to run.
UNORDERED = {'attributes': 'id'}


def diff(a, b, path=''):
    key = UNORDERED.get(path.rsplit('.', 1)[-1])
    if key and isinstance(a, list) and isinstance(b, list):
        a, b = (sorted(v, key=lambda e: str(e.get(key)) if isinstance(e, dict) else str(e)) for v in (a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return [d for key in sorted(set(a) | set(b))
                for d in (diff(a.get(key), b.get(key), f'{path}.{key}' if path else key))]
    if isinstance(a, list) and isinstance(b, list):
        out = [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f'{path}[{i}]')]
        return out + ([(f'{path}[#]', len(a), len(b))] if len(a) != len(b) else [])
    return [] if a == b else [(path, a, b)]


DATA = re.compile(r'^(.*) has the following (?:entity|block|storage) data: (.*)$', re.S)


def read_data(record):
    """A /data get record as (holder name, parsed data), or None when it reported no data."""
    text = ''.join(span[0] for message in record['messages'] for span in message)
    match = DATA.match(text)
    if not match:
        return None
    try:
        return match.group(1), parse_snbt(match.group(2))
    except (AssertionError, IndexError):
        return match.group(1), match.group(2)  # not SNBT this parser knows: compared as text


def normalise(path):
    return re.sub(r'\[\d+\]', '[]', path)


def allowed(path, vanilla, horizon):
    return any(re.fullmatch(pattern, normalise(path)) and test(vanilla, horizon) for pattern, _, test in ALLOWED)


def compare(vanilla, rust):
    """The differences between two records of one read, as [(field, vanilla, horizon)], allowlisted ones left out."""
    if vanilla['cmd'].startswith('/data get '):
        left, right = read_data(vanilla), read_data(rust)
        if left is None or right is None:
            if (left is None) != (right is None) or harness.compare(vanilla, rust):
                return [('<result>', harness.render(vanilla)[:300], harness.render(rust)[:300])]
            return []
        found = ([('<name>', left[0], right[0])] if left[0] != right[0] else []) + diff(left[1], right[1])
        return [d for d in found if not allowed(*d)]
    kind = harness.compare(vanilla, rust)
    return [(f'<{kind}>', harness.render(vanilla)[:300], harness.render(rust)[:300])] if kind else []


# ---------------------------------------------------------------------------------------------------------------
# Scenarios: a seed, a family and steps. Offsets are relative to the scenario's area; NBT values are SNBT text.

def snbt(value):
    if isinstance(value, dict):
        return '{' + ','.join(f'{k}:{snbt(v)}' for k, v in value.items()) + '}'
    if isinstance(value, list):
        return '[' + ','.join(snbt(v) for v in value) + ']'
    return value


def uuid_words(slot, n):
    return [0x46555a5a, slot, n, 1]  # "FUZZ", then unique within a batch


def uuid_text(words):
    return '-'.join([f'{words[0] & 0xffffffff:08x}', f'{words[1] >> 16:04x}', f'{words[1] & 0xffff:04x}',
                     f'{words[2] >> 16:04x}', f'{words[2] & 0xffff:04x}{words[3]:08x}'])


def patch_nbt(step):
    """A stack step's component patch as saved NBT: {component: SNBT value}, None removing one (`"!id":{}`)."""
    return '{' + ','.join(f'"!{key}":{{}}' if value is None else f'"{key}":{value}'
                          for key, value in (step.get('nbt') or {}).items()) + '}'


def stack_text(step, nbt, head=''):
    """A stack step's item with its patch: `id[patch]` as a command takes it, or the NBT a save holds."""
    patch = step.get('nbt') or {}
    if not nbt:
        parts = ','.join(f'!{key}' if value is None else f'{key}={value}' for key, value in patch.items())
        return step['item'] + (f'[{parts}]' if patch else '')
    return f'{{{head}id:"{step["item"]}",count:{step["count"]}' + (f',components:{patch_nbt(step)}' if patch else '') + '}'


def stack_command(step, pos):
    """How a stack step reaches slot 0 of the container at its place: `/item replace ... with`; the container placed
    with the stack in its NBT; `/data merge` into a placed container; `/item modify` (set_components) of a stack
    already there; an inline loot table whose entry sets the components."""
    how = step['how']
    if how == 'with':
        return f'/item replace block {pos} container.0 with {stack_text(step, False)} {step["count"]}'
    if how == 'nbt':
        return f'/setblock {pos} {step["block"]}{{Items:[{stack_text(step, True, "Slot:0b,")}]}}'
    if how == 'merge':
        return f'/data merge block {pos} {{Items:[{stack_text(step, True, "Slot:0b,")}]}}'
    set_components = f'{{type:"minecraft:set_components",components:{patch_nbt(step)}}}'
    if how == 'modify':
        return f'/item modify block {pos} container.0 {set_components}'
    if how == 'loot':
        return (f'/loot replace block {pos} container.0 loot {{pools:[{{rolls:1,entries:[{{type:"minecraft:item",'
                f'name:"{step["item"]}",functions:[{set_components},{{type:"minecraft:set_count",'
                f'count:{step["count"]}}}]}}]}}]}}')
    raise ValueError(how)


def sync_op(seed, slot):
    """Both engines' randoms from one state; entity ids are unique in a batch, the same modulo 4 in every slot."""
    return {'op': 'sync_randoms', 'uniquifier': seed, 'entity_id': 10_000_000 + 10_000 * slot, 'level_seed': seed}


def render(scenario, slot):
    """The scenario's harness operations in this slot, each command op tagged with its step index."""
    x0, y0, z0 = area(slot)

    def at(offset):
        return f'{x0 + offset[0]} {y0 + offset[1]} {z0 + offset[2]}'

    def cmd(text, step, record=True):
        op = {'op': 'cmd', 'cmd': text, 'step': step}
        if not record:
            op['record'] = False
        return op

    chunk = [[x0 >> 4, z0 >> 4]]
    seed = scenario['seed']
    ops = [cmd(f'/forceload add {x0} {z0}', -1, False), {'op': 'wait_chunks', 'chunks': chunk},
           cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 - 1} {z0 + 15} minecraft:stone', -1, False),
           sync_op(seed, slot)]
    player_at = scenario.get('player_at')
    if player_at is not None:
        # A survival player can be targeted. Resistance keeps repeated scenarios from depleting their health.
        ops += [cmd('/gamemode survival @a', -1, False),
                cmd('/effect give @a minecraft:resistance 10 4 true', -1, False),
                cmd(f'/tp @a {at(player_at)}', -1, False)]
    for i, step in enumerate(scenario['steps']):
        kind = step['k']
        if kind == 'summon':
            nbt = {'UUID': '[I;' + ','.join(map(str, uuid_words(slot, step['n']))) + ']', **step['nbt']}
            text = snbt(nbt)  # `{iuN}`: the UUID of the entity summoned as n=N, as an int array
            text = re.sub(r'\{iu(\d+)\}', lambda m: '[I;' + ','.join(map(str, uuid_words(slot, int(m.group(1))))) + ']', text)
            ops.append(cmd(f'/summon {step["type"]} {at(step["at"])} {text}', i))
        elif kind == 'block':
            if step.get('merge'):  # into the block already there: `BlockEntity.loadWithComponents` of the merged tag
                ops.append(cmd(f'/data merge block {at(step["at"])} {snbt(step.get("nbt") or {})}', i))
            else:
                ops.append(cmd(f'/setblock {at(step["at"])} {step["block"]}{snbt(step.get("nbt") or {}) if step.get("nbt") else ""}', i))
        elif kind == 'stack':
            ops.append(cmd(stack_command(step, at(step['at'])), i))
        elif kind == 'cmd':
            text = step['cmd']
            if step.get('uuids'):  # `{uN}`: the UUID of the entity summoned as n=N
                text = re.sub(r'\{u(\d+)\}', lambda m: uuid_text(uuid_words(slot, int(m.group(1)))), text)
            # `{N}`: the step's Nth place. Not str.format: a command's SNBT has braces of its own.
            op = cmd(re.sub(r'\{(\d+)\}', lambda m: at(step['at'][int(m.group(1))]), text), i)
            if 'guard' in step:
                op['guard'] = step['guard']
            ops.append(op)
        elif kind == 'tick':
            ops.append({'op': 'tick', 'n': step['n']})
        elif kind == 'dispense':
            ops.append({'op': 'crafter_tick', 'pos': [x0 + step['at'][0], y0 + step['at'][1], z0 + step['at'][2]]})
        elif kind == 'sync':
            # A second sync at the tick a redstone dispenser fires: the ticks since the scenario's own sync leave
            # the engines' randoms apart, so the fire would draw different values (#16834).
            ops.append(sync_op(seed, slot))
        elif kind == 'reload':
            summoned = [uuid_text(uuid_words(slot, s['n'])) for s in scenario['steps'][:i] if s['k'] == 'summon']
            # `allow_chunk_io`: the oracle's ticks have no spare time by default, so `ChunkMap.processUnloads` would
            # leave the chunk in memory, unsaved, with its scheduled ticks still counting.
            ops += [cmd(f'/forceload remove {x0} {z0}', i, False),
                    {'op': 'tick', 'n': RELOAD_TICKS, 'allow_chunk_io': True},
                    {**cmd(f'/execute unless loaded {x0} {y0} {z0}', i), 'guard': 'chunk'},
                    *({**cmd(f'/execute if entity {uuid}', i), 'guard': 'entity'} for uuid in summoned),
                    cmd(f'/forceload add {x0} {z0}', i, False), {'op': 'wait_chunks', 'chunks': chunk}]
        elif kind == 'read_entity':
            path = f' {step["path"]}' if step.get('path') else ''
            ops.append({**cmd(f'/data get entity {uuid_text(uuid_words(slot, step["n"]))}{path}', i), 'read': True})
        elif kind == 'read_near':
            ox, oy, oz = (a + b for a, b in zip(area(slot), step['at']))
            path = f' {step["path"]}' if step.get('path') else ''
            ops.append({**cmd(f'/data get entity @e[type={step["type"]},x={ox}.5,y={oy},z={oz}.5,distance=..6,'
                              f'sort=nearest,limit=1]{path}', i), 'read': True})
            ops.append({**cmd(f'/execute if entity @e[type={step["type"]},x={ox}.5,y={oy},z={oz}.5,distance=..12]', i),
                        'read': True})
        elif kind == 'read_block':
            path = f' {step["path"]}' if step.get('path') else ''
            ops.append({**cmd(f'/data get block {at(step["at"])}{path}', i), 'read': True})
        else:
            raise ValueError(kind)
    if player_at is not None:
        ops += [cmd('/gamemode creative @a', -1, False),
                cmd('/effect clear @a minecraft:resistance', -1, False),
                cmd('/effect clear @a minecraft:levitation', -1, False)]
    if player_at is not None or any('/tp @a' in s.get('cmd', '') for s in scenario['steps']):
        ops.append(cmd('/tp @a 8.5 230 8.5', -1, False))
    ops += [cmd(f'/kill @e[type=!player,x={x0},y={y0 - 8},z={z0},dx=16,dy=48,dz=16]', -1, False),
            cmd(f'/fill {x0} {y0 - 1} {z0} {x0 + 15} {y0 + 12} {z0 + 15} minecraft:air', -1, False),
            cmd(f'/forceload remove {x0} {z0}', -1, False)]
    return ops


def subject(scenario, step):
    """The entity or block type a read looks at, or the label its family gave the step."""
    if step.get('subject'):
        return step['subject']
    if step['k'] == 'read_entity':
        return next((s['type'] for s in scenario['steps'] if s['k'] == 'summon' and s['n'] == step['n']), '?')
    if step['k'] == 'read_near':
        return step['type']
    if step['k'] == 'read_block':
        placed = [s['block'] for s in scenario['steps'] if s['k'] == 'block' and s['at'] == step['at']]
        return re.sub(r'[\[{].*', '', placed[-1]) if placed else '?'
    return '?'


# ---------------------------------------------------------------------------------------------------------------
# Running batches through the command-exec harness.

def witness(family, op, vanilla, rust):
    """Require the scenario's precondition in vanilla and compare the corresponding Horizon result."""
    guard = op['guard']
    if guard == 'shulker_hit':
        field, expected, reason = '<hit>', [[True, 26]], 'vanilla bullet did not deal four damage'
    else:
        field, expected, reason = '<unload>', GUARDS[guard], f'vanilla kept the {guard} loaded'
    invalid = f'{reason}: {vanilla["results"]}' if vanilla['results'] != expected else None
    failure = ((f'{family}|{guard}|{field}', op['cmd'], [(field, vanilla['results'], rust['results'])])
               if rust['results'] != vanilla['results'] else None)
    return invalid, failure


class Runner:
    def __init__(self, work, binary, size):
        self.work, self.binary, self.size, self.batches = work, binary, size, 0

    def run(self, scenarios):
        """Per scenario: {'failures': [(signature, read, differences)], 'invalid': reason} or {'crash': log}."""
        if len(scenarios) > self.size:
            return [r for start in range(0, len(scenarios), self.size) for r in self.run(scenarios[start:start + self.size])]
        if not scenarios:
            return []
        self.batches += 1
        batch = self.work / f'batch-{self.batches:03d}'
        batch.mkdir(parents=True, exist_ok=True)
        rendered = [render(s, slot) for slot, s in enumerate(scenarios)]
        groups = [[f'fuzz {s["family"]} {s["seed"]}', [{k: v for k, v in op.items() if k not in ('step', 'read', 'guard')}
                                                       for op in ops]] for s, ops in zip(scenarios, rendered)]
        (batch / 'groups.json').write_text(json.dumps(groups), encoding='utf-8')
        print(f'batch {self.batches}: {len(scenarios)} scenarios', flush=True)
        command = [sys.executable, str(ROOT / 'tools/test_command_exec_parity.py'), '--allow-gaps',
                   '--groups-file', str(batch / 'groups.json'), '--work-dir', str(batch / 'run')]
        if self.binary:  # otherwise the harness builds the game (the first batch; later ones find it built)
            command += ['--binary', str(self.binary)]
        result = subprocess.run(command, cwd=ROOT, capture_output=True, encoding='utf-8', errors='replace',
                                # The game's script deadline (600 s by default) is for the fixed corpus.
                                env={**os.environ, 'HORIZON_HARNESS_TIMEOUT': str(30 * len(scenarios) + 600)})
        (batch / 'harness.log').write_text(result.stdout + result.stderr, encoding='utf-8')
        if result.returncode:
            if len(scenarios) == 1:
                return [{'crash': (result.stdout + result.stderr)[-2000:]}]
            half = len(scenarios) // 2  # find the scenario that stops the run
            return self.run(scenarios[:half]) + self.run(scenarios[half:])
        records = [[json.loads(line) for line in (batch / 'run' / f'{side}.jsonl').open(encoding='utf-8')]
                   for side in ('vanilla', 'rust')]
        for world in ('vanilla', 'rust', 'classes'):  # tens of MB a batch; the records and logs stay
            shutil.rmtree(batch / 'run' / world, ignore_errors=True)
        out, position = [], 0
        for scenario, ops in zip(scenarios, rendered):
            entry = {'failures': [], 'invalid': None, 'refused': None, 'reads': []}
            for op in ops:
                if op['op'] != 'cmd' or op.get('record') is False:
                    continue
                vanilla, rust = records[0][position], records[1][position]
                position += 1
                step = scenario['steps'][op['step']]
                if step.get('setup') and not any(result[0] for result in vanilla['results']):
                    # Not invalid: a setup Horizon accepts and vanilla refuses is a difference, compared below.
                    entry['refused'] = op['cmd']
                if op.get('guard'):
                    invalid, failure = witness(scenario['family'], op, vanilla, rust)
                    if invalid:
                        entry['invalid'] = invalid
                    if failure:
                        entry['failures'].append(failure)
                    continue
                if op.get('read'):
                    entry['reads'].append((op['cmd'], vanilla, rust))
                differences = compare(vanilla, rust)
                if differences:
                    what = subject(scenario, step) if op.get('read') else step.get('subject') or op['cmd'].split()[0]
                    reloaded = any(s['k'] == 'reload' for s in scenario['steps'][:op['step']])
                    first = normalise(differences[0][0])
                    entry['failures'].append((f'{scenario["family"]}|{what}|{"reloaded" if reloaded else "live"}|{first}',
                                              op['cmd'], differences))
            out.append(entry)
        assert position == len(records[0]) == len(records[1]), (position, len(records[0]), len(records[1]))
        return out


def signatures(result):
    """A scenario's divergences: each failing read's signature once, or 'crash'. One read can't hide another's."""
    if 'crash' in result:
        return ['crash']
    return [] if result['invalid'] else list(dict.fromkeys(failure[0] for failure in result['failures']))

def classify(groups):
    """(known, new, not_observed): signatures by known issue, the groups no entry names, and entries whose
    pattern did not match this round.

    `groups` is {signature: members}; the result reuses the members so the caller can shrink only new ones.
    A pattern that matches nothing only means the round did not exercise it: that is not evidence its bug is
    fixed, so the caller reports it as "not observed", never as "fixed".
    """
    known, new, matched = {}, {}, set()
    for signature, members in groups.items():
        issue = None
        for pattern, candidate in KNOWN:
            if re.fullmatch(pattern, signature):
                matched.add(pattern)
                if issue is None:
                    issue = candidate
        if issue is None:
            new[signature] = members
        else:
            known.setdefault(issue, []).append(signature)
    not_observed = [(issue, pattern) for pattern, issue in KNOWN if pattern not in matched]
    return known, new, not_observed


# ---------------------------------------------------------------------------------------------------------------
# Shrinking: drop steps, NBT keys and ticks while the same divergence stays.

def reduce(scenario, edits):
    """The scenario with these edits: ('drop', i) a step, ('half', i) a tick count, ('nbt', i, key) an NBT key."""
    steps = []
    for i, step in enumerate(scenario['steps']):
        if ('drop', i) in edits:
            continue
        if ('half', i) in edits:
            step = {**step, 'n': step['n'] // 2}
        gone = {e[2] for e in edits if e[:2] == ('nbt', i)}
        if gone:
            step = {**step, 'nbt': {k: v for k, v in step['nbt'].items() if k not in gone}}
        steps.append(step)
    return {**scenario, 'steps': steps}


def edits(scenario):
    for i, step in enumerate(scenario['steps']):
        if step['k'] == 'sync' or step.get('guard'):
            # Each reduced scenario must still check the same preconditions.
            continue
        if step['k'] == 'tick' and step['n'] > 1:
            yield ('half', i)
        yield ('drop', i)
        yield from (('nbt', i, key) for key in step.get('nbt') or {})


def runnable(scenario):
    """A candidate still reads something, and activates a dispenser only where one was placed."""
    steps = scenario['steps']
    placed = {tuple(s['at']) for s in steps if s['k'] == 'block' and s['block'].startswith('dispenser')}
    return (any(s['k'].startswith('read') for s in steps)
            and all(tuple(s['at']) in placed for s in steps if s['k'] == 'dispense'))


def shrink(runner, failing, rounds):
    """{signature: (scenario, result)} -> the same, each scenario as small as this many rounds could make it.

    A round runs every single edit of each scenario, then each scenario with all its edits that kept its signature
    applied together; if that loses the signature, the smallest single edit that kept it stands.
    """
    current = dict(failing)
    for _ in range(rounds):
        tried = [(sig, edit, reduce(s, {edit})) for sig, (s, _) in current.items() for edit in edits(s)]
        tried = [t for t in tried if runnable(t[2])]
        kept, best = {}, {}
        for (sig, edit, candidate), result in zip(tried, runner.run([t[2] for t in tried])):
            if sig in signatures(result):
                kept.setdefault(sig, set()).add(edit)
                if sig not in best or len(json.dumps(candidate)) < len(json.dumps(best[sig][0])):
                    best[sig] = (candidate, result)
        if not kept:
            break
        combined = [(sig, reduce(current[sig][0], e)) for sig, e in kept.items() if len(e) > 1]
        for (sig, candidate), result in zip(combined, runner.run([c for _, c in combined])):
            if sig in signatures(result):
                best[sig] = (candidate, result)
        current.update(best)
    return current


# ---------------------------------------------------------------------------------------------------------------
# Command line

# Fixed scenarios that match today: a chest with items and a bare armor stand, read after an unload and reload.
SMOKE = [{'family': 'smoke', 'seed': 1, 'steps': [
    {'k': 'block', 'block': 'chest', 'at': [7, 0, 7], 'nbt': {'Items': '[{Slot:0b,id:"minecraft:stone",count:3}]'}},
    {'k': 'summon', 'type': 'armor_stand', 'at': [3.5, 0, 3.5], 'n': 0, 'nbt': {}},
    {'k': 'reload'}, {'k': 'read_block', 'at': [7, 0, 7]}, {'k': 'read_entity', 'n': 0}]}]


def main():
    try:
        from . import parity_fuzz_scenarios as families
    except ImportError:
        import parity_fuzz_scenarios as families
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('family', choices=sorted(families.FAMILIES), nargs='?', default='persist')
    parser.add_argument('--seed', type=int, default=1, help='first scenario seed')
    parser.add_argument('--count', type=int, default=40, help='scenarios to generate')
    source = parser.add_mutually_exclusive_group()
    source.add_argument('--replay', type=Path, help='run the scenarios of this JSON file instead of generating them')
    source.add_argument('--smoke', action='store_true', help='fixed scenarios that must match: the tool works')
    parser.add_argument('--batch', type=int, default=40, help='scenarios per harness run')
    parser.add_argument('--shrink-rounds', type=int, default=6)
    parser.add_argument('--ordinary', action='store_true',
                        help='items, blockdata, commands: only what a command or a played world gives')
    parser.add_argument('--binary', type=Path, help='an already built Horizon game')
    parser.add_argument('--work-dir', type=Path, default=lab.LOCAL / 'parity-fuzz')
    args = parser.parse_args()
    work = args.work_dir.resolve()
    work.mkdir(parents=True, exist_ok=True)
    runner = Runner(work, args.binary and args.binary.resolve(strict=True), args.batch)
    if args.smoke or args.replay:
        scenarios = SMOKE if args.smoke else json.loads(args.replay.read_text(encoding='utf-8'))
    else:
        families.parity_fuzz_data.EDGES = not args.ordinary
        families.prepare(args.family, runner, work)
        scenarios = [families.FAMILIES[args.family](random.Random(seed), seed)
                     for seed in range(args.seed, args.seed + args.count)]
    results = runner.run(scenarios)
    failing = [(s, r) for s, r in zip(scenarios, results) if signatures(r)]
    invalid = [(s['seed'], r['invalid']) for s, r in zip(scenarios, results) if r.get('invalid')]
    refused = [(s['seed'], r['refused']) for s, r in zip(scenarios, results) if r.get('refused')]
    print(f'{len(scenarios)} scenarios: {len(failing)} diverge, {len(invalid)} invalid, vanilla refused the setup of '
          f'{len(refused)}', flush=True)
    if args.smoke:
        empty = [cmd for r in results for cmd, vanilla, _ in r.get('reads', []) if vanilla['results'][:1] != [[True, 1]]]
        if failing or invalid or empty:
            sys.exit(f'smoke scenarios diverge: {[s["seed"] for s, _ in failing]} invalid: {invalid} '
                     f'empty reads: {empty}; see {work}')
        return
    groups = {}
    for scenario, result in failing:
        for sig in signatures(result):
            groups.setdefault(sig, []).append((scenario, result))
    known, new_groups, not_observed = classify(groups)
    # Before shrinking, which can outlast a runner's time limit: every group with its seeds, and each member's first
    # differing command with its first difference (what tells two causes in one group apart).
    found = [{'signature': sig, 'seeds': [s['seed'] for s, _ in members],
              'where': [[s['seed'], *next(([cmd, d[:2]] for g, cmd, d in r.get('failures', []) if g == sig),
                                          [r.get('crash', '')[-300:]])] for s, r in members[:40]]}
             for sig, members in sorted(groups.items(), key=lambda g: -len(g[1]))]
    lab.write_json(work / 'found.json', {'scenarios': len(scenarios), 'invalid': invalid, 'refused': refused,
                                         'groups': found})
    for group in found:
        print(f'found {len(group["seeds"]):3d}  {group["signature"]}  seeds {group["seeds"][:8]}', flush=True)
    # One bug, one shrink: each new group's smallest scenario stands for it.
    shrunk = shrink(runner, {sig: min(members, key=lambda m: len(json.dumps(m[0])))
                             for sig, members in new_groups.items()}, args.shrink_rounds)
    report = {'scenarios': len(scenarios), 'invalid': invalid, 'refused': refused, 'diverging': len(failing),
              'known': [{'issue': issue, 'signatures': signatures} for issue, signatures in sorted(known.items())],
              'not_observed': [{'issue': issue, 'pattern': pattern} for issue, pattern in not_observed],
              # Each diverging seed's signatures, to compare runs of the same seeds before and after a change.
              'by_seed': {s['seed']: signatures(r) for s, r in failing}, 'groups': []}
    for sig, members in sorted(new_groups.items(), key=lambda g: -len(g[1])):
        scenario, result = shrunk[sig]
        report['groups'].append({
            'signature': sig, 'seeds': [s['seed'] for s, _ in members], 'scenario': scenario,
            'commands': [op['cmd'] if op['op'] == 'cmd' else json.dumps({k: v for k, v in op.items() if k != 'step'})
                         for op in render(scenario, 0)],
            'differences': result.get('crash') or [[cmd, d[:8]] for s, cmd, d in result['failures'] if s == sig][:3]})
    lab.write_json(work / 'report.json', report)
    print(f'{len(failing)} diverging scenarios in {len(groups)} groups ({runner.batches} harness runs):')
    for entry in report['known']:
        print(f'  known  #{entry["issue"]}: {", ".join(entry["signatures"])}')
    for entry in report['not_observed']:
        print(f'  not observed this round (not evidence of a fix): #{entry["issue"]} ({entry["pattern"]})')
    for group in report['groups']:
        print(f'  {len(group["seeds"]):3d}  {group["signature"]}')
    print(f'report: {work / "report.json"}')


if __name__ == '__main__':
    main()
