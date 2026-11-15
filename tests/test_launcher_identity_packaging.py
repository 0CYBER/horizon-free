"""Packaging comparisons against independent pre-change git object fixtures.

Fixtures are verbatim da81e38705 objects, not outputs of the staging function.
"""
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('stage', ROOT / 'tools/stage_launcher_packaging.py')
STAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STAGE)
FIXTURES = ROOT / 'tests/fixtures/launcher-upstream'

class PackagingIdentity(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust for real compiled identity metadata")
    def test_compiled_namespace_metadata_preserves_upstream_and_isolates_fork(self):
        expected = json.loads((FIXTURES / 'identity.json').read_text())
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            # Compile the real identity module, not a Python model of its selection.
            program = '#[allow(dead_code)] #[path = ' + json.dumps(str(ROOT / 'launcher/src/identity.rs')) + '] mod identity;\nfn main() {\n'
            program += ''.join('println!("{}\\t{}", "' + name + '", identity::' + name + ');\n' for name in expected)
            program += '}\n'
            source = temporary / 'metadata.rs'
            source.write_text(program)
            outputs = []
            addresses = temporary / 'libhorizon_addresses.rlib'
            subprocess.run(['rustc', '--edition=2024', '--crate-name=horizon_addresses', '--crate-type=rlib', str(ROOT / 'crates/horizon-addresses/src/lib.rs'), '-o', str(addresses)], check=True)
            build_script = temporary / 'build-script'
            subprocess.run(['rustc', '--edition=2021', str(ROOT / 'launcher/build.rs'), '-o', str(build_script)], check=True)
            for repository in (STAGE.UPSTREAM, STAGE.FORK):
                env = dict(os.environ, HORIZON_LAUNCHER_REPOSITORY=repository,
                           CARGO_CFG_TARGET_OS='linux', CARGO_CFG_TARGET_ENV='gnu',
                           CARGO_MANIFEST_DIR=str(ROOT / 'launcher'))
                directives = subprocess.check_output([str(build_script)], env=env, text=True)
                flags = []
                for line in directives.splitlines():
                    if line.startswith('cargo:rustc-cfg='):
                        flags += ['--cfg', line.removeprefix('cargo:rustc-cfg=')]
                binary = temporary / 'metadata'
                subprocess.run(['rustc', '--edition=2021', str(source), '-o', str(binary), '--extern', f'horizon_addresses={addresses}', *flags], check=True)
                output = subprocess.check_output([str(binary)], text=True)
                outputs.append(dict(line.split('\t', 1) for line in output.splitlines()))
            upstream, fork = outputs
            self.assertEqual(upstream, expected)
            # A fork build presents as the Horizon launcher and opens its data folders and saved
            # accounts, since it updates it; how it is installed keeps the fork's own names.
            shared = {'WINDOW_TITLE', 'DISPLAY_NAME', 'KEYCHAIN_SERVICE', 'LOCATION_FOLDER', 'LAUNCHER_FOLDER', 'GAME_FOLDER'}
            self.assertEqual({name: fork[name] for name in shared}, {name: upstream[name] for name in shared})
            self.assertTrue(all(fork[name] != upstream[name] for name in expected.keys() - shared))
            self.assertEqual(fork['APP_ID'], 'io.github.arismoko.horizonfork.launcher')
            self.assertEqual(fork['EXECUTABLE'], 'horizon-fork-launcher')
            self.assertEqual(fork['INSTALL_KEY'], 'Software\\ArismokoHorizonFork\\Launcher')

    def test_real_workflow_repository_input_and_matrix(self):
        workflow = (ROOT / '.github/workflows/launcher.yml').read_text()
        self.assertIn('HORIZON_LAUNCHER_REPOSITORY: ${{ github.repository }}', workflow)
        self.assertIn('--repository "$GITHUB_REPOSITORY" --tag', workflow)
        original = (ROOT / 'launcher/tests/fixtures-launcher-workflow-upstream.yml').read_text()
        block = original.split('      matrix:\n', 1)[1].split('    runs-on:', 1)[0]
        expected = []
        for line in block.splitlines():
            if line.startswith('          - name: '):
                expected.append({'name': line.split(': ', 1)[1]})
            elif line.startswith('            ') and not line.strip().startswith('#') and ': ' in line:
                key, value = line.strip().split(': ', 1)
                expected[-1][key] = True if value == 'true' else value
        for repository in (STAGE.UPSTREAM, STAGE.FORK):
            output = subprocess.check_output(['python3', str(ROOT / 'tools/launcher_build_matrix.py')],
                                            env=dict(os.environ, GITHUB_REPOSITORY=repository), text=True)
            entries = json.loads(output)['include']
            if repository == STAGE.UPSTREAM:
                self.assertEqual(entries, expected)
            else:
                self.assertEqual([entry['name'] for entry in entries], ['linux', 'macos', 'windows'])
                self.assertEqual([entry['asset'] for entry in entries],
                                 ['horizon-fork-launcher-linux-x86_64.tar.gz', 'horizon-fork-launcher-macos-arm64.zip', 'horizon-fork-launcher-windows-x64.zip'])
                self.assertEqual(entries[1], json.loads(
                    (ROOT / 'tests/fixtures/launcher-fork-macos.json').read_text()))
                self.assertEqual(entries[2]['setup_asset'], 'horizon-fork-launcher-setup.exe')

    def test_upstream_metadata_is_byte_identical_to_base_objects(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / 'packaging'
            STAGE.stage(ROOT / 'packaging', destination, STAGE.UPSTREAM)
            for fixture in FIXTURES.glob('packaging_*'):
                relative = fixture.name.removeprefix('packaging_').replace('_', '/')
                self.assertEqual((destination / relative).read_bytes(), fixture.read_bytes(), relative)
            for name, digest in json.loads((FIXTURES / 'icon-sha256.json').read_text()).items():
                self.assertEqual(hashlib.sha256((destination / name.removeprefix('packaging/')).read_bytes()).hexdigest(), digest, name)

    def test_fork_staging_does_not_modify_source_and_has_distinct_bundle(self):
        source = ROOT / 'packaging'
        before = {p.relative_to(source): p.read_bytes() for p in source.rglob('*') if p.is_file()}
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / 'packaging'
            STAGE.stage(source, destination, STAGE.FORK)
            plist = (destination / 'macos/Info.plist').read_text()
            self.assertIn('<string>io.github.arismoko.horizonfork.launcher</string>', plist)
            self.assertIn('<string>horizon-fork-launcher</string>', plist)
            self.assertNotIn('<string>Horizon</string>', plist)
            self.assertEqual((destination / 'icons/horizon.icns').read_bytes(),
                             (source / 'icons/fork/launcher.icns').read_bytes())
            script = (destination / 'macos/package.sh').read_text()
            self.assertIn('identity=-', script)
            self.assertIn('zip=horizon-fork-launcher-macos-$arch.zip', script)
            self.assertIn('dmg=$out/horizon-fork-launcher-macos-$arch.dmg', script)
            self.assertIn('\"Horizon Fork Launcher (arismoko)\" \"$dmg\"', script)
            desktop = (destination / 'linux/io.github.arismoko.horizonfork.launcher.desktop').read_text()
            self.assertIn('Name=Horizon Fork Launcher (arismoko)\n', desktop)
            self.assertIn('Exec=horizon-fork-launcher\n', desktop)
            self.assertIn('StartupWMClass=io.github.arismoko.horizonfork.launcher\n', desktop)
            for script in ('linux/appimage.sh', 'macos/package.sh'):
                subprocess.run(['bash', '-n', str(destination / script)], check=True)
            self.assertNotEqual((destination / 'icons/horizon.ico').read_bytes(), (source / 'icons/horizon.ico').read_bytes())
        self.assertEqual(before, {p.relative_to(source): p.read_bytes() for p in source.rglob('*') if p.is_file()})

if __name__ == '__main__':
    unittest.main()
