#!/usr/bin/env python3
"""Pure-move mod screen construction out of the shared gameplay module."""
from pathlib import Path
root = Path(__file__).resolve().parents[1]
p = root / 'crates/horizon-client/src/game/native_hooks/shared.rs'
s = p.read_text()
a = s.index('    /// The container slots (of `size`)')
b = s.index('    /// A data slot of window', a)
q = p.parent / 'shared/book_screen/open.rs'
q.write_text('use super::*;\n\nimpl Game<\'_> {\n' + s[a:b] + '}\n')
p.write_text(s[:a] + s[b:])
p = p.parent / 'shared/book_screen.rs'
s = p.read_text()
p.write_text(s.replace('use super::*;', 'use super::*;\n#[path = "book_screen/open.rs"]\nmod open;', 1))
