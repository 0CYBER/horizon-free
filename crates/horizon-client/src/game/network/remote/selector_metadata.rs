use super::*;

/// The hurt sound of a `ClientboundDamageEventPacket`'s damage type, by its
/// synchronized registry id: `Player.getHurtSound` picks the type's effects.
pub(super) fn hurt_sound_for(
    registries: &Dynamic,
    damage: &horizon_world::damage_source::Catalog,
    source_type: i32,
) -> &'static str {
    if let Some(data) = registries.data("minecraft:damage_type", source_type) {
        let effects = data.get("effects").and_then(|t| match t {
            Tag::String(effects) => effects.to_string().ok(),
            _ => None,
        });
        return horizon_client::health::hurt_sound(effects.as_deref().unwrap_or("hurt"));
    }
    registries
        .name("minecraft:damage_type", source_type)
        .and_then(|name| damage.id(name).and_then(|id| damage.definition(id)))
        .map_or(horizon_client::health::hurt_sound("hurt"), |d| {
            horizon_client::health::hurt_sound(&d.effects)
        })
}

impl Game<'_> {
    pub(super) fn receive_selector_tokens(&mut self, data: &[u8]) -> Result<()> {
        let Some(remote) = self.remote.as_mut() else {
            return Ok(());
        };
        if !remote.selector_metadata_supported {
            return Ok(());
        }
        match horizon_client::lan::commands::selector_sync::decode(data)? {
            horizon_client::lan::commands::selector_sync::Snapshot::Tokens(tokens) => {
                self.ui.registries.selector_tokens = tokens;
            }
            horizon_client::lan::commands::selector_sync::Snapshot::Unsupported(revision) => {
                remote.selector_metadata_supported = false;
                self.ui.registries.selector_tokens.clear();
                log::warn!(
                    "Horizon: selector metadata revision {revision} is unsupported; tokens disabled for this connection"
                );
            }
        }
        Ok(())
    }
}
