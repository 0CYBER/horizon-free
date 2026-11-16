//! Prepared cauldron callbacks and their authoritative action savepoint.
use super::*;
use horizon_client::cauldron::interactions::{Dispatcher, Entry};
use mod_api::__private::game::RegistrationContextNew;
use std::{cell::RefCell, sync::Arc};

#[derive(Clone)]
pub(super) struct Callback {
    pub(super) handler: Arc<dyn mod_api::Interaction>,
    pub(super) owner: &'static str,
}
pub(super) type Table = Dispatcher<Callback>;
thread_local! {
    static TABLE: RefCell<Arc<Table>> = RefCell::new(Arc::new(Table::default()));
}
/// Share the reentry boundary between authoritative and predicted decisions.
pub(super) fn enter_decision() -> Option<Active> {
    if ACTIVE.with(|active| active.replace(true)) {
        None
    } else {
        Some(Active)
    }
}
pub(super) fn snapshot() -> Arc<Table> {
    TABLE.with_borrow(Arc::clone)
}
pub(super) fn clear() {
    install(Arc::new(Table::default()));
}
pub(super) fn install(table: Arc<Table>) {
    TABLE.with_borrow_mut(|old| *old = table);
}

/// What a `register_to_all` or `register_tag_to_all` entry answers for.
enum Target {
    Item(String),
    Tag(String),
}
struct Registrar<'a> {
    table: Table,
    items: &'a Items,
    registry: &'a horizon_world::registry::BlockRegistry,
    all: Vec<(Target, Callback)>,
    error: Option<mod_api::CauldronRegistrationError>,
}
/// A tag as entries keep it: its resource location, without `#`.
fn tag_id(tag: &str) -> Option<String> {
    let tag = tag.strip_prefix('#').unwrap_or(tag);
    mod_api::Identifier::parse(tag)
        .ok()
        .map(|id| id.to_string())
}
impl Registrar<'_> {
    fn insert_tag(
        &mut self,
        block: String,
        tag: String,
        callback: Callback,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let new_owner = callback.owner.to_owned();
        self.table
            .register_tag(block, tag, callback)
            .map_err(|old| mod_api::CauldronRegistrationError::Duplicate {
                owners: [old.owner.to_owned(), new_owner],
            })
    }
    /// Keep a refusal, which fails the whole preparation when it finishes.
    fn admit(
        &mut self,
        result: std::result::Result<(), mod_api::CauldronRegistrationError>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }
    fn insert(
        &mut self,
        block: String,
        item: String,
        callback: Callback,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let new_owner = callback.owner.to_owned();
        self.table.register(block, item, callback).map_err(|old| {
            let owner = match old {
                Entry::Mod(callback) => callback.owner.to_owned(),
                _ => "minecraft".to_owned(),
            };
            mod_api::CauldronRegistrationError::Duplicate {
                owners: [owner, new_owner],
            }
        })
    }
    fn finish(mut self) -> Result<Arc<Table>> {
        if let Some(error) = self.error {
            return Err(format!("cauldron registration: {error:?}").into());
        }
        for (target, callback) in std::mem::take(&mut self.all) {
            for block in self.table.blocks() {
                match &target {
                    Target::Item(item) => self.insert(block, item.clone(), callback.clone()),
                    Target::Tag(tag) => self.insert_tag(block, tag.clone(), callback.clone()),
                }
                .map_err(|e| format!("cauldron registration: {e:?}"))?;
            }
        }
        Ok(Arc::new(self.table))
    }
}
impl mod_api::CauldronInteractions for Registrar<'_> {
    fn register(
        &mut self,
        block: mod_api::BlockKey,
        item: mod_api::ItemKey,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        use mod_api::CauldronRegistrationError as Error;
        let result = (|| {
            let block = mod_api::__private::block_key_id(block)
                .filter(|id| self.registry.default_state(id).is_some())
                .ok_or(Error::UnknownBlock)?
                .to_string();
            let item = mod_api::__private::item_key_id(item)
                .filter(|id| self.items.items.contains_key(*id))
                .ok_or(Error::UnknownItem)?
                .to_string();
            self.insert(
                block,
                item,
                Callback {
                    handler,
                    owner: mod_api::__private::cauldron_owner(),
                },
            )
        })();
        self.admit(result)
    }
    fn register_to_all(
        &mut self,
        item: mod_api::ItemKey,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let Some(item) =
            mod_api::__private::item_key_id(item).filter(|id| self.items.items.contains_key(*id))
        else {
            self.error = Some(mod_api::CauldronRegistrationError::UnknownItem);
            return Err(mod_api::CauldronRegistrationError::UnknownItem);
        };
        self.all.push((
            Target::Item(item.to_string()),
            Callback {
                handler,
                owner: mod_api::__private::cauldron_owner(),
            },
        ));
        Ok(())
    }
    fn register_tag(
        &mut self,
        block: mod_api::BlockKey,
        tag: &str,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        use mod_api::CauldronRegistrationError as Error;
        let result = (|| {
            let block = mod_api::__private::block_key_id(block)
                .filter(|id| self.registry.default_state(id).is_some())
                .ok_or(Error::UnknownBlock)?
                .to_string();
            let tag = tag_id(tag).ok_or(Error::InvalidTag)?;
            let owner = mod_api::__private::cauldron_owner();
            self.insert_tag(block, tag, Callback { handler, owner })
        })();
        self.admit(result)
    }
    fn register_tag_to_all(
        &mut self,
        tag: &str,
        handler: Arc<dyn mod_api::Interaction>,
    ) -> std::result::Result<(), mod_api::CauldronRegistrationError> {
        let Some(tag) = tag_id(tag) else {
            return self.admit(Err(mod_api::CauldronRegistrationError::InvalidTag));
        };
        let owner = mod_api::__private::cauldron_owner();
        self.all
            .push((Target::Tag(tag), Callback { handler, owner }));
        Ok(())
    }
}
impl Game<'_> {
    pub(super) fn prepare_cauldron_interactions(&self, items: &Items) -> Result<Arc<Table>> {
        let mut registrar = Registrar {
            table: Table::default(),
            items,
            registry: self.pack.assets.registry,
            all: Vec::new(),
            error: None,
        };
        Self::native_register_cauldron_interaction(
            &mod_api::RegistrationContext::new(),
            &mut registrar,
        );
        registrar.finish()
    }
}

thread_local! { static ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(super) struct Active;
impl Drop for Active {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(false));
    }
}

/// Effects owned by the existing transaction journal, published at root commit only.
pub(super) struct Effects {
    player: u128,
    pos: [i32; 3],
    sound: Option<String>,
    event: Option<&'static str>,
    statistics: Vec<String>,
    item: String,
}
impl Game<'_> {
    pub(super) fn commit_cauldron_effects(&mut self, effects: Effects) {
        self.native_as_player(effects.player, |game| {
            for stat in &effects.statistics {
                if stat == "minecraft:used" {
                    game.award_stat("used", &effects.item, 1);
                } else {
                    game.award_stat("custom", stat, 1);
                }
            }
        });
        if let Some(sound) = effects.sound {
            self.audio.play(
                &sound,
                Some(effects.pos.map(|v| f64::from(v) + 0.5)),
                1.0,
                1.0,
            );
        }
        if let Some(event) = effects.event {
            self.post_block_game_event(event, effects.pos, None, None);
        }
    }
    /// Read-only selection followed by one authoritative action savepoint.
    pub(super) fn use_mod_cauldron(
        &mut self,
        pos: [i32; 3],
        under_water: bool,
        callback: &Callback,
        table: &Arc<Table>,
    ) -> bool {
        use crate::game::api_values::{TryIntoApi, TryIntoEngine};
        use mod_api::{CauldronDecision, CauldronFire, CauldronPlan, CauldronUse};
        let Some(_active) = enter_decision() else {
            return true;
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> std::result::Result<bool, ()> {
                let held = self.held().cloned().ok_or(())?;
                let before = self.world.block(pos[0], pos[1], pos[2]);
                let index = api_values::block_state::current().map_err(|_| ())?;
                let epoch = self.native_lifecycle.epoch().ok_or(())?;
                let player = self.native_acting_id();
                let dimension = self.dim;
                let actor = mod_api::EntityRef::new(epoch, self.native_acting_entity());
                let input = CauldronUse {
                    dimension: mod_api::Identifier::parse(self.dim.id()).map_err(|_| ())?,
                    pos,
                    state: index.to_api(before).map_err(|_| ())?,
                    actor,
                    hand: match self.hand {
                        Hand::Main => mod_api::Hand::Main,
                        Hand::Off => mod_api::Hand::Off,
                    },
                    held: held.try_into_api().map_err(|_| ())?,
                    underwater: under_water,
                    creative: self.creative,
                };
                let _owner = mod_api::__private::enter(callback.owner, "cauldron_interaction");
                let plan = match callback.handler.decide(self, &input) {
                    CauldronDecision::Consume => CauldronPlan::default(),
                    CauldronDecision::Success(plan) => plan,
                    // `Pass`, and any decision this build doesn't know: no action.
                    _ => return Ok(false),
                };
                let unchanged = |game: &mut Game<'_>| {
                    Arc::ptr_eq(table, &snapshot())
                        && game.dim == dimension
                        && game.native_lifecycle.epoch() == Some(epoch)
                        && game.native_acting_id() == player
                        && game.hand
                            == match input.hand {
                                mod_api::Hand::Main => Hand::Main,
                                mod_api::Hand::Off => Hand::Off,
                            }
                        && game.world.block(pos[0], pos[1], pos[2]) == before
                        && game.held() == Some(&held)
                        && api_values::block_state::current()
                            .is_ok_and(|now| Arc::ptr_eq(&index, &now))
                };
                if !unchanged(self) {
                    return Err(());
                }
                // Validate the complete untrusted proposal before entering the write phase.
                if plan.statistics.len() > 2
                    || (plan.statistics.len() == 2 && plan.statistics[0] == plan.statistics[1])
                {
                    return Err(());
                }
                if plan.sound.as_ref().is_some_and(|sound| {
                    !chat::command_export(&self.options.resources, true).is_some_and(|registry| {
                        registry
                            .ids("minecraft:sound_event")
                            .iter()
                            .any(|known| known == &sound.to_string())
                    })
                }) || plan.statistics.iter().any(|stat| {
                    let name = stat.to_string();
                    name != "minecraft:used"
                        && !name.strip_prefix("minecraft:").is_some_and(|key| {
                            horizon_client::scoreboard::CUSTOM_STATS.contains(&key)
                        })
                }) {
                    return Err(());
                }
                let block = plan
                    .block
                    .map(|state| index.to_engine(state))
                    .transpose()
                    .map_err(|_| ())?;
                if let Some(block) = block {
                    let state = self.pack.assets.registry.state(block).ok_or(())?;
                    if !table.contains_block(&state.name) {
                        return Err(());
                    }
                }
                let exchange = plan
                    .exchange
                    .as_ref()
                    .map(|stack| stack.try_into_engine(self.items()))
                    .transpose()
                    .map_err(|_| ())?;
                let event = match plan
                    .fluid_event
                    .as_ref()
                    .map(ToString::to_string)
                    .as_deref()
                {
                    None => None,
                    Some("minecraft:fluid_place") => Some("minecraft:fluid_place"),
                    Some("minecraft:fluid_pickup") => Some("minecraft:fluid_pickup"),
                    _ => return Err(()),
                };
                let effects = Effects {
                    player,
                    pos,
                    event,
                    sound: plan.sound.as_ref().map(ToString::to_string),
                    statistics: plan.statistics.iter().map(ToString::to_string).collect(),
                    item: held.item.to_string(),
                };
                mod_api::run_transaction(self, |game, _| {
                    game.journal_player(player);
                    if callback.handler.fire(game, &input, &plan) != CauldronFire::Applied
                        || !unchanged(game)
                    {
                        return Err(());
                    }
                    if let Some(block) = block
                        && !game.journal_set_block(pos, block)
                    {
                        return Err(());
                    }
                    if let Some(exchange) = exchange {
                        let slot = game.held_slot();
                        let mut hand = powder_snow::HeldSlot {
                            game,
                            slot,
                            dropper: Some(player),
                        };
                        horizon_client::cauldron::create_filled_result(&mut hand, exchange, true);
                    }
                    game.after_commit(transactions::After::Cauldron(effects));
                    Ok(())
                })?;
                Ok(true)
            },
        ));
        match result {
            Ok(Ok(consumed)) => consumed,
            _ => {
                log::warn!("cauldron action refused for {}", callback.owner);
                true
            }
        }
    }
}

#[cfg(test)]
#[path = "cauldron_interactions/tests.rs"]
mod tests;
