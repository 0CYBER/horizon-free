# Offline YAML parser

`pyyaml-6.0.3.zip` contains the 17 unmodified pure-Python `yaml/*.py` modules and the upstream MIT license from PyYAML 6.0.3. No compiled extension, wheel metadata, bytecode or downloaded executable is included. Python imports the archive directly; nothing is extracted or installed.

- Archive: 41,923 bytes; uncompressed source plus license: 218,607 bytes.
- SHA256: `94ac21b4343e15647b3b5b5bd594aab5b7d3cd8d2ce17db15acb7260d489b6fc`.
- Source wheel URL and SHA256 are pinned in `../vendor_pyyaml.py`.
- License: MIT; full original copyright and permission notice are inside the archive as `LICENSE`.

To reproduce, download that pinned wheel separately, then run `python3 tools/ci/vendor_pyyaml.py path/to/wheel`. The generator verifies the upstream wheel checksum before selecting source modules, and writes fixed archive timestamps/permissions and sorted source entries. Confirm the resulting archive checksum against both this document and the guard's pinned value. Review upstream source and license changes before updating.

The repository guard verifies this archive checksum before importing. Missing, corrupt or unloadable parser fails closed in CI and pre-push, even if a system YAML package exists. There is no fallback to site packages and no parser download during CI or hook execution. Tests exercise successful parsing without site packages and reject missing or corrupt archives. The same parser is used by lint, tooling tests and hooks.
