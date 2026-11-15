//! `Projectile.applyOnProjectileSpawned`: once a projectile is in the level,
//! the `minecraft:projectile_spawned` enchantment effects of the stack it was
//! made from, then of an arrow's weapon, run on it
//! (`EnchantmentHelper.onProjectileSpawned`). One path for every spawn site:
//! bows, crossbows, thrown items, dispensers, mobs and the mod host.
use super::{Game, enchant_effects::look_impulse};
use horizon_client::{
    enchant::{EffectContext, level_value},
    items::ItemStack,
    projectiles::Projectile,
};
use horizon_world::worldgen::value_provider::FloatProvider;
use serde_json::Value;

use super::projectile_spawned_validation::{requirements_supported, valid_ranges};

const COMPONENT: &str = "minecraft:projectile_spawned";

/// An `EnchantmentEntityEffect` this path applies to a new projectile.
#[derive(Clone, Copy)]
enum SpawnedEffect {
    AllOf,
    Ignite,
    Impulse,
    Sound,
    /// `ApplyMobEffect` and `ApplyExhaustion` act on a living entity or a
    /// player only: on a projectile they do nothing and draw nothing.
    Inert,
}

/// The effect `effect` names, None for one this path does not apply yet. A
/// bare type name is minecraft's own.
fn spawned_effect(effect: &Value) -> Option<SpawnedEffect> {
    let kind = effect["type"].as_str()?;
    Some(match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "all_of" => SpawnedEffect::AllOf,
        "ignite" => SpawnedEffect::Ignite,
        "apply_impulse" => SpawnedEffect::Impulse,
        "play_sound" => SpawnedEffect::Sound,
        "apply_mob_effect" | "apply_exhaustion" => SpawnedEffect::Inert,
        _ => return None,
    })
}

/// Whether this path applies `effect`, with everything an `all_of` holds.
fn applied(effect: &Value) -> bool {
    match spawned_effect(effect) {
        Some(SpawnedEffect::AllOf) => effect["effects"]
            .as_array()
            .into_iter()
            .flatten()
            .all(applied),
        Some(_) => true,
        None => false,
    }
}

/// `PlaySoundEffect`'s sound for `level`: its list's entry at `level - 1`,
/// clamped to the list (`Mth.clamp`). An entry is a sound id or a
/// `{sound_id, range}` event.
fn level_sound(effect: &Value, level: u32) -> Option<&str> {
    fn name(v: &Value) -> Option<&str> {
        v.as_str().or_else(|| v["sound_id"].as_str())
    }
    match &effect["sound"] {
        Value::Array(list) => {
            let last = list.len().checked_sub(1)?;
            let index = usize::try_from(level.saturating_sub(1)).unwrap_or(usize::MAX);
            name(&list[index.min(last)])
        }
        one => name(one),
    }
}

/// A `FloatProvider` field of an effect (a number or a provider object).
fn float_provider(v: &Value) -> Option<FloatProvider> {
    FloatProvider::from_json(&v.to_string()).ok()
}

impl Game<'_> {
    /// `Projectile.spawnProjectile` once the shot is aimed: adds `projectile`
    /// to the level, then runs `applyOnProjectileSpawned` with `stack`, the
    /// item it was made from (the held or dispensed stack, an arrow's ammo).
    /// The projectile's id.
    pub(super) fn spawn_projectile(&mut self, projectile: Projectile, stack: &ItemStack) -> u32 {
        let id = self.add_projectile(projectile);
        self.apply_on_projectile_spawned(id, stack);
        id
    }

    /// `Projectile.spawnProjectile` for a mob's shot: an arrow is made from its
    /// ammo, the mobs' other projectiles from fresh stacks with no
    /// enchantments. `Projectile.tick` posts on the first tick, through the
    /// shared arrow-state continuation for ordinary arrows and tridents.
    pub(super) fn spawn_mob_projectile(&mut self, projectile: Projectile) {
        let ammo = projectile.arrow.as_ref().map(|a| a.pickup_item.clone());
        let id = self.add_projectile(projectile);
        if let Some(ammo) = ammo {
            self.apply_on_projectile_spawned(id, &ammo);
        }
    }

    /// `Projectile.applyOnProjectileSpawned` for the projectile `id`, already
    /// in the level: `stack`'s effects, then for an arrow those of the weapon
    /// it was fired from (its own copy, `AbstractArrow.getWeaponItem`) when
    /// that is another item than `stack`.
    pub(super) fn apply_on_projectile_spawned(&mut self, id: u32, stack: &ItemStack) {
        self.on_projectile_spawned(id, stack);
        let weapon = self
            .spawned_projectile(id)
            .and_then(|p| p.arrow.as_ref())
            .and_then(|a| a.fired_from_weapon.as_ref())
            .filter(|w| w.count > 0 && &*w.item != "minecraft:air" && w.item != stack.item)
            .cloned();
        if let Some(weapon) = weapon {
            self.on_projectile_spawned(id, &weapon);
        }
    }

    /// Whether every `projectile_spawned` effect of `stack`'s enchantments is
    /// one this path applies, so a spawn from it loses none.
    pub(super) fn projectile_spawned_effects_applied(&self, stack: &ItemStack) -> bool {
        stack.all_enchantments().iter().all(|(enchantment, _)| {
            world_stations!(self)
                .enchantments
                .get(enchantment)
                .is_none_or(|def| {
                    def.effects[COMPONENT]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .all(|conditional| {
                            requirements_supported(&conditional["requirements"])
                                && valid_ranges(&conditional["effect"])
                                && applied(&conditional["effect"])
                        })
                })
        })
    }

    /// `EnchantmentHelper.onProjectileSpawned`: each enchantment of `stack`,
    /// in the stack's order, runs its effects on the projectile in their
    /// order (`Enchantment.onProjectileSpawned`). An effect's requirements are
    /// tested just before it would apply, against the projectile as `this`
    /// with the level's random (the loot context has no seed of its own); an
    /// effect draws from the projectile's own random.
    fn on_projectile_spawned(&mut self, id: u32, stack: &ItemStack) {
        for (enchantment, level) in stack.all_enchantments().iter() {
            // Copied out only when the enchantment has the component: an
            // effect may change what the next requirement reads.
            let Some(effects) = world_stations!(self)
                .enchantments
                .get(enchantment)
                .and_then(|def| def.effects[COMPONENT].as_array())
                .filter(|effects| !effects.is_empty())
                .cloned()
            else {
                continue;
            };
            for conditional in &effects {
                // The live context cannot answer arbitrary entity predicates. Do not
                // run an unsupported predicate as if it matched, or evaluate invalid ranges.
                if !requirements_supported(&conditional["requirements"])
                    || !valid_ranges(&conditional["effect"])
                {
                    eprintln!(
                        "projectile_spawned refused enchantment {enchantment}: unsupported requirement context or invalid level-value range"
                    );
                    continue;
                }
                let Some(kind) = self.spawned_projectile(id).map(Projectile::entity_type) else {
                    return;
                };
                let ctx = EffectContext {
                    this: Some(kind),
                    items: Some(world_items!(self)),
                    ..Default::default()
                };
                let matches = conditional.get("requirements").is_none_or(|r| {
                    world_stations!(self).enchantments.condition(
                        r,
                        *level,
                        &ctx,
                        &mut self.world.random,
                    )
                });
                if matches {
                    self.apply_spawned_effect(id, &conditional["effect"], *level);
                }
            }
        }
    }

    /// `EnchantmentEntityEffect.apply` on the new projectile `id`, at its
    /// position.
    fn apply_spawned_effect(&mut self, id: u32, effect: &Value, level: u32) {
        let Some(kind) = spawned_effect(effect) else {
            return;
        };
        if let SpawnedEffect::AllOf = kind {
            for e in effect["effects"].as_array().into_iter().flatten() {
                self.apply_spawned_effect(id, e, level);
            }
            return;
        }
        let Some(index) = self.creatures.projectiles.iter().position(|p| p.id == id) else {
            return;
        };
        let sound = self.creatures.projectiles.update(index, |p| match kind {
            SpawnedEffect::Ignite => {
                p.ignite_for_seconds(level_value(&effect["duration"], level));
                None
            }
            // Entity.addDeltaMovement skips a non-finite impulse; the motion
            // is sent either way (syncVelocity).
            SpawnedEffect::Impulse => {
                let impulse = look_impulse(effect, level, p.yaw, p.pitch);
                if impulse.is_finite() {
                    p.velocity = p.velocity.add_vector(impulse);
                }
                p.needs_sync = true;
                None
            }
            // A new projectile is never silent: the volume, then the pitch,
            // from its random, at its position.
            SpawnedEffect::Sound => {
                let sound = level_sound(effect, level)?;
                let volume = float_provider(&effect["volume"])?;
                let pitch = float_provider(&effect["pitch"])?;
                let volume = volume.sample(&mut p.random.0);
                let pitch = pitch.sample(&mut p.random.0);
                Some((
                    sound,
                    [p.position.x, p.position.y, p.position.z],
                    volume,
                    pitch,
                ))
            }
            SpawnedEffect::AllOf | SpawnedEffect::Inert => None,
        });
        if let Some((sound, at, volume, pitch)) = sound {
            self.audio.play_on(
                mod_api::SoundSource::Neutral as usize,
                sound,
                Some(at),
                volume,
                pitch,
            );
        }
    }

    fn spawned_projectile(&self, id: u32) -> Option<&Projectile> {
        self.creatures.projectiles.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// `PlaySoundEffect.apply`: `Mth.clamp(level - 1, 0, size - 1)` picks the
    /// event, so levels past the list keep its last sound.
    #[test]
    fn a_sound_list_is_indexed_by_level_and_clamped() {
        let effect = json!({"sound": ["a:one", {"sound_id": "a:two", "range": 8.0}]});
        let picks: Vec<_> = (0..4)
            .map(|level| super::level_sound(&effect, level))
            .collect();
        assert_eq!(
            picks,
            [Some("a:one"), Some("a:one"), Some("a:two"), Some("a:two")]
        );
        assert_eq!(
            super::level_sound(&json!({"sound": "a:solo"}), 3),
            Some("a:solo")
        );
        assert_eq!(super::level_sound(&json!({"sound": []}), 1), None);
    }

    /// An `all_of` is applied only when everything in it is.
    #[test]
    fn all_of_is_applied_only_when_every_effect_is() {
        let ignite = json!({"type": "ignite", "duration": 1});
        let explode = json!({"type": "minecraft:explode", "radius": 1});
        assert!(super::applied(
            &json!({"type": "minecraft:all_of", "effects": [ignite]})
        ));
        assert!(!super::applied(
            &json!({"type": "minecraft:all_of", "effects": [ignite, explode]})
        ));
        assert!(!super::applied(&json!({"type": "mod:ignite"})));
    }
}
