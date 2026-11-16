#!/usr/bin/env python3
"""Extract the small motion ID helpers so game.rs can accept a level delegation."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
path = root / 'crates/horizon-client/src/game.rs'
source = path.read_text()
start = source.index("/// A block entity's `mark_motion` id:")
end = source.index('/// `FallingBlockRenderer.shouldRender`', start)
helpers = source[start:end]
assert helpers.count('\nfn ') == 2
out = root / 'crates/horizon-client/src/game/blocks/motion_ids.rs'
out.write_text('use super::Game;\n\n' + helpers.replace('\nfn ', '\npub(super) fn '))
path.write_text((source[:start] + source[end:]).replace('use key_codes::{', 'use motion_ids::{motion_at, motion_block_entity};\nuse key_codes::{', 1))
