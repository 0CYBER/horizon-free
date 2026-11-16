//! A mob the game has made but not yet added to a level, lent to one hook call
//! (`finalize_spawn`'s `mob`, docs/modding/src/how-to/world/spawning.md): the lend,
//! the handle's reads (`UnspawnedMobRef`), the scoped editor handed out with it
//! (`UnspawnedMobEdits`), and the spawn group data at the API boundary
//! (`SpawnGroup`).
//!
//! A lend copies the mob into `Game::lent_mobs` for the call and copies it back
//! after, so it works where the caller holds the level shared (natural
//! spawning's and the spawners' passes): reads and edits reach that copy only,
//! never the level. A handle from a finished call, or another lend's, finds no
//! copy and is refused with `Expired`.

use super::Game;
use super::api_values::TryIntoEngine;
use super::living_attributes::{AttributeWrite, collect_equipment, write_unspawned_attribute};
use horizon_client::mobs::finalize::{Ageable, Group};
use horizon_client::mobs::{Mob, equine, llama, render, tame};
use horizon_client::text::Text;
use horizon_world::attribute_map::ValueKind;
use horizon_world::attributes::Operation;
use mod_api::{
    AgeableGroup, AttributeModifier, AttributeOperation, EntityRef, EquipmentSlot, Identifier,
    ItemStack, SpawnGroup, TropicalVariant, UnspawnedMobEdits, UnspawnedMobError, UnspawnedMobRef,
    WorldEpoch,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// The most tags an entity holds (`Entity.addTag`).
const MAX_TAGS: usize = 1024;

/// Lends are numbered across every `Game` of the process, so no two calls'
/// handles are ever equal.
static NEXT_LEND: AtomicU64 = AtomicU64::new(1);

/// The mobs lent to the hook calls running now, the latest last.
#[derive(Default)]
pub(in crate::game) struct LentMobs(Vec<Lent>);

#[cfg(test)]
impl LentMobs {
    /// Whether no lend is open.
    pub(in crate::game) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One lend: the handle's numbers, what every lend answers, and the copy of the
/// mob when the game keeps one (not for the ender dragon).
struct Lent {
    handle: UnspawnedMobRef,
    kind: String,
    position: [f64; 3],
    mob: Option<Mob>,
}

/// Ends a lend when it drops, after the call or while a listener's panic
/// unwinds: the lend is withdrawn, so its handle answers `Expired`, and the
/// lent mob moves back into the caller's slot.
struct Withdraw<'a, 'g, 'm> {
    game: &'a Game<'g>,
    handle: UnspawnedMobRef,
    slot: Option<&'m mut Mob>,
}

impl Drop for Withdraw<'_, '_, '_> {
    fn drop(&mut self) {
        let lent = {
            let mut lent = self.game.lent_mobs.borrow_mut();
            let at = lent.0.iter().rposition(|l| l.handle == self.handle);
            at.map(|at| lent.0.remove(at))
        };
        if let (
            Some(slot),
            Some(Lent {
                mob: Some(back), ..
            }),
        ) = (self.slot.take(), lent)
        {
            *slot = back;
        }
    }
}

/// The editor a lend hands out: it reaches only the lent copies, through the
/// game it was made from.
struct Edits<'a, 'g> {
    game: &'a Game<'g>,
}

impl Game<'_> {
    /// Lends `mob` (or, with `None`, only its `kind` and `position`) to `call`,
    /// then takes it back with whatever the call changed. `call` gets the
    /// handle and the editor; both are dead once it returns.
    ///
    /// The mob itself moves into the lend and back, so state a copy leaves
    /// out (a mod goal's own object) is kept; the caller's slot holds a copy
    /// meanwhile, which nothing reads. A listener that panics still ends the
    /// lend and puts the mob back ([`Withdraw`]).
    pub(in crate::game) fn lend_unspawned<R>(
        &self,
        kind: &str,
        position: [f64; 3],
        mob: Option<&mut Mob>,
        call: impl FnOnce(UnspawnedMobRef, &dyn UnspawnedMobEdits) -> R,
    ) -> R {
        let handle =
            UnspawnedMobRef::new(self.lend_epoch(), NEXT_LEND.fetch_add(1, Ordering::Relaxed));
        let mob = mob.map(|slot| {
            let stand_in = slot.clone();
            (std::mem::replace(slot, stand_in), slot)
        });
        let (lent, slot) = match mob {
            Some((lent, slot)) => (Some(lent), Some(slot)),
            None => (None, None),
        };
        self.lent_mobs.borrow_mut().0.push(Lent {
            handle,
            kind: kind.to_owned(),
            position,
            mob: lent,
        });
        let _withdraw = Withdraw {
            game: self,
            handle,
            slot,
        };
        call(handle, &Edits { game: self })
    }

    /// The world a lend's handles belong to; a game with none open lends
    /// under epoch 0, which no open world has.
    pub(in crate::game) fn lend_epoch(&self) -> WorldEpoch {
        self.native_lifecycle.epoch().unwrap_or(WorldEpoch::new(0))
    }

    /// Runs `read` on the lend `handle` names.
    fn with_lent<T>(
        &self,
        handle: UnspawnedMobRef,
        read: impl FnOnce(&Lent) -> Result<T, UnspawnedMobError>,
    ) -> Result<T, UnspawnedMobError> {
        let lent = self.lent_mobs.borrow();
        read(
            lent.0
                .iter()
                .find(|l| l.handle == handle)
                .ok_or(UnspawnedMobError::Expired)?,
        )
    }

    /// Runs `read` on the mob the lend `handle` names.
    fn with_lent_mob<T>(
        &self,
        handle: UnspawnedMobRef,
        read: impl FnOnce(&Mob) -> Result<T, UnspawnedMobError>,
    ) -> Result<T, UnspawnedMobError> {
        self.with_lent(handle, |l| {
            read(l.mob.as_ref().ok_or(UnspawnedMobError::Unsupported)?)
        })
    }

    /// Runs `edit` on the mob the lend `handle` names.
    fn edit_lent_mob<T>(
        &self,
        handle: UnspawnedMobRef,
        edit: impl FnOnce(&Game<'_>, &mut Mob) -> Result<T, UnspawnedMobError>,
    ) -> Result<T, UnspawnedMobError> {
        let mut lent = self.lent_mobs.borrow_mut();
        let l = lent
            .0
            .iter_mut()
            .find(|l| l.handle == handle)
            .ok_or(UnspawnedMobError::Expired)?;
        edit(self, l.mob.as_mut().ok_or(UnspawnedMobError::Unsupported)?)
    }

    /// The mob's entity type (`minecraft:zombie`).
    pub(super) fn unspawned_mob_kind(
        &self,
        mob: UnspawnedMobRef,
    ) -> Result<Identifier, UnspawnedMobError> {
        self.with_lent(mob, |l| {
            Identifier::parse(&l.kind).map_err(|_| UnspawnedMobError::Unsupported)
        })
    }

    /// Where the mob will join the level.
    pub(super) fn unspawned_mob_position(
        &self,
        mob: UnspawnedMobRef,
    ) -> Result<[f64; 3], UnspawnedMobError> {
        self.with_lent(mob, |l| Ok(l.position))
    }

    /// The stack in `slot` (`LivingEntity.getItemBySlot`) and its count; `None`
    /// for an empty slot.
    pub(super) fn unspawned_mob_equipment(
        &self,
        mob: UnspawnedMobRef,
        slot: EquipmentSlot,
    ) -> Result<Option<horizon_client::items::ItemStack>, UnspawnedMobError> {
        self.with_lent_mob(mob, |m| Ok(m.equipment_stacks()[slot as usize].clone()))
    }

    /// An attribute's base value, before modifiers (`AttributeMap.getBaseValue`).
    /// `Unsupported` for an attribute the mob's type does not have.
    pub(super) fn unspawned_mob_attribute_base(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
    ) -> Result<f64, UnspawnedMobError> {
        self.unspawned_attribute(mob, attribute, ValueKind::Base)
    }

    /// An attribute's value with its modifiers, its equipment's included
    /// (`AttributeMap.getValue`). `Unsupported` for an attribute the mob's type
    /// does not have.
    pub(super) fn unspawned_mob_attribute_value(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
    ) -> Result<f64, UnspawnedMobError> {
        self.unspawned_attribute(mob, attribute, ValueKind::Computed)
    }

    fn unspawned_attribute(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
        kind: ValueKind,
    ) -> Result<f64, UnspawnedMobError> {
        let catalog = &self.motion.attributes;
        self.with_lent_mob(mob, |m| {
            m.extra
                .live
                .read(catalog, &m.kind.id, &attribute.to_string(), kind, || {
                    horizon_client::entity_nbt::attribute_entries(m)
                        .into_iter()
                        .map(|(_, entry)| entry)
                        .collect()
                })
                .ok_or(UnspawnedMobError::Unsupported)
        })
    }

    /// The tags a mod gave the mob so far (`Entity.getTags`).
    pub(super) fn unspawned_mob_tags(
        &self,
        mob: UnspawnedMobRef,
    ) -> Result<Vec<String>, UnspawnedMobError> {
        self.with_lent_mob(mob, |m| Ok(m.extra.spawn_tags.clone()))
    }

    /// The mob's custom name, if it has one.
    pub(super) fn unspawned_mob_custom_name(
        &self,
        mob: UnspawnedMobRef,
    ) -> Result<Option<Text>, UnspawnedMobError> {
        self.with_lent_mob(mob, |m| Ok(m.custom_name_text.clone()))
    }

    /// The tags a mod gave the mobs from index `from` on before they joined
    /// move to where the level keeps a joined entity's tags.
    pub(super) fn join_spawn_tags(&mut self, from: usize) {
        for i in from..self.mobs.len() {
            if self.mobs[i].extra.spawn_tags.is_empty() {
                continue;
            }
            let given = std::mem::take(&mut self.mobs[i].extra.spawn_tags);
            let target = super::commands::Target::Mob(self.mobs[i].id);
            let mut tags = self.tags_of(target);
            for tag in given {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
            self.set_entity_tags(target, &tags);
        }
    }
}

/// The pack's group data as the API's, a school's leader a handle of `epoch`;
/// None where a value has no API name.
pub(in crate::game) fn native_spawn_group(group: &Group, epoch: WorldEpoch) -> Option<SpawnGroup> {
    let named = |names: &[&'static str], i: u8| names.get(usize::from(i)).map(|n| (*n).to_owned());
    Some(match group {
        Group::Ageable(a) => SpawnGroup::Ageable(native_ageable(a)),
        Group::Zombie { baby, jockey, husk } => SpawnGroup::Zombie {
            baby: *baby,
            jockey: *jockey,
            tried_camel: *husk,
        },
        Group::Spider(effect) => SpawnGroup::Spider {
            effect: match effect {
                Some(e) => Some(Identifier::parse(e).ok()?),
                None => None,
            },
        },
        Group::Horse(a, v) => SpawnGroup::Horse {
            ageable: native_ageable(a),
            variant: named(&equine::VARIANT_NAMES, *v)?,
        },
        Group::Llama(a, v) => SpawnGroup::Llama {
            ageable: native_ageable(a),
            variant: named(&llama::VARIANTS, *v)?,
        },
        Group::Fox(a, v) => SpawnGroup::Fox {
            ageable: native_ageable(a),
            variant: named(&FOX_VARIANTS, *v)?,
        },
        Group::Rabbit(a, v) => SpawnGroup::Rabbit {
            ageable: native_ageable(a),
            variant: RABBIT_VARIANTS
                .iter()
                .find(|(id, _)| id == v)
                .map(|(_, n)| (*n).to_owned())?,
        },
        Group::Axolotl(a, [first, second]) => SpawnGroup::Axolotl {
            ageable: native_ageable(a),
            variants: [
                named(&horizon_client::entity_nbt::AXOLOTL_VARIANTS, *first)?,
                named(&horizon_client::entity_nbt::AXOLOTL_VARIANTS, *second)?,
            ],
        },
        Group::Wolf(a, v) => SpawnGroup::Wolf {
            ageable: native_ageable(a),
            variant: Identifier::parse(tame::WOLF_VARIANTS.get(usize::from(*v))?.0).ok()?,
        },
        Group::School(leader, look) => SpawnGroup::School {
            leader: EntityRef::new(epoch, *leader),
            tropical: match look {
                Some((pattern, base, color)) => Some(TropicalVariant::new(
                    named(&horizon_client::entity_nbt::TROPICAL_PATTERNS, *pattern)?,
                    named(&render::DYE_NAMES, *base)?,
                    named(&render::DYE_NAMES, *color)?,
                )),
                None => None,
            },
        },
    })
}

/// The API's group data as the game's; None for a name the game does not know,
/// which drops the group.
pub(in crate::game) fn spawn_group(group: &SpawnGroup) -> Option<Group> {
    let index = |names: &[&str], name: &str| names.iter().position(|n| *n == name).map(|i| i as u8);
    Some(match group {
        SpawnGroup::Ageable(a) => Group::Ageable(ageable(a)),
        SpawnGroup::Zombie {
            baby,
            jockey,
            tried_camel,
        } => Group::Zombie {
            baby: *baby,
            jockey: *jockey,
            husk: *tried_camel,
        },
        SpawnGroup::Spider { effect } => Group::Spider(match effect {
            Some(id) => {
                let def = horizon_client_core::potions::effect(&id.to_string())?;
                Some(horizon_client_core::potions::namespaced(def.id))
            }
            None => None,
        }),
        SpawnGroup::Horse {
            ageable: a,
            variant,
        } => Group::Horse(ageable(a), index(&equine::VARIANT_NAMES, variant)?),
        SpawnGroup::Llama {
            ageable: a,
            variant,
        } => Group::Llama(ageable(a), index(&llama::VARIANTS, variant)?),
        SpawnGroup::Fox {
            ageable: a,
            variant,
        } => Group::Fox(ageable(a), index(&FOX_VARIANTS, variant)?),
        SpawnGroup::Rabbit {
            ageable: a,
            variant,
        } => Group::Rabbit(
            ageable(a),
            RABBIT_VARIANTS.iter().find(|(_, n)| n == variant)?.0,
        ),
        SpawnGroup::Axolotl {
            ageable: a,
            variants: [first, second],
        } => Group::Axolotl(
            ageable(a),
            [
                index(&horizon_client::entity_nbt::AXOLOTL_VARIANTS, first)?,
                index(&horizon_client::entity_nbt::AXOLOTL_VARIANTS, second)?,
            ],
        ),
        SpawnGroup::Wolf {
            ageable: a,
            variant,
        } => {
            if variant.namespace() != "minecraft" {
                return None;
            }
            let name = variant.path();
            let i = tame::WOLF_VARIANTS.iter().position(|(n, _)| *n == name)?;
            Group::Wolf(ageable(a), i as u8)
        }
        SpawnGroup::School { leader, tropical } => Group::School(
            leader.id(),
            match tropical {
                Some(look) => Some((
                    index(
                        &horizon_client::entity_nbt::TROPICAL_PATTERNS,
                        &look.pattern,
                    )?,
                    index(&render::DYE_NAMES, &look.base_color)?,
                    index(&render::DYE_NAMES, &look.pattern_color)?,
                )),
                None => None,
            },
        ),
        _ => return None,
    })
}

/// `Fox.Variant` names, by the game's index.
const FOX_VARIANTS: [&str; 2] = ["red", "snow"];
/// `Rabbit.Variant`: the game's id and its saved name.
const RABBIT_VARIANTS: [(u8, &str); 7] = [
    (0, "brown"),
    (1, "white"),
    (2, "black"),
    (3, "white_splotched"),
    (4, "gold"),
    (5, "salt"),
    (99, "evil"),
];

fn native_ageable(a: &Ageable) -> AgeableGroup {
    let mut out = AgeableGroup::new(a.should_spawn_baby, a.baby_chance);
    out.size = a.size;
    out
}

fn ageable(a: &AgeableGroup) -> Ageable {
    Ageable {
        size: a.size,
        should_spawn_baby: a.should_spawn_baby,
        baby_chance: a.baby_chance,
    }
}

impl UnspawnedMobEdits for Edits<'_, '_> {
    fn set_equipment(
        &self,
        mob: UnspawnedMobRef,
        slot: EquipmentSlot,
        item: Option<ItemStack>,
    ) -> Result<(), UnspawnedMobError> {
        self.game.edit_lent_mob(mob, |game, m| {
            let stack = item
                .try_into_engine(world_items!(game))
                .map_err(|_| UnspawnedMobError::Invalid)?;
            if !m.set_equipment_stack(slot as usize, stack) {
                return Err(UnspawnedMobError::Unsupported);
            }
            collect_equipment(&game.motion.attributes, world_items!(game), m);
            Ok(())
        })
    }

    fn set_attribute_base(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
        base: f64,
    ) -> Result<(), UnspawnedMobError> {
        if !base.is_finite() {
            return Err(UnspawnedMobError::Invalid);
        }
        self.write_attribute(mob, attribute, AttributeWrite::Base(base))
            .map(|_| ())
    }

    fn add_attribute_modifier(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
        modifier: AttributeModifier,
    ) -> Result<(), UnspawnedMobError> {
        let operation = match modifier.operation {
            AttributeOperation::AddValue => Operation::AddValue,
            AttributeOperation::AddMultipliedBase => Operation::AddMultipliedBase,
            AttributeOperation::AddMultipliedTotal => Operation::AddMultipliedTotal,
            _ => return Err(UnspawnedMobError::Invalid),
        };
        if !modifier.amount.is_finite() {
            return Err(UnspawnedMobError::Invalid);
        }
        let write = AttributeWrite::add(
            &modifier.id.to_string(),
            modifier.amount,
            operation,
            true,
            false,
        )
        .ok_or(UnspawnedMobError::Invalid)?;
        match self.write_attribute(mob, attribute, write)? {
            true => Ok(()),
            false => Err(UnspawnedMobError::Invalid),
        }
    }

    fn remove_attribute_modifier(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
        id: &Identifier,
    ) -> Result<bool, UnspawnedMobError> {
        self.write_attribute(mob, attribute, AttributeWrite::Remove(id.to_string()))
    }

    fn add_tag(&self, mob: UnspawnedMobRef, tag: &str) -> Result<bool, UnspawnedMobError> {
        self.game.edit_lent_mob(mob, |_, m| {
            let tags = &mut m.extra.spawn_tags;
            if tags.iter().any(|t| t == tag) {
                return Ok(false);
            }
            if tag.is_empty() || tags.len() >= MAX_TAGS {
                return Err(UnspawnedMobError::Invalid);
            }
            tags.push(tag.to_owned());
            Ok(true)
        })
    }

    fn remove_tag(&self, mob: UnspawnedMobRef, tag: &str) -> Result<bool, UnspawnedMobError> {
        self.game.edit_lent_mob(mob, |_, m| {
            let tags = &mut m.extra.spawn_tags;
            let before = tags.len();
            tags.retain(|t| t != tag);
            Ok(tags.len() != before)
        })
    }

    fn set_custom_name(
        &self,
        mob: UnspawnedMobRef,
        name: Option<mod_api::Text>,
    ) -> Result<(), UnspawnedMobError> {
        let name = name.try_into_engine(&())?;
        self.game.edit_lent_mob(mob, |_, m| {
            m.custom_name = name.as_ref().map(Text::plain);
            m.custom_name_text = name;
            Ok(())
        })
    }
}

impl Edits<'_, '_> {
    /// An attribute write on the lent mob; `Unsupported` for an attribute it
    /// does not keep itself.
    fn write_attribute(
        &self,
        mob: UnspawnedMobRef,
        attribute: &Identifier,
        write: AttributeWrite,
    ) -> Result<bool, UnspawnedMobError> {
        self.game.edit_lent_mob(mob, |game, m| {
            write_unspawned_attribute(
                &game.motion.attributes,
                world_items!(game),
                m,
                &attribute.to_string(),
                write,
            )
            .ok_or(UnspawnedMobError::Unsupported)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::commands::Target;
    use super::super::creatures::SpawnSite;
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::mobs::finalize::{self, Attr};
    use horizon_client::mobs::spawn_rules::Reason;
    use mod_api::{FinalizeSpawnEvent, Flow, Host};
    use std::sync::{Arc, Mutex};

    type Listen = dyn Fn(&dyn Host, &mut FinalizeSpawnEvent<'_>) -> Flow + Send + Sync;
    /// Answers `finalize_spawn` with the test's closure.
    struct Listener(Box<Listen>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn finalize_spawn(&self, host: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            (self.0)(host, event)
        }
    }
    fn listening<R>(
        listen: impl Fn(&dyn Host, &mut FinalizeSpawnEvent<'_>) -> Flow + Send + Sync + 'static,
        run: impl FnOnce() -> R,
    ) -> R {
        let listener = Listener(Box::new(listen));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("unspawned", Box::new(listener))].into(),
            run,
        )
    }

    const AT: [f64; 3] = [8.5, 64.0, 8.5];
    fn id(s: &str) -> Identifier {
        Identifier::parse(s).unwrap()
    }
    fn pumpkin() -> ItemStack {
        super::super::api_values::test_snapshot(&horizon_client::items::ItemStack::new(
            "minecraft:carved_pumpkin",
            1,
        ))
    }
    fn spawn_pig(game: &mut Game<'_>) -> Option<u32> {
        game.spawn_mob(
            "minecraft:pig",
            AT,
            false,
            SpawnSite::Finalizing(Reason::Natural),
        )
    }

    /// What a listener edits through the editor is on the mob that joins the
    /// level, and the handle reads the edits back inside the call.
    #[test]
    #[ignore = "daily"]
    fn edits_through_the_editor_are_on_the_mob_that_joins() {
        type Seen = (
            Result<Identifier, UnspawnedMobError>,
            Result<f64, UnspawnedMobError>,
            Result<Option<ItemStack>, UnspawnedMobError>,
        );
        let seen: Arc<Mutex<Option<Seen>>> = Arc::new(Mutex::new(None));
        let saw = seen.clone();
        let max_health = id("minecraft:max_health");
        listening(
            move |host, event| {
                let (mob, edits) = (event.mob, event.edits);
                edits
                    .set_equipment(mob, EquipmentSlot::Head, Some(pumpkin()))
                    .unwrap();
                let mut oversized = pumpkin();
                oversized.set_count(100).unwrap();
                assert_eq!(
                    edits.set_equipment(mob, EquipmentSlot::Head, Some(oversized)),
                    Err(UnspawnedMobError::Invalid)
                );
                assert_eq!(
                    mob.equipment(host, EquipmentSlot::Head),
                    Ok(Some(pumpkin()))
                );
                edits.set_equipment(mob, EquipmentSlot::Head, None).unwrap();
                assert_eq!(mob.equipment(host, EquipmentSlot::Head), Ok(None));
                edits
                    .set_equipment(mob, EquipmentSlot::Head, Some(pumpkin()))
                    .unwrap();
                edits.set_attribute_base(mob, &max_health, 20.0).unwrap();
                assert_eq!(edits.add_tag(mob, "test:tagged"), Ok(true));
                edits
                    .set_custom_name(mob, Some(mod_api::Text::literal("Bacon")))
                    .unwrap();
                *saw.lock().unwrap() = Some((
                    mob.kind(host),
                    mob.attribute_base(host, &max_health),
                    mob.equipment(host, EquipmentSlot::Head),
                ));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    let head = &m.equipment_stacks()[EquipmentSlot::Head as usize];
                    assert_eq!(
                        head.as_ref().map(|s| &*s.item),
                        Some("minecraft:carved_pumpkin")
                    );
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 20.0);
                    assert_eq!(m.custom_name.as_deref(), Some("Bacon"));
                    assert_eq!(game.tags_of(Target::Mob(spawned)), ["test:tagged"]);
                });
            },
        );
        let (kind, base, head) = seen.lock().unwrap().take().expect("the hook ran");
        assert_eq!(kind, Ok(id("minecraft:pig")));
        assert_eq!(base, Ok(20.0));
        assert_eq!(head, Ok(Some(pumpkin())));
    }

    /// A handle kept past its hook reads `Expired`, and the next call's
    /// editor refuses it the same way: it reaches only its own lend.
    #[test]
    #[ignore = "daily"]
    fn a_handle_kept_past_its_hook_is_expired() {
        let kept: Arc<Mutex<Vec<UnspawnedMobRef>>> = Arc::default();
        let refused = Arc::new(Mutex::new(Vec::new()));
        let (keep, refuse) = (kept.clone(), refused.clone());
        listening(
            move |_, event| {
                let earlier = keep.lock().unwrap().last().copied();
                if let Some(earlier) = earlier {
                    refuse
                        .lock()
                        .unwrap()
                        .push(event.edits.add_tag(earlier, "test:late"));
                }
                keep.lock().unwrap().push(event.mob);
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let first = spawn_pig(game).unwrap();
                    spawn_pig(game).unwrap();
                    let old = kept.lock().unwrap()[0];
                    assert_eq!(old.kind(game), Err(UnspawnedMobError::Expired));
                    assert_eq!(old.position(game), Err(UnspawnedMobError::Expired));
                    assert!(game.tags_of(Target::Mob(first)).is_empty());
                });
            },
        );
        assert_eq!(*refused.lock().unwrap(), [Err(UnspawnedMobError::Expired)]);
        let kept = kept.lock().unwrap();
        assert_ne!(kept[0], kept[1], "each call lends anew");
    }

    /// `Flow::Cancel` skips finalization and keeps the listener's edits on
    /// the mob that still joins; `spawn_cancelled` adds no mob at all.
    #[test]
    #[ignore = "daily"]
    fn both_cancels_keep_their_contract_with_edits() {
        let max_health = id("minecraft:max_health");
        listening(
            move |_, event| {
                event
                    .edits
                    .set_attribute_base(event.mob, &max_health, 20.0)
                    .unwrap();
                Flow::Cancel
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).expect("a skipped finalization still spawns");
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert_eq!(finalize::base_value(m, Attr::MaxHealth), 20.0);
                    assert!(
                        !finalize::has_modifier(
                            m,
                            Attr::FollowRange,
                            "minecraft:random_spawn_bonus"
                        ),
                        "no finalizeSpawn ran"
                    );
                });
            },
        );
        listening(
            |_, event| {
                assert_eq!(event.edits.add_tag(event.mob, "test:gone"), Ok(true));
                event.spawn_cancelled = true;
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let before = game.mobs.len();
                    assert_eq!(spawn_pig(game), None);
                    assert_eq!(game.mobs.len(), before, "a stopped spawn adds no mob");
                });
            },
        );
    }

    /// Every live site hands `group` over empty today; group data a listener
    /// leaves there is what finalization reads: an ageable group whose later
    /// members are all babies makes this pig one.
    #[test]
    #[ignore = "daily"]
    fn the_group_a_listener_leaves_is_what_finalization_reads() {
        with_game(|game| {
            let alone = spawn_pig(game).unwrap();
            let m = game.mobs.iter().find(|m| m.id == alone).unwrap();
            assert!(!m.is_baby(), "a pack's first member is never a baby");
        });
        let given = Arc::new(Mutex::new(Vec::new()));
        let heard = given.clone();
        listening(
            move |_, event| {
                heard.lock().unwrap().push(event.group.clone());
                let mut ageable = AgeableGroup::new(true, 1.0);
                ageable.size = 1;
                *event.group = Some(SpawnGroup::Ageable(ageable));
                Flow::Continue
            },
            || {
                with_game(|game| {
                    let spawned = spawn_pig(game).unwrap();
                    let m = game.mobs.iter().find(|m| m.id == spawned).unwrap();
                    assert!(m.is_baby());
                });
            },
        );
        assert_eq!(*given.lock().unwrap(), [None]);
    }

    /// A mod goal that only names itself: a mob carries it under this name
    /// while it has its goal, and as `ModGoal` once a copy left it out.
    struct Kept;
    impl mod_api::Goal for Kept {
        fn flags(&self) -> mod_api::GoalFlags {
            mod_api::GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "Kept"
        }
        fn can_use(&mut self, _: &mut mod_api::GoalContext<'_>) -> bool {
            false
        }
    }
    struct KeptGoal;
    #[mod_api::horizon_part]
    impl mod_api::EntityPart for KeptGoal {
        fn register_goals(&self, goals: &mut mod_api::GoalRegistrar<'_>) {
            goals.add_goal(7, Kept);
        }
    }
    struct KeptGoalMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for KeptGoalMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.entity_part("minecraft:zombie", KeptGoal);
        }
    }

    /// A listener that panics still ends its lend: the handle is withdrawn
    /// and the mob in the caller's slot is the one that was lent, mod goal
    /// and all, not a copy (whose mod goal would have no goal) or a default.
    #[test]
    #[ignore = "daily"]
    fn a_panicking_listener_still_ends_the_lend() {
        let installed = || vec![mod_api::ModEntry::new("kept-goal", Box::new(KeptGoalMod))];
        mod_platform::testing::with_mods(installed().into(), || {
            super::super::api_values::install_parts(&installed()).unwrap();
            with_game(|game| {
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut zombie = Mob::new(70, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
                zombie.custom_name = Some("Lent".to_owned());
                let goals = |m: &Mob| -> Vec<&'static str> {
                    m.ai.goals.entries().map(|(_, name, _)| name).collect()
                };
                assert!(
                    goals(&zombie).contains(&"Kept"),
                    "the part attached its goal"
                );
                let mut kept = None;
                let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.lend_unspawned("minecraft:zombie", AT, Some(&mut zombie), |mob, _| {
                        kept = Some(mob);
                        panic!("a listener's bug");
                    })
                }));
                assert!(panicked.is_err());
                assert!(
                    game.lent_mobs.borrow().0.is_empty(),
                    "the lend is withdrawn"
                );
                let kept = kept.expect("the call ran");
                assert_eq!(kept.kind(game), Err(UnspawnedMobError::Expired));
                assert_eq!(zombie.id, 70);
                assert_eq!(zombie.custom_name.as_deref(), Some("Lent"));
                let back = goals(&zombie);
                assert!(
                    back.contains(&"Kept"),
                    "the mod goal keeps its goal: {back:?}"
                );
                assert!(!back.contains(&"ModGoal"), "no copied mod goal: {back:?}");
            });
            super::super::api_values::install_parts(&[]).unwrap();
        });
    }

    /// Group data crosses the boundary by the names vanilla saves its
    /// variants under, both ways, and a name the game does not know drops
    /// the group.
    #[test]
    fn group_data_crosses_by_vanilla_names() {
        let epoch = WorldEpoch::new(3);
        let ageable = Ageable::babies(true);
        let api = AgeableGroup::new(true, 0.05);
        let cases = [
            (
                Group::Horse(ageable, 1),
                SpawnGroup::Horse {
                    ageable: api,
                    variant: "creamy".into(),
                },
            ),
            (
                Group::Rabbit(ageable, 99),
                SpawnGroup::Rabbit {
                    ageable: api,
                    variant: "evil".into(),
                },
            ),
            (
                Group::Fox(ageable, 1),
                SpawnGroup::Fox {
                    ageable: api,
                    variant: "snow".into(),
                },
            ),
            (
                Group::Axolotl(ageable, [0, 4]),
                SpawnGroup::Axolotl {
                    ageable: api,
                    variants: ["lucy".into(), "blue".into()],
                },
            ),
            (
                Group::Wolf(ageable, 1),
                SpawnGroup::Wolf {
                    ageable: api,
                    variant: id("minecraft:spotted"),
                },
            ),
            (
                Group::Spider(Some("minecraft:speed")),
                SpawnGroup::Spider {
                    effect: Some(id("minecraft:speed")),
                },
            ),
            (
                Group::School(7, Some((0, 1, 0))),
                SpawnGroup::School {
                    leader: EntityRef::new(epoch, 7),
                    tropical: Some(TropicalVariant::new(
                        "kob".into(),
                        "orange".into(),
                        "white".into(),
                    )),
                },
            ),
            (
                Group::Zombie {
                    baby: true,
                    jockey: false,
                    husk: Some(true),
                },
                SpawnGroup::Zombie {
                    baby: true,
                    jockey: false,
                    tried_camel: Some(true),
                },
            ),
        ];
        for (game, api) in cases {
            assert_eq!(native_spawn_group(&game, epoch).as_ref(), Some(&api));
            assert_eq!(spawn_group(&api), Some(game));
        }
        let unknown = SpawnGroup::Horse {
            ageable: api,
            variant: "plaid".into(),
        };
        assert_eq!(spawn_group(&unknown), None);
    }
}
