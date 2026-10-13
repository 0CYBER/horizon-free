#!/usr/bin/env python3
"""End-to-end check of agent control: play Horizon on a virtual screen with horizon-ctl.

A script creates a world through the title menus, finds a tree it can walk to, breaks a log,
checks the inventory, crafts planks in the inventory screen, places one and reads it back. It
runs through the CLI (one horizon-ctl process per command) and through `horizon-ctl mcp` with a
small JSON-RPC client, and saves a screenshot of each run.

    python3 tools/agent_control_e2e.py                 # both modes
    python3 tools/agent_control_e2e.py --mode mcp      # one of them
    python3 tools/agent_control_e2e.py --bin-dir target/fast --out /tmp/agent-e2e

Needs vscreen (Linux desktop), the prepared resources in .local/horizon-client and a build of
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

ROOT = Path(__file__).resolve().parents[1]
LAVAPIPE = Path('/run/opengl-driver/share/vulkan/icd.d/lvp_icd.x86_64.json')
LOG_KINDS = ['oak_log', 'birch_log', 'spruce_log', 'jungle_log', 'acacia_log', 'dark_oak_log',
             'mangrove_log', 'cherry_log']
# Positional arguments of the commands the scenario uses, for the CLI.
POSITIONAL = {
    'goto': ['x', 'y', 'z'], 'break': ['x', 'y', 'z'], 'place': ['x', 'y', 'z'], 'block': ['x', 'y', 'z'],
    'find_block': ['block', 'radius'], 'entities': ['radius', 'type'], 'click_slot': ['slot'],
    'click_button': ['label'], 'type': ['text'], 'wait': ['ticks'], 'wait_ready': ['seconds'],
    'command': ['command'], 'look_at_block': ['x', 'y', 'z'],
}
# Booleans the game takes as bare words.
BARE = {'click_slot': ['right', 'shift']}


class Failed(Exception):
    pass


def vscreen_env(screen):
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
    d('click_button', label='Create New World')
    d('type', field='name', replace=True, text='Agent World')
    d('type', field='seed', text='5')  # spawn near a walkable oak
    created = d('click_button', label='Create New World')
    assert created['screen'] == 'playing', created
    d('wait_ready', seconds=400)
    state = d('state')
    assert state['gamemode'] == 'survival' and state['player']['health'] > 0, state
    d('command', command='time set day')

    # 2. Walk to a tree: the nearest trunk the path finder can reach.
    for _, base, log_id in trunk_bases(d)[:8]:
        x, y, z = base
        try:
            d('goto', x=x + 0.5, y=y, z=z + 0.5, near=2.5, timeout=60)
            break
        except Failed as e:
            print(f'  cannot reach {log_id} at {base}: {e}')
    else:
        raise Failed('no reachable tree near spawn')

    # 3. Break the log at the bottom of the trunk; it must end up in the inventory.
    broke = d('break', x=x, y=y, z=z)
    assert broke['block_before'].startswith(log_id), broke
    assert broke['drops'].get(log_id, 0) >= 1, broke
    if broke['picked_up'].get(log_id, 0) < 1:
        items = d('entities', radius=8, type='item')['entities']
        drop = next(e for e in items if e['item']['item'] == log_id)
        d('goto', x=drop['pos'][0], y=drop['pos'][1], z=drop['pos'][2], near=0.4, timeout=20)
        d('wait', ticks=10)
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
    result = next(s for s in d('screen')['slots'] if s['slot'] == 200)
    assert result.get('item') == planks_id and result['count'] == 4, result
    d('click_slot', slot=200, shift=True)    # craft into the inventory
    d('close')
    inventory = d('inventory')
    assert inventory['counts'].get(planks_id, 0) >= 4, inventory

    # 5. Place a plank beside the player and read it back.
    p = d('state')['player']['block_pos']
    target = None
    for dx, dz in [(2, 0), (-2, 0), (0, 2), (0, -2), (2, 1), (1, 2), (-2, -1), (-1, -2)]:
        at = (p[0] + dx, p[1], p[2] + dz)
        here = d('block', x=at[0], y=at[1], z=at[2])['block']
        below = d('block', x=at[0], y=at[1] - 1, z=at[2])['block']
        if here in ('minecraft:air', 'minecraft:short_grass') and below not in ('minecraft:air', 'minecraft:water'):
            target = at
            break
    if target is None:
        raise Failed(f'no free spot beside {p} to place a block')
    hotbar = d('inventory')['hotbar']
    if not any(s.get('item') == planks_id for s in hotbar):
        raise Failed(f'{planks_id} did not land in the hotbar: {hotbar}')
    placed = d('place', x=target[0], y=target[1], z=target[2], item=planks_id)
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


def run(mode, ctl, out, resources):
    work = Path(tempfile.mkdtemp(prefix=f'hzac-{mode}-'))
    (work / '.local').mkdir()
    (work / '.local/horizon-client').mkdir()
    os.symlink(resources, work / '.local/horizon-client/26.3')
    socket = work / 'c.sock'
    env = vscreen_env(os.environ.get('VSCREEN', 'agent-control'))
    driver = (Cli if mode == 'cli' else Mcp)(ctl, socket, work, env)
    started = time.time()
    try:
        driver('launch', wait=False)
        summary = scenario(driver, out)
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
    resources = (ROOT / '.local/horizon-client/26.3').resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    modes = ['cli', 'mcp'] if args.mode == 'both' else [args.mode]
    results = {m: run(m, bins / 'horizon-ctl', out, resources) for m in modes}
    print(json.dumps(results))
    sys.exit(0 if all(results.values()) else 1)


if __name__ == '__main__':
    main()
