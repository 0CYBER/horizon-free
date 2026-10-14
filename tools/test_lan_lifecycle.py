#!/usr/bin/env python3
"""Check LAN shutdown when leaving a singleplayer world through the pause menu.

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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--resources', type=Path, default=ROOT / '.local/horizon-client/26.3')
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
                process = subprocess.Popen([
                    str(client), '--resources', str(resources), '--render-distance', '2',
                    '--test-terrain', '--control-socket', str(endpoint), '--timeout-seconds', '180',
                ], cwd=work, env=dict(os.environ, HORIZON_HOME=str(work), HORIZON_NO_AUDIO='1',
                                     HORIZON_MENU_STYLE='classic'),
                    stdout=log, stderr=subprocess.STDOUT)
                try:
                    scenario(endpoint, scanner)
                    assert process.wait(timeout=20) == 0, 'client exited unsuccessfully'
                except Exception:
                    log.seek(0)
                    print('\n'.join(log.read().splitlines()[-20:]))
                    raise
                finally:
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait(timeout=10)
    print('LAN lifecycle passed: leaving stops hosting and discovery; reopening stays private')


if __name__ == '__main__':
    main()
