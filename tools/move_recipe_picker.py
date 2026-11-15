#!/usr/bin/env python3
"""Move the existing augmenting-path recipe picker into its own module."""
from pathlib import Path
root = Path(__file__).resolve().parents[1]
p = root / 'crates/horizon-client/src/menu/place.rs'
s = p.read_text()
a = s.index('/// `StackedContents.RecipePicker`')
b = s.index('/// `RecipeBookMenu.PostPlaceAction`', a)
block = s[a:b].replace("struct Picker<'a>", "pub(super) struct Picker<'a>")
for method in ('new', 'try_pick', 'try_pick_all'):
    block = block.replace(f'    fn {method}(', f'    pub(super) fn {method}(')
q = p.parent / 'place/picker.rs'
q.write_text('//! Shared augmenting-path recipe assignment.\nuse super::*;\n\n' + block)
p.write_text(s[:a] + '#[path = "place/picker.rs"]\nmod picker;\nuse picker::Picker;\n\n' + s[b:])
