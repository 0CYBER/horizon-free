#!/usr/bin/env python3
"""End-to-end check of the launcher's MCP playing the game through its game_* tools.

A scratch launcher home gets one "installed" version made of this checkout's builds
(horizon-client and horizon-ctl) and the prepared resources in .local/horizon-client. An MCP
client then talks to `horizon-launcher mcp` alone: every CLI command is listed as a tool of the same
name (versions, settings, mods_search...), a destructive one needs confirm, the game tools are
listed, a call before the game runs says to start it, play starts it under control, and
game_wait_ready, game_state, game_ui and game_screenshot play it until game_quit. Then No AI is turned on with
`horizon-launcher no-ai on`: `horizon-launcher mcp` refuses to start, and a session already running
refuses its next call.

    python3 tools/launcher_mcp_e2e.py --bin-dir target/fast --launcher launcher/target/debug/horizon-launcher

On macOS the game renders offscreen (it runs in a locked session or over SSH); on Linux it needs
a display (vscreen on the desktop). Games started here have no audio.
"""
import argparse
import base64
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]
TAG = 'v2099.1.1'


class Failed(Exception):
    pass


class Mcp:
    def __init__(self, launcher, env):
        self.proc = subprocess.Popen([str(launcher), 'mcp'], env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, text=True)
        self.next_id = 0

    def rpc(self, method, params):
        self.next_id += 1
        self.proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': self.next_id, 'method': method,
                                          'params': params}) + '\n')
        self.proc.stdin.flush()
        reply = json.loads(self.proc.stdout.readline())
        if 'error' in reply:
            raise Failed(f'{method}: {reply["error"]}')
        return reply['result']

    def call(self, tool, **args):
        result = self.rpc('tools/call', {'name': tool, 'arguments': args})
        texts = [c['text'] for c in result['content'] if c['type'] == 'text']
        images = [c for c in result['content'] if c['type'] == 'image']
        print(f'[launcher-mcp] {tool} {args} -> {(texts[-1] if texts else "")[:300]}')
        return result, texts, images

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=60)


def scenario(m, out):
    init = m.rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                'clientInfo': {'name': 'launcher_mcp_e2e', 'version': '1'}})
    assert init['serverInfo']['name'] == 'horizon-launcher', init
    tools = {t['name'] for t in m.rpc('tools/list', {})['tools']}
    for name in ['play', 'server', 'versions', 'settings', 'mods_search', 'modpack_list', 'servers_list',
                 'data_status', 'account_list', 'game_args', 'game_state', 'game_ui', 'game_click',
                 'game_screenshot']:
        assert name in tools, f'{name} missing from {sorted(tools)}'
    for name in ['game_launch', 'mcp', 'no_ai', 'agent_config']:
        assert name not in tools, f'{name} is a tool'

    # Generated from the CLI: the same command, its JSON output.
    result, texts, _ = m.call('versions', installed=True)
    assert json.loads(texts[-1])['installed'] == [TAG], texts
    result, texts, _ = m.call('settings', key='opening-animation', value='off')
    assert json.loads(texts[-1])['opening_animation'] is False, texts
    result, texts, _ = m.call('game_args')
    assert 'game_args' in json.loads(texts[-1]), texts
    result, texts, _ = m.call('remove', tag=TAG)
    assert result['isError'] and 'confirm' in texts[-1], texts

    result, texts, _ = m.call('game_state')
    assert result['isError'] and 'with play' in texts[-1], texts

    result, texts, _ = m.call('play', world='Agent', creative=True,
                              offscreen=sys.platform == 'darwin')
    assert not result['isError'], texts
    result, texts, _ = m.call('game_wait_ready', seconds=300)
    assert not result['isError'], texts
    result, texts, _ = m.call('game_state')
    state = json.loads(texts[-1])
    assert state['screen'] == 'playing' and state['gamemode'] == 'creative', state
    m.call('game_pause')
    result, texts, _ = m.call('game_ui')
    ids = {w['id'] for w in json.loads(texts[-1])['widgets']}
    assert 'back-to-game' in ids, ids
    result, texts, _ = m.call('game_click', id='back-to-game')
    assert json.loads(texts[-1])['screen'] == 'playing', texts
    result, _, images = m.call('game_screenshot')
    assert images, 'no image'
    (out / 'launcher-mcp.png').write_bytes(base64.b64decode(images[0]['data']))
    m.call('game_quit')
    return {'tools': len(tools)}


REFUSAL = "AI features are off: turn off No AI in the launcher's settings"


def no_ai_scenario(launcher, env, home):
    run = lambda *a: subprocess.run([str(launcher), *a], env=env, input='', capture_output=True, text=True,
                                    timeout=60)
    assert json.loads(run('no-ai', '--json').stdout) == {'no_ai': False}
    config = json.loads(run('agent-config', '--json').stdout)
    assert config['server']['args'] == ['mcp'] and len(config['clients']) == 4, config
    assert 'agent-config' in run('--help').stdout
    test = json.loads(run('agent-config', '--test', '--json').stdout)
    assert test['answered'] and test['tools'] > 100, test
    m = Mcp(launcher, env)
    m.rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                         'clientInfo': {'name': 'launcher_mcp_e2e', 'version': '1'}})
    r = run('no-ai', 'on')
    assert r.returncode == 0, r
    assert 'noAi:true' in (home / 'horizon-extras.txt').read_text()
    try:
        m.call('links')
        raise AssertionError('links ran under No AI')
    except Failed as e:
        assert REFUSAL in str(e), e
    m.close()
    for command in ('mcp', 'agent-config'):
        r = run(command)
        print(f'[launcher-mcp] {command} under No AI -> exit {r.returncode}: {r.stderr.strip()}')
        assert r.returncode == 1 and r.stderr.strip() == f'error: {REFUSAL}', r
    assert 'agent-config' not in run('--help').stdout and ' mcp ' not in run('--help').stdout
    # What the launcher needs keeps working.
    assert run('versions', '--installed', '--json').returncode == 0
    assert run('no-ai', 'off').returncode == 0
    assert json.loads(run('no-ai', '--json').stdout) == {'no_ai': False}
    return {'no_ai': 'refused'}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', required=True, help='horizon-client and horizon-ctl built from this checkout')
    parser.add_argument('--launcher', required=True, help='a horizon-launcher binary')
    parser.add_argument('--out', default=str(ROOT / '.local/launcher-mcp-e2e'), help='the screenshot goes here')
    args = parser.parse_args()
    bins = Path(args.bin_dir).resolve()
    resources = (lab.RESOURCES).resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='hzlm-'))
    version = work / 'launcher/versions' / TAG
    (version / '.local/horizon-client').mkdir(parents=True)
    for name in ['horizon-client', 'horizon-ctl']:
        os.symlink(bins / name, version / name)
    os.symlink(resources, version / '.local/horizon-client' / lab.VERSION)
    (version / '.installed').write_text('')
    env = lab.game_env(HORIZON_LAUNCHER_HOME=work / 'launcher', HORIZON_HOME=work / 'home', HORIZON_TELEMETRY_URL='off')
    m = Mcp(Path(args.launcher).resolve(), env)
    try:
        summary = scenario(m, out)
        summary.update(no_ai_scenario(Path(args.launcher).resolve(), env, work / 'home'))
        print(f'[launcher-mcp] PASS: {summary}')
        ok = True
    except (Failed, AssertionError, KeyError, json.JSONDecodeError) as e:
        print(f'[launcher-mcp] FAIL: {e!r}')
        ok = False
        try:
            m.call('game_quit')
        except Exception:
            pass
    finally:
        m.close()
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(0 if ok else 1)


if __name__ == '__main__':
    main()
