#!/usr/bin/env python3
"""End-to-end check of the launcher's MCP playing the game through its game_* tools.

A scratch launcher home gets one "installed" version made of this checkout's builds
(horizon-client and horizon-ctl) and the prepared resources in .local/horizon-client. An MCP
client then talks to `horizon-launcher mcp` alone: the game tools are listed, a call before the
game runs says to launch it, launch_client starts it under control, and game_wait_ready,
game_state, game_ui and game_screenshot play it until game_quit.

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
    for name in ['launch_client', 'game_state', 'game_ui', 'game_click', 'game_screenshot']:
        assert name in tools, f'{name} missing from {sorted(tools)}'
    assert 'game_launch' not in tools, 'launch_client covers horizon-ctl launch'

    result, texts, _ = m.call('game_state')
    assert result['isError'] and 'launch_client' in texts[-1], texts

    result, texts, _ = m.call('launch_client', world='Agent', creative=True,
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
    env = dict(os.environ, HORIZON_LAUNCHER_HOME=str(work / 'launcher'), HORIZON_HOME=str(work / 'home'),
               HORIZON_NO_AUDIO='1')
    m = Mcp(Path(args.launcher).resolve(), env)
    try:
        summary = scenario(m, out)
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
