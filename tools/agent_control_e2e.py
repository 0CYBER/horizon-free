#!/usr/bin/env python3
"""End-to-end check of agent control: play Horizon with horizon-ctl.

A script creates a world through the title menus, finds a tree it can walk to, breaks a log,
checks the inventory, crafts planks in the inventory screen, places one and reads it back. It
runs through the CLI (one horizon-ctl process per command) and through `horizon-ctl mcp` with a
small JSON-RPC client, and saves a screenshot of each run. The ui scenario drives the menus by
widget id instead: the title screen, the options (a slider set by id) and the inventory's slots;
the options scenario reads and sets options.txt values by key; the events scenario waits for
states (wait-for) and reads the event log; the consume scenario eats and drinks consumables (a
golden apple, milk, honey, chorus fruit) and checks their effects, remainders and teleport; the
pick scenario middle-clicks slots (AbstractContainerScreen's cloning): a full stack in creative, from
the hotbar and from an item tab, and nothing in survival.

    python3 tools/agent_control_e2e.py                 # both modes
    python3 tools/agent_control_e2e.py --mode mcp      # one of them
    python3 tools/agent_control_e2e.py --bin-dir target/fast --out /tmp/agent-e2e
    python3 tools/agent_control_e2e.py --scenario ui   # menus by widget id only (a minute)

Needs vscreen on Linux (the desktop); on macOS the game renders offscreen, so it also runs in a
locked session or over SSH. It needs the prepared resources in .local/horizon-client and a build of
horizon-client and horizon-ctl (built with the fast profile unless --bin-dir is given). Games
started here have no audio (HORIZON_NO_AUDIO=1).
"""
import argparse
import base64
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]
LAVAPIPE = Path('/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json')
LOG_KINDS = ['oak_log', 'birch_log', 'spruce_log', 'jungle_log', 'acacia_log', 'dark_oak_log',
             'mangrove_log', 'cherry_log']
# Positional arguments of the commands the scenario uses, for the CLI.
POSITIONAL = {
    'goto': ['x', 'y', 'z'], 'break': ['x', 'y', 'z'], 'place': ['x', 'y', 'z'], 'block': ['x', 'y', 'z'],
    'find_block': ['block', 'radius'], 'entities': ['radius', 'type'], 'click_slot': ['slot'],
    'click_button': ['label'], 'type': ['text'], 'wait': ['ticks'], 'wait_ready': ['seconds'],
    'command': ['command'], 'look_at_block': ['x', 'y', 'z'], 'click': ['id'], 'set': ['id', 'value'],
    'give': ['item', 'count'], 'options_get': ['keys'], 'options_set': ['key', 'value'],
    'wait_for': ['condition', 'value'],
}
# Booleans the game takes as bare words.
BARE = {'click_slot': ['right', 'shift'], 'click': ['right', 'middle', 'shift']}


class Failed(Exception):
    pass


def game_env(screen):
    if sys.platform == 'darwin':
        return dict(os.environ, HORIZON_NO_AUDIO='1')
    env = dict(os.environ, VSCREEN=screen)
    subprocess.run(['vscreen', 'up'], env=env, check=False, capture_output=True)
    text = subprocess.run(['vscreen', 'env'], env=env, capture_output=True, text=True, check=True).stdout
    env.pop('DISPLAY', None)
    for match in re.finditer(r'(\w+)=(\S+)', text.split('unset')[0]):
        env[match.group(1)] = match.group(2)
    env['HORIZON_NO_AUDIO'] = '1'
    if LAVAPIPE.exists():
        env['VK_ICD_FILENAMES'] = str(LAVAPIPE)
    return env


class Cli:
    """One horizon-ctl process per command, as a shell script would."""
    name = 'cli'

    def __init__(self, ctl, socket, cwd, env):
        self.base = [str(ctl), '--socket', str(socket)]
        self.cwd, self.env = cwd, env

    def __call__(self, tool, **args):
        cmd = tool.replace('_', '-')
        positional = [str(args.pop(k)) for k in POSITIONAL.get(tool, []) if k in args]
        options = []
        for k, v in args.items():
            if v is True:
                options.append(k if k in BARE.get(tool, []) else '--' + k)
            elif v is not False and v is not None:
                options += ['--' + k, str(v)]
        # Options first: free text (type, command) runs to the end of the line.
        words = [cmd] + (options + positional if tool in ('type', 'command') else positional + options)
        if tool == 'screenshot':
            words = ['screenshot', str(args.get('path', self.cwd / 'shot.png'))]
        result = subprocess.run(self.base + words, cwd=self.cwd, env=self.env, capture_output=True, text=True,
                                timeout=600)
        out = result.stdout.strip().splitlines()
        reply = json.loads(out[-1]) if out else {'error': result.stderr.strip()}
        log(self.name, words, reply)
        if result.returncode or 'error' in reply:
            raise Failed(f'{" ".join(words)}: exit {result.returncode}: {reply}')
        return reply

    def close(self):
        pass


class Mcp:
    """`horizon-ctl mcp` driven over JSON-RPC, as an AI agent's client would."""
    name = 'mcp'

    def __init__(self, ctl, socket, cwd, env):
        self.proc = subprocess.Popen([str(ctl), '--socket', str(socket), 'mcp'], cwd=cwd, env=env,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.next_id = 0
        self.cwd = cwd
        init = self.rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                       'clientInfo': {'name': 'agent_control_e2e', 'version': '1'}})
        assert init['serverInfo']['name'] == 'horizon-ctl', init
        self.notify('notifications/initialized')
        self.tools = {t['name']: t for t in self.rpc('tools/list', {})['tools']}
        print(f'[mcp] {len(self.tools)} tools: {", ".join(sorted(self.tools))}')

    def notify(self, method):
        self.proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': method}) + '\n')
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.next_id += 1
        self.proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': self.next_id, 'method': method,
                                          'params': params}) + '\n')
        self.proc.stdin.flush()
        reply = json.loads(self.proc.stdout.readline())
        assert reply['id'] == self.next_id, reply
        if 'error' in reply:
            raise Failed(f'{method}: {reply["error"]}')
        return reply['result']

    def __call__(self, tool, **args):
        if tool not in self.tools:
            raise Failed(f'no MCP tool {tool}')
        result = self.rpc('tools/call', {'name': tool, 'arguments': args})
        texts = [c for c in result['content'] if c['type'] == 'text']
        reply = json.loads(texts[-1]['text']) if texts else {}
        images = [c for c in result['content'] if c['type'] == 'image']
        if images:
            reply['_image'] = base64.b64decode(images[0]['data'])
        log(self.name, [tool, args], {k: v for k, v in reply.items() if k != '_image'})
        if result['isError']:
            raise Failed(f'{tool} {args}: {reply}')
        return reply

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=30)


def log(mode, words, reply):
    text = json.dumps(reply)
    print(f'[{mode}] {words if isinstance(words, list) else words} -> {text[:400]}{"..." if len(text) > 400 else ""}')


def trunk_bases(d):
    """Logs with no log under them, nearest first: (distance, pos, log id)."""
    bases = []
    for kind in LOG_KINDS:
        found = d('find_block', block=kind, radius=40)
        for n in found['nearest']:
            x, y, z = n['pos']
            below = d('block', x=x, y=y - 1, z=z)['block']
            if not below.endswith('_log') and not below.endswith('_wood'):
                bases.append((n['distance'], (x, y, z), found['block']))
    return sorted(bases)


def scenario(d, out):
    # 1. A world, made through the title screen's menus.
    screen = d('screen')
    assert screen['screen'] == 'title', screen
    assert any(b['label'] == 'Singleplayer' for b in screen['buttons']), screen
    d('click_button', label='Singleplayer')
    if d('screen')['menu'].get('mode') != 'create-world':  # with no worlds, Singleplayer opens it itself
        d('click_button', label='Create New World')
    d('type', field='name', replace=True, text='Agent World')
    d('type', field='seed', text='5')  # spawn near a walkable oak
    d('click_button', label='Create New World')
    # The world opens a few frames later: in the frame it is prepared off the UI thread behind the
    # title screen (world_loading.rs), on the server thread once the server has it.
    d('wait_for', condition='loaded', timeout=400)
    d('wait_ready', seconds=400)
    state = d('state')
    assert state['gamemode'] == 'survival' and state['player']['health'] > 0, state
    d('command', command='time set day')
    # The spawn is random within the spawn radius, on the first full top face below the
    # MOTION_BLOCKING height (PlayerSpawnFinder.getLevelRespawnPos): a tree's leaves too, too high
    # for goto's drops.
    x, y, z = state['player']['block_pos']
    ground = y - 1
    while ground > y - 16 and d('block', x=x, y=ground, z=z)['block'].endswith(('_leaves', 'air')):
        ground -= 1
    if ground < y - 1:
        d('command', command=f'tp @s {x + 0.5} {ground + 1} {z + 0.5}')
        d('wait', ticks=5)

    # 2. Walk to a tree, the nearest trunk the path finder can reach, and 3. break the log at the
    # bottom of the trunk (it must end up in the inventory). Low leaves can hide every face of that
    # log from where the walk stops: then the next tree.
    for _, base, log_id in trunk_bases(d)[:8]:
        x, y, z = base
        try:
            d('goto', x=x + 0.5, y=y, z=z + 0.5, near=2.5, timeout=60)
        except Failed as e:
            print(f'  cannot reach {log_id} at {base}: {e}')
            continue
        try:
            broke = d('break', x=x, y=y, z=z)
            break
        except Failed as e:
            if 'no face' not in str(e):
                raise
            print(f'  {log_id} at {base} is hidden: {e}')
    else:
        raise Failed('no reachable tree near spawn')
    assert broke['block_before'].startswith(log_id), broke
    assert broke['drops'].get(log_id, 0) >= 1, broke
    # On a client of a server (single player's server thread) the drop and its pickup both come from
    # the server a few ticks later: wait for the logs in the inventory, or walk to the drop.
    for _ in range(20):
        if broke['picked_up'].get(log_id, 0) >= 1 or d('inventory')['counts'].get(log_id, 0) >= 1:
            break
        items = d('entities', radius=8, type='item')['entities']
        drop = next((e for e in items if e['item']['item'] == log_id), None)
        if drop is not None:
            d('goto', x=drop['pos'][0], y=drop['pos'][1], z=drop['pos'][2], near=1.0, timeout=20)  # pickup reaches a block; the drop may lie under the trunk
            d('wait', ticks=10)
            break
        d('wait', ticks=1)
    inventory = d('inventory')
    assert inventory['counts'].get(log_id, 0) >= 1, inventory

    # 4. Craft planks in the inventory screen's 2x2 grid.
    planks_id = log_id.replace('_log', '_planks')
    d('open_inventory')
    screen = d('screen')
    assert screen['screen'] == 'inventory' and screen['craft_size'] == 2, screen
    log_slot = next(s['slot'] for s in screen['slots'] if s.get('item') == log_id and s['slot'] < 36)
    d('click_slot', slot=log_slot)           # pick up the logs
    d('click_slot', slot=100, right=True)    # put one in the crafting grid
    d('click_slot', slot=log_slot)           # put the rest back
    d('wait', ticks=2)                       # the server crafts the result (single player's server thread)
    result = next(s for s in d('screen')['slots'] if s['slot'] == 200)
    assert result.get('item') == planks_id and result['count'] == 4, result
    d('click_slot', slot=200, shift=True)    # craft into the inventory
    d('close')
    d('wait', ticks=2)
    inventory = d('inventory')
    assert inventory['counts'].get(planks_id, 0) >= 4, inventory

    # 5. Place a plank beside the player and read it back.
    p = d('state')['player']['block_pos']
    hotbar = d('inventory')['hotbar']
    if not any(s.get('item') == planks_id for s in hotbar):
        raise Failed(f'{planks_id} did not land in the hotbar: {hotbar}')
    target = placed = None
    for dx, dz in [(2, 0), (-2, 0), (0, 2), (0, -2), (2, 1), (1, 2), (-2, -1), (-1, -2)]:
        at = (p[0] + dx, p[1], p[2] + dz)
        here = d('block', x=at[0], y=at[1], z=at[2])['block']
        below = d('block', x=at[0], y=at[1] - 1, z=at[2])['block']
        if here not in ('minecraft:air', 'minecraft:short_grass') or below in ('minecraft:air', 'minecraft:water'):
            continue
        # The tree's trunk above the broken log can hide a spot's faces: try the next one.
        try:
            placed = d('place', x=at[0], y=at[1], z=at[2], item=planks_id)
        except Failed as e:
            if 'no visible face' in str(e):
                continue
            raise
        target = at
        break
    if target is None:
        raise Failed(f'no free spot beside {p} to place a block')
    assert placed['changed'], placed
    back = d('block', x=target[0], y=target[1], z=target[2])
    assert back['block'] == planks_id, back
    d('look_at_block', x=target[0], y=target[1], z=target[2])
    shot = d('screenshot', path=str(out / f'{d.name}.png'))
    if '_image' in shot:
        (out / f'{d.name}.png').write_bytes(shot['_image'])
    seen = d('looking_at')
    assert seen['kind'] == 'block' and seen['block']['pos'] == list(target), seen
    return {'tree': base, 'log': log_id, 'placed': target}


def widgets(d):
    """The ui tree as {id: widget}."""
    return {w['id']: w for w in d('ui')['widgets']}


def ui_scenario(d, out):
    # 1. Create a world through the title menus, by widget id.
    tree = widgets(d)
    assert tree['singleplayer']['kind'] == 'button', tree
    d('click', id='singleplayer')
    if 'game' not in widgets(d):  # with no worlds, Singleplayer opens Create New World itself
        d('click', id='create-new-world')
    tree = widgets(d)
    assert tree['game']['kind'] == 'button' and any(w['kind'] == 'field' for w in tree.values()), tree
    d('type', field='name', replace=True, text='UI World')
    d('click', id='create-new-world')
    d('wait_for', condition='loaded', timeout=400)  # a few frames later, as in the survival scenario
    d('wait_ready', seconds=400)

    # 2. Options from the pause menu: a slider set by id moves its value and label.
    d('pause')
    d('click', id='options')
    fov = widgets(d)['fov']
    assert fov['kind'] == 'slider' and abs(fov['value'] - 0.5) < 0.01, fov
    d('set', id='fov', value=1)
    moved = widgets(d)['fov']
    assert moved['value'] > 0.99 and moved['label'] != fov['label'], moved
    d('set', id='fov', value=0.5)
    try:
        d('click', id='no-such-widget')
        raise AssertionError('clicking an unknown id succeeded')
    except Failed as e:
        assert 'no widget' in str(e), e
    try:
        d('set', id='done', value=0.5)
        raise AssertionError('set accepted a button')
    except Failed as e:
        assert 'set moves sliders' in str(e), e
    d('click', id='done')
    assert d('click', id='back-to-game')['screen'] == 'playing'

    # 3. The inventory's slots are widgets too, with their stacks.
    d('give', item='oak_log', count=2)
    d('wait', ticks=2)  # single player's server thread gives them
    d('open_inventory')
    tree = widgets(d)
    log = next(w for w in tree.values() if w['kind'] == 'slot' and (w.get('stack') or {}).get('item') == 'minecraft:oak_log')
    assert log['stack']['count'] == 2 and 'slot-200' in tree, log
    d('click', id=log['id'])
    d('click', id='slot-100', right=True)
    d('click', id=log['id'])
    d('wait', ticks=2)
    assert widgets(d)['slot-200']['stack']['item'] == 'minecraft:oak_planks'
    d('close')
    shot = d('screenshot', path=str(out / f'{d.name}-ui.png'))
    if '_image' in shot:
        (out / f'{d.name}-ui.png').write_bytes(shot['_image'])
    return {'widgets': len(tree)}


def options_scenario(d, out):
    # Every option, typed, from the title screen; then set, refused, applied and saved.
    options = d('options_get')['options']
    assert len(options) > 100, len(options)
    assert options['fov'] == {'type': 'int', 'min': 30, 'max': 110, 'value': 70}, options['fov']
    assert options['renderClouds']['choices'] == ['false', 'fast', 'true'], options['renderClouds']
    assert d('options_set', key='fov', value='90')['option']['value'] == 90
    try:
        d('options_set', key='fov', value='200')
        raise AssertionError('fov 200 was accepted')
    except Failed as e:
        assert '"max":110' in str(e), e
    d('options_set', key='renderClouds', value='fast')
    d('options_set', key='key_key.jump', value='key.keyboard.j')
    got = d('options_get', keys='fov renderClouds key_key.jump')['options']
    assert [got[k]['value'] for k in ('fov', 'renderClouds', 'key_key.jump')] == [90, 'fast', 'key.keyboard.j'], got
    saved = (d.cwd / 'options.txt').read_text()
    assert 'fov:0.5' in saved and 'renderClouds:"fast"' in saved and 'key_key.jump:key.keyboard.j' in saved, saved
    # The options screen shows it: FOV 90 sits at (90 - 30) / 80 of its slider.
    d('click', id='options')
    assert abs(widgets(d)['fov']['value'] - 0.75) < 0.01
    # Applied at once: a GUI scale of 2 on the 1280x720 window makes the GUI 640x360.
    d('options_set', key='guiScale', value='2')
    assert d('ui')['gui_size'] == [640, 360], 'the GUI scale did not apply'
    d('options_set', key='guiScale', value='0')
    return {'options': len(options)}


def events_scenario(d, out):
    # Waits instead of polling, and the event log of what happened meanwhile.
    start = d('events')['last']
    d('click', id='singleplayer')
    if 'game' not in widgets(d):
        d('click', id='create-new-world')
    d('type', field='name', replace=True, text='Events')
    d('click', id='create-new-world')
    d('wait_for', condition='loaded', timeout=400)
    seq = d('events')['last']  # an event can happen before the wait starts
    d('give', item='oak_log', count=3)
    d('wait_for', condition='event', value='item', timeout=5, since=seq)
    seq = d('events')['last']
    d('command', command='say hello agents')
    d('wait_for', condition='chat', value='hello agents', timeout=5, since=seq)
    d('open_inventory')
    d('wait_for', condition='screen', value='inventory', timeout=5)
    d('close')
    seq = d('events')['last']
    d('command', command='kill @s')
    d('wait_for', condition='event', value='death', timeout=10, since=seq)
    d('wait', ticks=25)  # the death screen's buttons wake after 20 ticks
    d('respawn')
    d('wait_for', condition='event', value='respawn', timeout=10, since=seq)
    try:
        d('wait_for', condition='screen', value='advancements', timeout=0.5)
        raise AssertionError('the wait did not time out')
    except Failed as e:
        assert 'timed out' in str(e) and 'timeout' in str(e), e
    events = d('events', since=start)['events']
    kinds = [e['event'] for e in events]
    for kind in ['world', 'screen', 'item', 'chat', 'death', 'respawn']:
        assert kind in kinds, f'no {kind} event in {kinds}'
    logs = next(e for e in events if e['event'] == 'item' and e['item'] == 'minecraft:oak_log')
    assert logs['delta'] == 3, logs
    assert all(b['seq'] > a['seq'] for a, b in zip(events, events[1:]))
    return {'events': len(events)}


def consume_scenario(d, out):
    # Consumables finish as vanilla's Consumable.onConsume: always-edible food at full hunger with its
    # effects, milk and honey (no food or drink sounds) with their effects and use remainders, and
    # chorus fruit's random teleport.
    d('click', id='singleplayer')
    if 'game' not in widgets(d):
        d('click', id='create-new-world')
    d('type', field='name', replace=True, text='Consume')
    d('click', id='create-new-world')
    d('wait_for', condition='loaded', timeout=400)

    def consume(item, seconds):
        d('command', command='clear @s')
        d('give', item=item, count=1)
        d('use_item', hold=seconds)
        d('wait', ticks=5)
        state = d('state')
        return state, {e['id'] for e in state['player']['effects']}

    def held(state):
        return state['held'] and state['held']['item']

    state, effects = consume('golden_apple', 2.2)
    assert state['player']['food'] == 20 and {'regeneration', 'absorption'} <= effects, effects
    assert held(state) is None, state['held']
    d('command', command='effect give @s minecraft:speed 60')
    state, effects = consume('milk_bucket', 2.2)
    assert effects == set(), effects
    assert held(state) == 'minecraft:bucket', state['held']
    d('command', command='effect give @s minecraft:poison 60')
    d('command', command='effect give @s minecraft:speed 60')
    state, effects = consume('honey_bottle', 2.6)
    assert effects == {'speed'}, effects
    assert held(state) == 'minecraft:glass_bottle', state['held']
    before = d('state')['player']['position']
    state, _ = consume('chorus_fruit', 2.2)
    after = state['player']['position']
    moved = [b - a for a, b in zip(before, after)]
    assert held(state) is None, state['held']
    assert moved != [0, 0, 0] and abs(moved[0]) <= 8 and abs(moved[2]) <= 8, moved
    d('screenshot', path=str(out / f'consume-{d.name}.png'))
    return {'teleported': [round(m, 2) for m in moved]}


def pick_scenario(d, out):
    # The pick block button (middle) over a container screen clones only with infinite materials:
    # AbstractContainerScreen.mouseClicked sends CLONE, which fills the empty hand with a full stack
    # of the slot's item (#1048: it clicked like the left button and took the slot's stack).
    d('click', id='singleplayer')
    if 'game' not in widgets(d):
        d('click', id='create-new-world')
    d('type', field='name', replace=True, text='Pick')
    d('click', id='create-new-world')
    d('wait_for', condition='loaded', timeout=400)
    d('command', command='gamemode creative')
    d('command', command='clear @s')
    d('give', item='oak_log', count=2)

    def carried():
        return d('inventory')['carried']

    # 1. An item tab: the hotbar slot's stack is cloned, the slot keeps its 2 logs.
    assert d('open_inventory')['screen'] == 'creative'
    tree = widgets(d)
    assert tree['slot-0']['stack'] == {'item': 'minecraft:oak_log', 'count': 2}, tree['slot-0']
    d('click', id='slot-0', middle=True)
    assert carried() == {'item': 'minecraft:oak_log', 'count': 64}, carried()
    assert widgets(d)['slot-0']['stack']['count'] == 2
    d('close')
    d('command', command='clear @s')
    d('give', item='oak_log', count=2)

    # 2. The tab's own display slot gives a full stack too, not one item.
    d('open_inventory')
    shown = widgets(d)['slot-900']['stack']
    d('click', id='slot-900', middle=True)
    assert carried() == {'item': shown['item'], 'count': 64}, (shown, carried())
    d('close')

    # 3. Survival: the middle button over a slot does nothing.
    d('command', command='gamemode survival')
    d('command', command='clear @s')
    d('give', item='oak_log', count=2)
    assert d('open_inventory')['screen'] == 'inventory'
    d('click', id='slot-0', middle=True)
    # The clone of step 2 lived only on this client: ItemPickerMenu's carried stack is the
    # inventoryMenu's, and AbstractContainerMenu.removed clears it only for a ServerPlayer. With a
    # stack in hand, mouseReleased sends CLONE, the server finds the carried stack it was told about
    # wrong and sends the empty one back (broadcastChanges). On the server thread that takes a
    # round trip, as in vanilla; in the frame, closing the menu already put the clone away.
    for _ in range(20):
        if carried() is None:
            break
        d('wait', ticks=1)
    assert carried() is None, carried()
    assert widgets(d)['slot-0']['stack'] == {'item': 'minecraft:oak_log', 'count': 2}
    d('close')
    return {'cloned': shown['item']}


SCENARIOS = {'survival': scenario, 'ui': ui_scenario, 'options': options_scenario, 'events': events_scenario,
             'consume': consume_scenario, 'pick': pick_scenario}


def run(mode, ctl, out, resources, which='survival'):
    work = Path(tempfile.mkdtemp(prefix=f'hzac-{mode}-'))
    (work / '.local').mkdir()
    (work / '.local/horizon-client').mkdir()
    os.symlink(resources, work / '.local/horizon-client' / lab.VERSION)
    socket = work / 'c.sock'
    env = game_env(os.environ.get('VSCREEN', 'agent-control'))
    driver = (Cli if mode == 'cli' else Mcp)(ctl, socket, work, env)
    started = time.time()
    try:
        driver('launch', wait=False, offscreen=sys.platform == 'darwin')
        summary = SCENARIOS[which](driver, out)
        print(f'[{mode}] PASS in {time.time() - started:.0f} s: {summary}')
        return True
    except (Failed, AssertionError, StopIteration) as e:
        print(f'[{mode}] FAIL: {e!r}')
        log_file = socket.with_suffix('.log')
        if log_file.exists():
            print('game log tail:\n' + '\n'.join(log_file.read_text(errors='replace').splitlines()[-15:]))
        return False
    finally:
        try:
            driver('quit')
        except Exception:
            pass
        driver.close()
        time.sleep(2)
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--mode', choices=['cli', 'mcp', 'both'], default='both')
    parser.add_argument('--scenario', choices=[*SCENARIOS, 'all'], default='all')
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build with the fast profile)')
    parser.add_argument('--out', default=str(ROOT / '.local/agent-control-e2e'), help='screenshots go here')
    args = parser.parse_args()
    if args.bin_dir:
        bins = Path(args.bin_dir).resolve()
    else:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '--bins'],
                       cwd=ROOT, check=True)
        meta = json.loads(subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT,
                                         capture_output=True, text=True, check=True).stdout)
        bins = Path(meta['target_directory']) / 'fast'
    resources = (lab.RESOURCES).resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    modes = ['cli', 'mcp'] if args.mode == 'both' else [args.mode]
    scenarios = list(SCENARIOS) if args.scenario == 'all' else [args.scenario]
    results = {f'{m}/{s}': run(m, bins / 'horizon-ctl', out, resources, s) for m in modes for s in scenarios}
    print(json.dumps(results))
    sys.exit(0 if all(results.values()) else 1)


if __name__ == '__main__':
    main()
