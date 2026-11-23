//! What a client's new player takes from the old one when a `ClientboundRespawnPacket`
//! replaces it (`ClientPacketListener.handleRespawn`): `AttributeMap.assignAllValues` when the
//! packet keeps the attribute modifiers (a level change, through a portal or not: the worn
//! armour's among them), `assignBaseValues` after a death. The new player is made once its
//! position arrives, so what the server sends for it in between waits here, as vanilla's new
//! `LocalPlayer` already holds it.
use super::super::{Game, Result};
use horizon_client::{lan::attributes::apply, motion::PlayerMotion};
use horizon_protocol::packets::play::AttributeSnapshot;
use horizon_world::attributes::{Instance, entity::Catalog};

/// The respawn in progress (`Game::portals`).
#[derive(Default)]
pub(in crate::game) struct RespawnAttributes {
    /// A respawn packet came: whether it keeps the modifiers.
    keep_modifiers: Option<bool>,
    carried: Option<Carried>,
    /// `UpdateAttributes` of the player's own entity that came while there was no player and no
    /// respawn to carry from: a login's, whose `LocalPlayer` vanilla already has. Kept until the
    /// player is made or the next login, as [`hold`] keeps them.
    login_early: Vec<AttributeSnapshot>,
}

struct Carried {
    keep_modifiers: bool,
    instances: Vec<(String, Instance)>,
    /// `UpdateAttributes` of the player's own entity that came before its new player, as
    /// [`hold`] keeps them.
    early: Vec<AttributeSnapshot>,
}

/// Holds `values` for a player not made yet, as its `LocalPlayer` would end up: each snapshot
/// replaces the attribute's base and modifiers (`handleUpdateAttributes`), so only the latest one
/// of each attribute is kept, and one the catalog doesn't know, which `apply` would skip, is
/// dropped. That bounds what a server withholding the position can make the client hold by the
/// attribute registry.
fn hold(early: &mut Vec<AttributeSnapshot>, catalog: &Catalog, values: Vec<AttributeSnapshot>) {
    for value in values {
        if !catalog
            .attributes()
            .any(|(id, _)| id.registry_id() == value.attribute)
        {
            continue;
        }
        match early.iter_mut().find(|held| held.attribute == value.attribute) {
            Some(held) => *held = value,
            None => early.push(value),
        }
    }
}

impl RespawnAttributes {
    /// `KEEP_ATTRIBUTE_MODIFIERS` of the respawn packet being handled.
    pub(in crate::game) fn respawn(&mut self, keep_modifiers: bool) {
        self.keep_modifiers = Some(keep_modifiers);
    }
}

impl Game<'_> {
    /// The old player goes (`handleRespawn`, `handleLogin`): after a respawn its attributes are
    /// kept for the next one; a login starts from nothing.
    pub(in crate::game) fn drop_remote_player(&mut self) {
        let respawn = &mut self.portals.respawn_attributes;
        match (respawn.keep_modifiers.take(), self.player.as_ref()) {
            (Some(keep_modifiers), Some(old)) => {
                respawn.carried = Some(Carried {
                    keep_modifiers,
                    instances: old
                        .attributes
                        .instances()
                        .map(|(_, key, instance)| (key.name().to_owned(), instance.clone()))
                        .collect(),
                    early: Vec::new(),
                });
            }
            // A respawn before the position: a first respawn's attributes are still the ones to
            // carry, and with none a login's early ones still reach the new player.
            (Some(_), None) => {}
            (None, _) => {
                respawn.carried = None;
                respawn.login_early.clear();
            }
        }
        self.remove_local_player_body();
    }

    /// `ClientPacketListener.handleUpdateAttributes` of the player's own entity.
    pub(in crate::game) fn apply_own_attributes(&mut self, values: Vec<AttributeSnapshot>) {
        if let Some(player) = self.player.as_mut() {
            apply(&mut player.attributes, &values);
        } else {
            let respawn = &mut self.portals.respawn_attributes;
            let early = match respawn.carried.as_mut() {
                Some(carried) => &mut carried.early,
                None => &mut respawn.login_early,
            };
            hold(early, &self.motion.attributes, values);
        }
    }

    /// The new player of a respawn takes the old one's attributes, then what the server sent.
    pub(in crate::game) fn take_carried_attributes(
        &mut self,
        player: &mut PlayerMotion,
    ) -> Result<()> {
        let login_early = std::mem::take(&mut self.portals.respawn_attributes.login_early);
        apply(&mut player.attributes, &login_early);
        let Some(carried) = self.portals.respawn_attributes.carried.take() else {
            return Ok(());
        };
        for (name, old) in &carried.instances {
            let Some(key) = player.attributes.key(name) else {
                continue;
            };
            let Some(handle) = player
                .attributes
                .get_instance(Some(key))
                .map_err(|e| format!("movement attribute: {e:?}"))?
            else {
                continue;
            };
            player
                .attributes
                .edit(&handle, |value, observer| {
                    if carried.keep_modifiers {
                        // `AttributeInstance.replaceFrom`: the base and every modifier.
                        value.replace_from(old, observer)
                    } else {
                        value.set_base_value(old.base_value(), observer)
                    }
                })
                .map_err(|e| format!("movement attribute: {e:?}"))?;
        }
        apply(&mut player.attributes, &carried.early);
        Ok(())
    }
}

#[cfg(test)]
#[path = "respawn_attributes/tests.rs"]
mod tests;
