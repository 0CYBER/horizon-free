//! Position-owned state follows the payload; observers are invalidated at the owning boundary.
use super::Game;
use horizon_client::{
    block_entities::Pos,
    chunk::PendingContainerLoot,
    world::{BlockEntityExtras, World},
};
use horizon_core::nbt::{Compound, Tag};

/// Mutable position-owned server data borrowed by the level adapter.
pub(super) struct PositionState<'a> {
    pub(super) gateways: &'a mut horizon_client::fast_hash::FxHashMap<
        [i32; 3],
        horizon_client::block_entity_anim::EndGateway,
    >,
    pub(super) hoppers: &'a mut std::collections::HashMap<Pos, super::machines::HopperState>,
}

pub(super) enum PositionChange {
    Carry {
        from: Pos,
        to: Pos,
        partner: Option<Pos>,
    },
    Land {
        at: Pos,
    },
}

pub(super) fn take_position_data(world: &mut World, from: Pos) -> Compound {
    let mut extras = world.block_entity_extras(from);
    if extras.source.is_none() {
        extras.source = super::spill::saved_entity(world, [from.0, from.1, from.2]);
    }
    let mut data = Compound::new();
    let name = world
        .block_names
        .remove(&from)
        .or_else(|| {
            extras
                .source
                .as_ref()
                .and_then(|t| t.get("CustomName"))
                .cloned()
        })
        .or_else(|| match world.block_entities.get(&from) {
            Some(horizon_client::containers::BlockEntity::Mod(e)) => {
                e.nbt.get(&"CustomName".into()).cloned()
            }
            _ => None,
        });
    if let Some(name) = name {
        data.insert("Name".into(), name);
    }
    if let Some(source) = extras.source {
        data.insert("Source".into(), source);
    }
    if let Some(pending) = extras.pending_loot {
        let mut tag = Compound::new();
        pending.write_tags(&mut tag);
        data.insert("PendingLoot".into(), Tag::Compound(tag));
    }
    world.set_block_entity_extras(from, BlockEntityExtras::default());
    data
}
pub(super) fn land_position_data(world: &mut World, at: Pos, data: &Compound) {
    if let Some(name) = data.get(&"Name".into()) {
        world.block_names.insert(at, name.clone());
    }
    let pending_loot = data
        .get(&"PendingLoot".into())
        .and_then(PendingContainerLoot::from_tag);
    let mut source = data.get(&"Source".into()).cloned();
    if let Some(Tag::Compound(fields)) = &mut source {
        for (name, n) in [("x", at.0), ("y", at.1), ("z", at.2)] {
            fields.insert(name.into(), Tag::Int(n));
        }
    }
    world.set_block_entity_extras(
        at,
        BlockEntityExtras {
            source,
            pending_loot,
        },
    );
}
impl Game<'_> {
    /// Reject obsolete mesh entries even if an old asynchronous upload arrives after a move.
    pub(super) fn live_block_entity_render(&self, pos: [i32; 3], state: u32) -> bool {
        self.world.block(pos[0], pos[1], pos[2]).0 == state
            && self.res.block_entity_render().renders(state)
    }
    /// Runs before with_level returns: no menu, ticker, save or packet sees an old owner.
    pub(super) fn finish_piston_position_changes(&mut self, changes: Vec<PositionChange>) {
        if changes.is_empty() {
            return;
        }
        for change in changes {
            match change {
                PositionChange::Carry { from, to, partner } => {
                    for at in [from, to] {
                        let p = [at.0, at.1, at.2];
                        self.be_anims.remove_position(p);
                        if let Some(renderer) = self.renderer.as_mut() {
                            renderer.invalidate_block_entity_position(p);
                        }
                        self.be_anims.present_revision = None;
                        self.world.shulkers.remove(&p);
                        self.world.chest_openers.remove(&at);
                        self.lid_openers.remove(&at);
                        self.far_beacons.remove(&at);
                        self.test_highlights.remove(&at);
                    }
                    self.mod_container_viewers.retain(|(_, at), _| *at != from);
                    if self.jukebox_sounds.contains_key(&from)
                        || self
                            .world
                            .block_entities
                            .carried(&to)
                            .map(|c| &c.entity)
                            .or_else(|| self.world.block_entities.get(&to))
                            .is_some_and(|e| {
                                matches!(e, horizon_client::containers::BlockEntity::Jukebox(_))
                            })
                    {
                        self.jukebox_removed([from.0, from.1, from.2]);
                    }
                    if self
                        .container
                        .is_some_and(|at| at == from || Some(at) == partner)
                    {
                        self.close_container();
                    }
                    let guests: Vec<_> = self
                        .lan
                        .iter()
                        .flat_map(|h| {
                            h.hub
                                .guests
                                .simulation_iter()
                                .filter(move |g| {
                                    g.menu.as_ref().is_some_and(|m| match m.slots {
                                        horizon_client::lan::menu::Slots::Block(at) => at == from,
                                        horizon_client::lan::menu::Slots::Double(a, b) => {
                                            a == from || b == from
                                        }
                                        _ => m.pos == Some(from),
                                    }) || h.menus.get(&g.conn).is_some_and(|m| {
                                        m.station
                                            .as_ref()
                                            .is_some_and(|s| s.pos == [from.0, from.1, from.2])
                                    }) || h.play.get(&g.conn).is_some_and(|p| {
                                        p.lectern.as_ref().is_some_and(|l| l.key == from)
                                    })
                                })
                                .map(|g| g.conn)
                        })
                        .collect();
                    for conn in guests {
                        self.guest_close_menus_and_tell(conn);
                    }
                }
                PositionChange::Land { at } => {
                    if self.world.block_entities.get(&at).is_none() {
                        self.world
                            .set_block_entity_extras(at, BlockEntityExtras::default());
                        self.world.block_names.remove(&at);
                        self.hoppers.remove(&at);
                    }
                    self.mark_block_entity_updated(at);
                }
            }
        }
    }
}
