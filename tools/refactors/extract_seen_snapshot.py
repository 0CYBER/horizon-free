#!/usr/bin/env python3
"""Keep the canonical mobs::Seen path while moving its source out of mobs.rs."""
from pathlib import Path

source = Path('crates/horizon-client/src/mobs.rs')
destination = source.parent / 'mobs/seen.rs'
text = source.read_text()
start = '/// A snapshot of another entity mobs can see this tick.\n'
end = 'impl Mob {\n    /// The name used by command replies:'
assert text.count(start) == text.count(end) == 1
assert not destination.exists()
a, b = text.index(start), text.index(end)
destination.write_text('// Included in mobs.rs: the snapshot retains its canonical mobs::Seen path.\n' + text[a:b])
source.write_text(text[:a] + 'include!("mobs/seen.rs");\n\n' + text[b:])
