#!/usr/bin/env python3
"""horizon-server's admin API on a real server, end to end (docs/api/server.md).

    python3 tools/server_admin_e2e.py --bin-dir target/fast

`horizon-server run` supervises Horizon's dedicated server (offline mode, a flat world); two horizon-bots
players join. Through the control API, with the token from .horizon/api.json, the run lists the players, then
ops and deops, whitelists, bans and pardons by name and by address, kicks, and turns the whitelist on and off,
checking each reply (`changed`, the console's lines), vanilla's files, the server's log and `GET players`.
It also checks the Prometheus text, and that a call without the token is refused and a name with a space is
not run; then reads the settings and changes the difficulty live, and backs up, downloads and deletes a backup.
The API listens on the address and with the token server.toml's [api] fixes, as a moderation bot's would.

Everything runs in a fresh temporary folder with fresh HORIZON_HOME and XDG folders and no audio. Logs go
to --out. Build first:
    cargo build --profile fast -p horizon-client --bins -p horizon-bots -p horizon-server
"""
import argparse
import http.client
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
IP = '203.0.113.7'
TOKEN = 'the-moderation-bots-token'


class Failed(Exception):
    pass


def check(condition, what):
    if not condition:
        raise Failed(what)
    print(f'ok: {what}')


class Api:
    def __init__(self, folder):
        endpoint = json.loads((folder / '.horizon' / 'api.json').read_text())
        self.host, port = endpoint['address'].rsplit(':', 1)
        self.port = int(port)
        self.token = endpoint['token']

    def call(self, method, path, body=None, token=None, raw=False):
        conn = http.client.HTTPConnection(self.host, self.port, timeout=60)
        headers = {'Authorization': f'Bearer {self.token if token is None else token}',
                   'Content-Type': 'application/json'}
        conn.request(method, path, json.dumps(body) if body is not None else None, headers)
        response = conn.getresponse()
        data = response.read()
        conn.close()
        return response.status, data if raw else data.decode()

    def __call__(self, method, path, body=None):
        status, text = self.call(method, path, body)
        if status != 200:
            raise Failed(f'{method} {path} {body}: {status} {text}')
        return json.loads(text)


def bot(bins, port, name):
    return subprocess.Popen([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1',
                             '--behaviour', 'stand', '--seconds', '600', '--name', name],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def wait_until(what, test, timeout=120):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if test():
            return
        time.sleep(0.5)
    raise Failed(f'not within {timeout} s: {what}')


def listed(folder, file):
    path = folder / file
    return [e.get('name') or e.get('ip') for e in json.loads(path.read_text())] if path.exists() else []


def online(api):
    return sorted(p['name'] for p in api('GET', '/api/v1/players')['online'])


def scenario(bins, folder, server, port):
    api = Api(folder)
    wait_until('the supervisor says running', lambda: api('GET', '/api/v1/status')['phase'] == 'running')
    check(api.call('GET', '/api/v1/players', token='nope')[0] == 401, 'a call without the token is refused')
    bots = [bot(bins, port, name) for name in ('Kim', 'Lou')]
    try:
        wait_until('Kim0 and Lou0 online', lambda: online(api) == ['Kim0', 'Lou0'])
        players = api('GET', '/api/v1/players')
        check(players['max_players'] == 20 and players['ops'] == [] and players['bans'] == [],
              f'players: 20 slots, no operators, no bans ({players})')
        check(all(p['address'] == '127.0.0.1' and p['op_level'] == 0 for p in players['online']),
              'online players have their address and no op level')
        wait_until('the sampler sees 2 players',
                   lambda: 'horizon_players_online 2\n' in api.call('GET', '/api/v1/metrics/prometheus')[1])
        text = api.call('GET', '/api/v1/metrics/prometheus')[1]
        check('horizon_up 1\n' in text and 'horizon_tps ' in text and 'horizon_mspt{stat="p95"}' in text,
              'Prometheus text: up, TPS and MSPT')

        def act(path, body, changed, line):
            reply = api('POST', path, body)
            check(reply['changed'] == changed and any(line in o for o in reply['output']),
                  f'{path} {body}: changed {changed}, "{line}" ({reply})')

        act('/api/v1/players/op', {'name': 'Kim0'}, True, 'Made Kim0 a server operator')
        check(listed(folder, 'ops.json') == ['Kim0'], 'ops.json has Kim0')
        check([p['op_level'] for p in api('GET', '/api/v1/players')['online'] if p['name'] == 'Kim0'] == [4],
              'Kim0 shows op level 4')
        act('/api/v1/players/op', {'name': 'Kim0'}, False, 'Nothing changed')
        act('/api/v1/players/deop', {'name': 'Kim0'}, True, 'Made Kim0 no longer a server operator')
        act('/api/v1/whitelist/add', {'name': 'Lou0'}, True, 'Added Lou0 to the whitelist')
        check(listed(folder, 'whitelist.json') == ['Lou0'], 'whitelist.json has Lou0')
        act('/api/v1/whitelist/remove', {'name': 'Lou0'}, True, 'Removed Lou0 from the whitelist')
        act('/api/v1/players/ban-ip', {'ip': IP, 'reason': 'spam bots'}, True, f'Banned IP {IP}: spam bots')
        check([b['ip'] for b in api('GET', '/api/v1/players')['ip_bans']] == [IP], 'players lists the IP ban')
        act('/api/v1/players/pardon-ip', {'ip': IP}, True, f'Unbanned IP {IP}')
        check(listed(folder, 'banned-ips.json') == [], 'banned-ips.json is empty again')

        status, text = api.call('POST', '/api/v1/players/ban', {'name': 'Kim0 extra'})
        check(status == 400, f'a name with a space is refused ({status} {text})')
        act('/api/v1/players/kick', {'name': 'Nobody'}, False, 'No player was found')
        act('/api/v1/players/kick', {'name': 'Kim0', 'reason': 'spamming chat'}, True, 'Kicked Kim0: spamming chat')
        server.wait_for(r'Kim0 lost connection: spamming chat', 30)
        act('/api/v1/players/ban', {'name': 'Lou0', 'reason': 'cheating'}, True, 'Banned Lou0: cheating')
        server.wait_for(r'Lou0 lost connection', 30)
        bans = api('GET', '/api/v1/players')['bans']
        check([(b['name'], b['reason'], b['source']) for b in bans] == [('Lou0', 'cheating', 'Server')],
              'players lists Lou0 banned for cheating by Server')
        wait_until('no one online', lambda: online(api) == [], 30)
        act('/api/v1/players/pardon', {'name': 'Lou0'}, True, 'Unbanned Lou0')

        act('/api/v1/whitelist', {'enabled': True}, True, 'Whitelist is now turned on')
        check(api('GET', '/api/v1/players')['whitelist']['enabled'] is True, 'players says the whitelist is on')
        act('/api/v1/whitelist', {'enabled': False}, True, 'Whitelist is now turned off')
        check(listed(folder, 'banned-players.json') == [], 'banned-players.json is empty again')

        config = api('GET', '/api/v1/config')['config']
        check(config['api']['token'] == '(set)' and config['world']['difficulty'] == 'easy',
              'config: the token hidden, the difficulty easy')
        outcome = api('PATCH', '/api/v1/config', {'changes': {'world.difficulty': 'hard', 'server.max_players': 30}})
        check(outcome['applied'] == ['world.difficulty'] and outcome['restart'] == ['server.max_players'],
              f'config: the difficulty applies now, max_players at the next start ({outcome})')
        said = api('POST', '/api/v1/command', {'line': 'difficulty'})['output']
        check(any('The difficulty is Hard' in line for line in said), f'the game plays on Hard ({said})')
        check('difficulty = "hard"' in (folder / 'server.toml').read_text(), 'server.toml says hard')
        status, _ = api.call('PATCH', '/api/v1/config', {'changes': {'engine.path': '/bin/sh'}})
        check(status == 403, 'the API cannot change the program the server runs')

        name = api('POST', '/api/v1/backups', {'name': 'e2e'})['name']
        check(name in [b['name'] for b in api('GET', '/api/v1/backups')['backups']], f'backup {name} is listed')
        status, data = api.call('GET', f'/api/v1/backups/download?name={name}', raw=True)
        check(status == 200 and data == (folder / 'backups' / name).read_bytes() and data[:2] == b'PK',
              f'the download is the zip ({len(data)} bytes)')
        api('DELETE', f'/api/v1/backups?name={name}')
        check(not (folder / 'backups' / name).exists(), 'the backup is deleted')
    finally:
        for b in bots:
            b.kill()
    api('POST', '/api/v1/stop')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-admin-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    work = Path(tempfile.mkdtemp(prefix='hz-server-admin-'))
    folder = work / 'server'
    folder.mkdir()
    port, api_port = lab.free_port(), lab.free_port()
    (folder / 'eula.txt').write_text('eula=true\n')
    (folder / 'server.toml').write_text(f'''[server]
port = {port}
online_mode = false
whitelist = false
view_distance = 3
simulation_distance = 3

[properties]
level-type = "minecraft:flat"
generate-structures = false
spawn-protection = 0

[engine]
path = {json.dumps(str(bins / f'horizon-client{EXE}'))}
resources = {json.dumps(str(lab.RESOURCES.resolve()))}
restart_on_crash = false

[backups]
every = "0"

[panel]
bind = ""

[api]
bind = "127.0.0.1:{api_port}"
token = "{TOKEN}"
''')
    server = Server([bins / f'horizon-server{EXE}', '--dir', folder, 'run'], folder, out / 'server.log')
    try:
        server.wait_for(DONE, 300)
        endpoint = json.loads((folder / '.horizon' / 'api.json').read_text())
        if (endpoint['address'], endpoint['token']) != (f'127.0.0.1:{api_port}', TOKEN):
            raise Failed(f'the API is not where [api] says: {endpoint}')
        scenario(bins, folder, server, port)
        server.process.wait(timeout=120)
    except (Failed, RuntimeError, OSError, subprocess.TimeoutExpired) as e:
        sys.exit(f'server_admin_e2e failed (logs in {out}): {e}')
    finally:
        server.stop()
        shutil.rmtree(work, ignore_errors=True)
    print('server_admin_e2e: the admin API listed, opped, whitelisted, banned, pardoned and kicked as vanilla does')


if __name__ == '__main__':
    main()
