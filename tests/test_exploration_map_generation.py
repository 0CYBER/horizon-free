"""The console-only generation server must not race another process for a probed port."""
import importlib
import json
from pathlib import Path
import socket
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1] / "tools"

# Replace Minecraft, not the harness's directory setup, process/console driver or NBT reader.
# This process really binds the configured port and acknowledges the real Commands barriers.
SERVER = r'''
from pathlib import Path
import socket
import struct
import sys

props = dict(line.split('=', 1) for line in Path('server.properties').read_text().splitlines())
with socket.socket() as listener:
    listener.bind(('127.0.0.1', int(props['server-port'])))
    listener.listen()
    Path('bound-port').write_text(str(listener.getsockname()[1]))
    def string(value):
        encoded = value.encode()
        return struct.pack('>H', len(encoded)) + encoded
    def named(kind, name, value):
        return bytes([kind]) + string(name) + value
    generator = named(8, 'type', string('minecraft:noise')) + b'\0'
    overworld = named(10, 'generator', generator) + b'\0'
    dimensions = named(10, 'minecraft:overworld', overworld) + b'\0'
    settings = (named(4, 'seed', struct.pack('>q', 42))
                + named(1, 'generate_structures', b'\1')
                + named(10, 'dimensions', dimensions) + b'\0')
    Path('world/data/minecraft').mkdir(parents=True)
    Path('world/level.dat').write_bytes(named(10, '', b'\0'))
    Path('world/data/minecraft/world_gen_settings.dat').write_bytes(
        named(10, '', named(10, 'data', settings) + b'\0'))
    print('Done (socket fixture)', flush=True)
    serial = 0
    for command in sys.stdin:
        command = command.strip()
        if command == 'stop':
            break
        if command.startswith('data modify storage h068:barrier '):
            serial = int(command.rsplit(' ', 1)[1])
        elif command == 'data get storage h068:barrier value':
            print(f'h068:barrier contents: {serial}', flush=True)
'''


class ExplorationMapGenerationTests(unittest.TestCase):
    def test_generation_succeeds_when_a_probed_port_is_taken(self):
        with patch.object(sys, "path", [str(TOOLS), *sys.path]):
            harness = importlib.import_module("test_exploration_map_command_parity")
        server_class = harness.net.Server

        def start_server(*args):
            server = server_class(*args)
            self.addCleanup(server.process.stdout.close)
            self.addCleanup(server.process.stdin.close)
            return server

        with tempfile.TemporaryDirectory() as scratch, socket.socket() as occupied:
            occupied.bind(("127.0.0.1", 0))
            occupied.listen()
            taken = occupied.getsockname()[1]
            build = Path(scratch)
            with patch.object(harness.net, "free_port", return_value=taken), \
                    patch.object(harness.net, "Server", side_effect=start_server):
                harness.generate([sys.executable, "-u", "-c", SERVER], build)
            bound = int((build / "generate/bound-port").read_text())
            self.assertGreater(bound, 0)
            self.assertNotEqual(bound, taken)
            self.assertEqual(occupied.getsockname()[1], taken)
            settings = json.loads((build / "initial-world-gen-settings.json").read_text())
            self.assertEqual(settings["body"]["data"]["seed"], ["l", 42])
            commands = json.loads((build / "generate-commands.json").read_text())
            self.assertEqual(commands[-1]["command"], "save-all flush")
            self.assertTrue(all(row["receipt_at"] is not None for row in commands))


if __name__ == "__main__":
    unittest.main()
