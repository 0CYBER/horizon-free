"""Scripted placement of arismoko/horizon#2533 additions; --moves performs only approved helper moves."""
import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / 'crates/horizon-client/src'


def change(path, old, new):
    text = path.read_text()
    assert text.count(old) == 1, (path, old[:80], text.count(old))
    path.write_text(text.replace(old, new))


def function(path, name, remove=True):
    text = path.read_text()
    import re
    match = re.search(r'^    (?:pub[^\n]* )?fn ' + name + r'\(', text, re.M)
    assert match, (path, name)
    start = match.start()
    # Move attached documentation with the function.
    while start and text[:start].splitlines()[-1].lstrip().startswith('///'):
        start = text.rfind('\n', 0, start - 1) + 1
    opening = text.index('{', match.end())
    depth, end = 1, opening + 1
    while depth:
        depth += (text[end] == '{') - (text[end] == '}')
        end += 1
    end += text[end:end + 1] == '\n'
    result = text[start:end]
    if remove:
        path.write_text(text[:start] + text[end:])
    return result


def append_impl(path, body, target="Game<'_>"):
    with path.open('a') as out:
        out.write(f'\nimpl {target} {{\n{body}}}\n')


def additions():
    game = SRC / 'game.rs'
    change(game, '    local_player_residency: horizon_client::join_seq::residency::member::Member,\n', '')
    change(game, '    knots: horizon_client::join_seq::JoinList<leads::Knot>,\n', '')
    change(game, '    /// Journal-fed residency across active and parked server dimensions.\n    entity_residency: Box<RefCell<horizon_client::join_seq::residency::table::EntityTable>>,', '    resident_entities: resident_state::ResidentEntities,')
    change(game, '            local_player_residency: Default::default(),\n', '')
    change(game, '            knots: Default::default(),\n', '')
    change(game, '            entity_residency: Box::new(RefCell::new(\n                horizon_client::join_seq::residency::table::EntityTable::new(Default::default()),\n            )),', '            resident_entities: Default::default(),')
    for path in [game, *sorted((SRC / 'game').rglob('*.rs'))]:
        text = path.read_text()
        for old, new in [('.entity_residency', '.resident_entities.table'), ('.local_player_residency', '.resident_entities.local_player'), ('.knots', '.resident_entities.knots')]:
            text = text.replace(old, new)
        path.write_text(text)
    state = SRC / 'game/entities/resident_state.rs'
    assert not state.exists()
    state.write_text('''//! Game-owned residency state; no second membership directory.
use super::*;
#[derive(Default)]
pub(super) struct ResidentEntities {
    pub table: Box<RefCell<horizon_client::join_seq::residency::table::EntityTable>>,
    pub local_player: horizon_client::join_seq::residency::member::Member,
    pub knots: horizon_client::join_seq::JoinList<leads::Knot>,
}
''')
    # EntityTable has no Default: initialize its single journal explicitly.
    change(state, '#[derive(Default)]\n', '')
    append_impl(state, '''    fn default() -> Self {
        Self {
            table: Box::new(RefCell::new(horizon_client::join_seq::residency::table::EntityTable::new(Default::default()))),
            local_player: Default::default(), knots: Default::default(),
        }
    }
''', 'Default for ResidentEntities')
    dragon = SRC / 'game/mobs/dragon.rs'
    change(dragon, 'mod save_lending;\n', 'mod save_lending;\n#[path = "dragon/residency.rs"]\nmod residency;\n')
    change(dragon, 'pub(in crate::game) struct Dragon {\n    /// Entity-owned registration metadata; content clones are detached.\n    pub(super) attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,', 'pub(in crate::game) struct Dragon {\n    pub(super) residency: residency::DragonResidency,')
    change(dragon, '    pub(super) part_uuids: [[i32; 4]; 8],\n', '')
    change(dragon, '    pub(super) fn new(position: Vec3, yaw: f32, seed: i64) -> Self {\n        Self {\n            attachment_lifetime: Default::default(),', '    pub(super) fn new(position: Vec3, yaw: f32, seed: i64) -> Self {\n        Self {\n            residency: Default::default(),')
    change(dragon, "            // Parts construct their own entity random; these UUID draws never\n            // advance the level's or dragon's ongoing stream.\n            part_uuids: std::array::from_fn(|_| {\n                horizon_client::save::insecure_uuid(&mut random_source::entity())\n            }),\n", '')
    change(dragon, 'horizon_mod_runtime::impl_attachment_lifetime!(Breath, Dragon);', 'horizon_mod_runtime::impl_attachment_lifetime!(Breath);')
    body = function(dragon, 'resident_dragon_part', remove=False)
    delegate = body[:body.index('{') + 1] + '\n        self.resident_dragon_part_source(id)\n    }\n'
    change(dragon, body, delegate)
    crystal = '        super::entity_residency::assign_resident_ids!(\n            self.crystals,\n            c,\n            &mut self.next_entity,\n            None\n        );'
    change(dragon, crystal, '        self.assign_resident_crystal_ids();')
    child = SRC / 'game/mobs/dragon/residency.rs'
    assert not child.exists()
    child.write_text('''//! Dragon-owned identity and attachment metadata.
use super::*;
pub(in crate::game) struct DragonResidency {
    pub(super) lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    pub(in crate::game) part_uuids: [[i32; 4]; 8],
}
impl Default for DragonResidency {
    fn default() -> Self {
        Self {
            lifetime: Default::default(),
            // Independent entity streams, never the level or dragon's ongoing RNG.
            part_uuids: std::array::from_fn(|_| horizon_client::save::insecure_uuid(&mut random_source::entity())),
        }
    }
}
impl horizon_mod_runtime::attachments::HasAttachmentLifetime for Dragon {
    fn attachment_lifetime(&mut self) -> &mut horizon_mod_runtime::attachments::AttachmentLifetime {
        &mut self.residency.lifetime
    }
}
''')
    append_impl(child, body.replace('pub(super) fn resident_dragon_part(', 'pub(in crate::game) fn resident_dragon_part_source(') + '\n    pub(super) fn assign_resident_crystal_ids(&mut self) {\n' + crystal + '\n    }\n')
    for path in (SRC / 'game').rglob('*.rs'):
        if path == child:
            continue
        text = path.read_text()
        path.write_text(text.replace('.part_uuids', '.residency.part_uuids'))
    creatures = SRC / 'game/mobs/creatures/mod.rs'
    change(creatures, '                Finalized::Cancelled => {\n                    self.forget_entity(mob.id);\n                    return None;\n                }', '                Finalized::Cancelled => return self.discard_unjoined_mob(mob.id),')
    change(creatures, 'mod borrowed_tick;', 'mod borrowed_tick;\nmod residency;')
    child = SRC / 'game/mobs/creatures/residency.rs'
    assert not child.exists()
    child.write_text('''//! Cleanup of constructor state that never joined a resident owner.
use super::*;
impl Game<'_> {
    pub(super) fn discard_unjoined_mob<T>(&mut self, id: u32) -> Option<T> {
        self.forget_entity(id);
        None
    }
}
''')
    lan = SRC / 'game/network/lan/mod.rs'
    change(lan, 'mod event_timings;', 'mod event_timings;\nmod residency;')
    binding = '        hub.guests\n            .bind_residency(&self.resident_entities.table.borrow().producer(), |id| {\n                crate::game::Dim::from_id(id).key()\n            });'
    change(lan, binding, '        self.bind_guest_residency(&mut hub);')
    child = SRC / 'game/network/lan/residency.rs'
    assert not child.exists()
    child.write_text('//! Bind actual network members to the existing residency journal.\nuse super::*;\n')
    append_impl(child, '    pub(super) fn bind_guest_residency(&self, hub: &mut Hub) {\n' + binding + '\n    }\n')


def moves():
    # Exactly one minimal existing helper per remaining over-cap file.
    for source, destination, name in [
        ('game.rs', 'game/entities/resident_state.rs', 'drop_plain'),
        ('game/mobs/creatures/mod.rs', 'game/mobs/creatures/residency.rs', 'clear_pending_mob_actions'),
        ('game/mobs/dragon.rs', 'game/mobs/dragon/residency.rs', 'tracked_special_box'),
        ('game/network/lan/mod.rs', 'game/network/lan/residency.rs', 'lan_synced_from'),
    ]:
        body = function(SRC / source, name)
        # Preserve the original visibility from its new child module.
        if source != 'game.rs':
            body = body.replace('pub(super) fn', 'pub(in crate::game) fn')
        append_impl(SRC / destination, body)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--moves', action='store_true')
    args = parser.parse_args()
    moves() if args.moves else additions()
