#!/usr/bin/env python3
"""Pure move of registration state, leaving room for menu-owned declarations."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
source = root / 'crates/horizon-mod-vocab/src/parts/shared.rs'
target = source.parent / 'shared/registration_state.rs'
target.parent.mkdir(exist_ok=True)
text = source.read_text()
blocks = []
for start_marker, end_marker in [
    ('/// What every mod\'s [`Hooks::register`](crate::Hooks::register) attached.', '\n}\n'),
    ('// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.\n/// The game\'s [`Registrar`]', '\n}\n'),
]:
    start = text.index(start_marker)
    end = text.index(end_marker, start) + len(end_marker)
    blocks.append(text[start:end])
    text = text[:start] + text[end:]
target.write_text('//! Registration state, separate from dispatch operations.\nuse super::*;\n\n' + '\n'.join(blocks))
source.write_text(text + '\n#[path = "shared/registration_state.rs"]\nmod registration_state;\npub use registration_state::PartRegistrar;\npub(crate) use registration_state::Installed;\n')
