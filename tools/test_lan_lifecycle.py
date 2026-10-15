#!/usr/bin/env python3
"""Check LAN shutdown when leaving a singleplayer world through the pause menu,
and that offline guests joining with their default names are not refused as the
host's profile (#195: everyone was "Player").

Requires a built horizon-client, prepared resources and a display on Unix:
    xvfb-run -a python3 tools/test_lan_lifecycle.py --client target/fast/horizon-client
Worlds and settings are created in a temporary directory.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]
GROUP = '224.0.2.60'
DISCOVERY_PORT = 4445


def command(endpoint, line):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(60)
        connection.connect(str(endpoint))
        connection.sendall((line + '\n').encode())
        with connection.makefile('r') as reply:
            result = json.loads(reply.readline())
    assert 'error' not in result, (line, result)
    return result


def wait_for(endpoint, line, predicate, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            result = command(endpoint, line)
        except (FileNotFoundError, ConnectionRefusedError):
            time.sleep(0.1)
            continue
        if predicate(result):
            return result
        time.sleep(0.1)
    raise AssertionError(f'timed out waiting for {line}')


def announcement(scanner, port, timeout):
    deadline = time.monotonic() + timeout
    marker = f'[AD]{port}[/AD]'.encode()
    while time.monotonic() < deadline:
        scanner.settimeout(max(0.001, deadline - time.monotonic()))
        try:
            message, _ = scanner.recvfrom(4096)
        except socket.timeout:
            return False
        if marker in message:
            return True
    return False


def scenario(endpoint, scanner):
    wait_for(endpoint, 'screen', lambda value: value['screen'] == 'title')
    command(endpoint, 'open LanLifecycle 0 creative')
    wait_for(endpoint, 'status', lambda value: value['player'] is not None
             and value['pending_chunks'] == 0)
    assert command(endpoint, 'lan status') == {'open': False}
    port = 0
    for _ in range(2):
        port = command(endpoint, f'lan open {port}')['port']
        with socket.create_connection(('127.0.0.1', port), timeout=2):
            pass
        assert announcement(scanner, port, 5), 'LAN world was not advertised'
        command(endpoint, 'key esc tap')
        screen = wait_for(endpoint, 'screen', lambda value: any(
            button['label'] == 'Save and Quit to Title' for button in value['buttons']))
        assert screen['screen'] == 'paused', screen
        command(endpoint, 'click-button Save and Quit to Title')
        wait_for(endpoint, 'screen', lambda value: value['screen'] == 'title')
        assert command(endpoint, 'lan status') == {'open': False}, 'LAN stayed open after leaving'
        try:
            connection = socket.create_connection(('127.0.0.1', port), timeout=2)
        except ConnectionRefusedError:
            pass
        else:
            connection.close()
            raise AssertionError('LAN still accepts connections after leaving')
        # Discard datagrams queued before shutdown, then listen for more than
        # two announcement intervals (the pinger sends every 1.5 seconds).
        scanner.setblocking(False)
        try:
            while True:
                scanner.recvfrom(4096)
        except BlockingIOError:
            pass
        assert not announcement(scanner, port, 3.5), 'LAN kept advertising after leaving'
        command(endpoint, 'open LanLifecycle')
        wait_for(endpoint, 'status', lambda value: value['player'] is not None
                 and value['pending_chunks'] == 0)
        assert command(endpoint, 'lan status') == {'open': False}, 'LAN reopened with the world'
    command(endpoint, 'lan open 0')
    command(endpoint, 'quit')


def start_client(client, resources, home, endpoint, log, *extra):
    return subprocess.Popen([
        str(client), '--resources', str(resources), '--render-distance', '2',
        '--test-terrain', '--control-socket', str(endpoint), '--timeout-seconds', '180', *extra,
    ], cwd=home, env=dict(os.environ, HORIZON_HOME=str(home), HORIZON_NO_AUDIO='1',
                         HORIZON_MENU_STYLE='classic', HORIZON_PLAYER_NAME=''),
        stdout=log, stderr=subprocess.STDOUT)


def guest_names(endpoint, port, guests):
    """Two guests with default names join together; a guest named like the host is refused."""
    names = []
    for home, guest in guests:
        wait_for(guest, 'screen', lambda value: value['screen'] == 'title')
        command(guest, f'connect 127.0.0.1:{port}')
        wait_for(guest, 'status', lambda value: (value.get('server') or {}).get('joined'))
        kept = (home / 'horizon-player.txt').read_text()
        assert kept.startswith('name:Player') and kept.strip() != 'name:Player', kept
        names.append(kept.strip().removeprefix('name:'))
    assert names[0] != names[1], names
    status = wait_for(endpoint, 'lan status', lambda value: len(value['guests']) == 2)
    assert sorted(g['name'] for g in status['guests']) == sorted(names), status
    # The same home keeps its name: leave and join again as the same profile.
    home, guest = guests[0]
    command(guest, 'disconnect')
    wait_for(endpoint, 'lan status', lambda value: len(value['guests']) == 1)
    command(guest, f'connect 127.0.0.1:{port}')
    wait_for(guest, 'status', lambda value: (value.get('server') or {}).get('joined'))
    assert (home / 'horizon-player.txt').read_text().strip() == f'name:{names[0]}'
    # A guest that really is the host's offline profile is refused, the others stay.
    command(guest, 'disconnect')
    command(guest, f'connect 127.0.0.1:{port} Player')
    refused = wait_for(guest, 'screen', lambda value: (value.get('menu') or {}).get('disconnected'))
    assert 'another location' in refused['menu']['disconnected'], refused
    status = command(endpoint, 'lan status')
    assert [g['name'] for g in status['guests']] == [names[1]], status
    return names


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    args = parser.parse_args()
    client, resources = args.client.resolve(), args.resources.resolve()
    with tempfile.TemporaryDirectory(prefix='horizon-lan-') as directory:
        work = Path(directory)
        endpoint = work / 'control.sock'
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as scanner:
            scanner.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            scanner.bind(('', DISCOVERY_PORT))
            scanner.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                              socket.inet_aton(GROUP) + socket.inet_aton('0.0.0.0'))
            with (work / 'client.log').open('w+') as log:
                process = start_client(client, resources, work, endpoint, log)
                try:
                    scenario(endpoint, scanner)
                    assert process.wait(timeout=20) == 0, 'client exited unsuccessfully'
                except Exception:
                    log.seek(0)
                    print('\n'.join(log.read().splitlines()[-20:]))
                    raise
                finally:
                    stop(process)
        names = guests_scenario(client, resources, work)
    print('LAN lifecycle passed: leaving stops hosting and discovery; reopening stays private')
    print(f'LAN guests passed: default names {names[0]} and {names[1]} joined together and kept; '
          '"Player" refused as the host\'s profile')


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)


def guests_scenario(client, resources, work):
    host_home = work / 'host'
    homes = [work / 'guest-a', work / 'guest-b']
    processes, logs = [], []
    try:
        for home in [host_home, *homes]:
            home.mkdir()
            log = (home / 'client.log').open('w+')
            logs.append(log)
            processes.append(start_client(client, resources, home, home / 'control.sock', log))
        endpoint = host_home / 'control.sock'
        wait_for(endpoint, 'screen', lambda value: value['screen'] == 'title')
        command(endpoint, 'open LanGuests 0 creative')
        wait_for(endpoint, 'status', lambda value: value['player'] is not None
                 and value['pending_chunks'] == 0)
        port = command(endpoint, 'lan open 0')['port']
        return guest_names(endpoint, port, [(home, home / 'control.sock') for home in homes])
    except Exception:
        for log in logs:
            log.seek(0)
            print(f'--- {log.name}')
            print('\n'.join(log.read().splitlines()[-20:]))
        raise
    finally:
        for process in processes:
            stop(process)
        for log in logs:
            log.close()


if __name__ == '__main__':
    main()
