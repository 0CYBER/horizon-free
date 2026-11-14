"""Move the shared wire fixture listener implementation into a discovered private module."""
from pathlib import Path

root = Path(__file__).resolve().parents[2]
p = root / 'crates/horizon-mod-wasm/wire-loopback/tests/wire_loopback.rs'
s = p.read_text()
start = s.index('    struct Mod;\n', s.index('fn __horizon_mod_hooks()'))
end = s.index('    Box::new(Mod)', start)
block = s[start:end]
fixture = p.parent / 'wire_loopback/interaction_fixture.rs'
fixture.write_text('use super::*;\n\n' + '\n'.join(line[4:] if line.startswith('    ') else line for line in block.splitlines()).replace('struct Mod;', 'pub(super) struct Mod;', 1) + '\n')
p.write_text(s[:start] + s[end:].replace('Box::new(Mod)', 'Box::new(interaction_fixture::Mod)', 1))
