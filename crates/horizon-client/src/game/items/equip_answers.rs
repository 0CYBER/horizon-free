//! `IItemExtension.canEquip` on the game's side (`native_can_equip`). The dispenser, a mob's
//! pickup and the two inventory menus ask it, each with the game as the read-only `Host` and the
//! target as a handle. A stack is converted for the parts once and lent to every slot and entity
//! asked about it.
use super::*;
use crate::game::api_values::TryIntoApi;
use horizon_client::mobs::equip::CanEquip;

/// A stack converted for the parts of its item, ready for any number of `can_equip` questions.
/// Empty when no part of the item listens (the component's answer stands and nothing is
/// converted). A listening stack that cannot be prepared refuses the decision.
pub(in crate::game) struct EquipStack {
    _leases: Vec<Box<dyn std::any::Any>>,
    reads: Vec<mod_api::EquipReadRow>,
    read_indices: std::collections::HashMap<u32, usize>,
    stack: Option<Box<mod_api::ItemStack>>,
    refused: bool,
    epoch: Option<mod_api::WorldEpoch>,
    admissions: std::collections::HashMap<u32, horizon_client::join_seq::JoinSeq>,
    source: Option<(usize, horizon_client::mobs::equip::EquipStamp)>,
}

impl Game<'_> {
    pub(in crate::game) fn equip_stack(
        &self,
        stack: &ItemStack,
        kinds: &[&str],
        targets: &[u32],
    ) -> EquipStack {
        let empty = |refused| EquipStack {
            _leases: Vec::new(),
            reads: Vec::new(),
            read_indices: Default::default(),
            stack: None,
            refused,
            source: None,
            epoch: None,
            admissions: Default::default(),
        };
        if !mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item) {
            return empty(false);
        }
        let address = stack as *const _ as usize;
        let source = match horizon_client::mobs::equip::source_stamp_at(address) {
            Some(Some(stamp)) => Some((address, stamp)),
            // A retired tracked scope is different from an ordinary immutable
            // input without a barrier. It must never prepare or ask a guest.
            Some(None) => return empty(true),
            None => None,
        };
        #[cfg(test)]
        EQUIP_CONVERSIONS.with(|count| count.set(count.get() + 1));
        let stack = stack.try_into_api().ok().map(Box::new);
        let admissions = {
            let mut table = self.resident_entities.table.borrow_mut();
            let order = table.admission_order();
            targets
                .iter()
                .filter_map(|id| order(*id).map(|generation| (*id, generation)))
                .collect::<std::collections::HashMap<_, _>>()
        };
        let rows = admissions
            .keys()
            .filter_map(|id| self.capture_equip_read(*id))
            .collect::<Vec<_>>();
        let prepared = stack
            .as_ref()
            .map(|stack| mod_api::__private::prepare_equip_parts(stack, kinds, &rows))
            .transpose();
        let (leases, refused) = match prepared {
            Ok(Some(leases)) => (leases, false),
            Ok(None) | Err(_) => (Vec::new(), true),
        };
        let reads = rows;
        let read_indices = reads
            .iter()
            .enumerate()
            .map(|(i, row)| (row.entity.id(), i))
            .collect();
        EquipStack {
            read_indices,
            epoch: self.native_lifecycle.epoch(),
            admissions,
            reads,
            _leases: leases,
            stack,
            refused,
            source,
        }
    }

    fn capture_equip_read(&self, id: u32) -> Option<mod_api::EquipReadRow> {
        let entity = self.entity(id)?;
        let living = mod_api::LivingRef::new(entity);
        let slots = [
            mod_api::EquipmentSlot::MainHand,
            mod_api::EquipmentSlot::OffHand,
            mod_api::EquipmentSlot::Feet,
            mod_api::EquipmentSlot::Legs,
            mod_api::EquipmentSlot::Chest,
            mod_api::EquipmentSlot::Head,
            mod_api::EquipmentSlot::Body,
            mod_api::EquipmentSlot::Saddle,
        ];
        Some(mod_api::EquipReadRow {
            kind: self.entity_kind(entity),
            position: self.entity_position(entity),
            health: self.living_health(living),
            max_health: self.living_max_health(living),
            alive: self.living_alive(living),
            living: self.living(entity).is_some(),
            equipment: std::array::from_fn(|i| {
                self.living_equipment(living, slots[i])
                    .and_then(|stack| stack.try_into_api().ok())
            }),
            ..mod_api::EquipReadRow::new(entity)
        })
    }
    pub(in crate::game) fn equip_can_apply(&self, prepared: &EquipStack, id: u32) -> bool {
        if prepared.stack.is_none() {
            return !prepared.refused;
        }
        if prepared.refused
            || prepared.epoch != self.native_lifecycle.epoch()
            || prepared.source.is_some_and(|(address, stamp)| {
                horizon_client::mobs::equip::source_stamp_at(address) != Some(Some(stamp))
            })
        {
            return false;
        }
        let current = {
            let mut table = self.resident_entities.table.borrow_mut();
            let order = table.admission_order();
            order(id)
        };
        if current.is_none() || current != prepared.admissions.get(&id).copied() {
            return false;
        }
        let Some(current) = self.capture_equip_read(id) else {
            return false;
        };
        prepared
            .reads
            .iter()
            .find(|row| row.entity == current.entity)
            .is_some_and(|expected| *expected == current)
    }

    /// `LivingEntity.isEquippableInSlot` with `IItemExtension.canEquip`: `data` is the stack's
    /// `equippable` answer for the entity `entity` of type `kind` in `slot`, and the parts of the
    /// prepared `stack`'s item may change it. When a part would be asked and `entity` is no live
    /// entity of this side (a missing or stale handle), the slot is refused.
    pub(in crate::game) fn can_equip(
        &self,
        stack: &EquipStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        if stack.refused
            || stack.source.is_some_and(|(address, stamp)| {
                horizon_client::mobs::equip::source_stamp_at(address) != Some(Some(stamp))
            })
        {
            return false;
        }
        if stack.stack.is_none() {
            return data;
        }
        if stack.epoch != self.native_lifecycle.epoch() {
            return false;
        }
        let current = {
            let mut table = self.resident_entities.table.borrow_mut();
            let order = table.admission_order();
            order(entity)
        };
        if current.is_none() || stack.admissions.get(&entity).copied() != current {
            return false;
        }
        let row = stack
            .read_indices
            .get(&entity)
            .and_then(|index| stack.reads.get(*index));
        let stack = stack.stack.as_ref().expect("prepared equip stack");
        let Some(entity) = self.entity(entity) else {
            return false;
        };
        let Some(row) = row else {
            return false;
        };
        let host = crate::game::equip_host::EquipHost { game: self, row };
        let native = crate::game::native_hooks::NativeHookHost { host: &host };
        let mut can_equip = data;
        native.native_can_equip(
            stack,
            slot,
            mod_api::LivingRef::new(entity),
            kind,
            &mut can_equip,
        );
        can_equip
    }

    /// `can_equip` lent to a menu click that runs while the game is at hand (the player's own
    /// inventory menu, predicted or authoritative).
    pub(in crate::game) fn menu_can_equip(&self) -> MenuEquip<'_, '_> {
        MenuEquip {
            game: self,
            prepared: Default::default(),
        }
    }
}

/// The game lent to a menu click as its `CanEquip`. Each source scope stays prepared by its
/// scalar identity and revision, so candidate traversal never compares stack payloads.
pub(in crate::game) struct MenuEquip<'g, 'a> {
    game: &'g Game<'a>,
    prepared: std::cell::RefCell<
        std::collections::HashMap<u64, (horizon_client::mobs::equip::EquipStamp, EquipStack)>,
    >,
}

impl MenuEquip<'_, '_> {
    // A replacement may change to an item with no listeners. It still invalidates
    // this decision's preparation, before the new item's data-only fast path.
    fn stale_source(&self, stack: &ItemStack) -> bool {
        match horizon_client::mobs::equip::source_stamp_at(stack as *const _ as usize) {
            Some(Some(current)) => self
                .prepared
                .borrow()
                .get(&current.scope())
                .is_some_and(|(original, _)| *original != current),
            Some(None) => true,
            None => false,
        }
    }
}

impl CanEquip for MenuEquip<'_, '_> {
    fn prepare(&self, stack: &ItemStack, entity: u32, kind: &str) -> bool {
        if self.stale_source(stack) {
            return false;
        }
        if !self.listens(stack) {
            return true;
        }
        let Some(stamp) = horizon_client::mobs::equip::source_stamp(stack) else {
            return false;
        };
        if let Some((original, ready)) = self.prepared.borrow().get(&stamp.scope()) {
            return *original == stamp && !ready.refused;
        }
        let ready = self.game.equip_stack(stack, &[kind], &[entity]);
        let valid = !ready.refused;
        self.prepared
            .borrow_mut()
            .insert(stamp.scope(), (stamp, ready));
        valid
    }
    fn can_apply(&self, stack: &ItemStack, entity: u32) -> bool {
        if self.stale_source(stack) {
            return false;
        }
        if !self.listens(stack) {
            return true;
        }
        let Some(stamp) = horizon_client::mobs::equip::source_stamp(stack) else {
            return false;
        };
        let prepared = self.prepared.borrow();
        prepared
            .get(&stamp.scope())
            .is_some_and(|(original, prepared)| {
                *original == stamp && self.game.equip_can_apply(prepared, entity)
            })
    }
    fn listens(&self, stack: &ItemStack) -> bool {
        // Keep stale observed inputs on the refusal path even if their replacement
        // item has no listeners; fresh unsubscribed inputs still use the data path.
        self.stale_source(stack)
            || mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item)
    }
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        if self.stale_source(stack) {
            return false;
        }
        if !mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item) {
            return data;
        }
        let Some(stamp) = horizon_client::mobs::equip::source_stamp(stack) else {
            return false;
        };
        let prepared = self.prepared.borrow();
        let Some((original, ready)) = prepared.get(&stamp.scope()) else {
            return false;
        };
        if *original != stamp {
            return false;
        }

        self.game.can_equip(ready, slot, entity, kind, data)
    }
}

#[cfg(test)]
thread_local! { static EQUIP_CONVERSIONS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)}; }
#[cfg(test)]
pub(in crate::game) fn take_equip_conversions() -> usize {
    EQUIP_CONVERSIONS.with(|count| count.replace(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The part of an iron helmet: it lets the helmet on the feet only, and counts the asks.
    #[derive(Clone, Default)]
    struct FeetOnly(std::sync::Arc<AtomicUsize>);
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for FeetOnly {
        fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            let entity = event.entity.entity();
            assert!(entity.position(host).is_some());
            assert!(entity.kind(host).is_some());
            let _health = event.entity.health(host);
            let _alive = event.entity.alive(host);
            let _equipment = event.entity.equipment(host, mod_api::EquipmentSlot::Head);
            let _hand = event.entity.hand_item(host, mod_api::Hand::Main);
            self.0.fetch_add(1, Ordering::Relaxed);
            event.can_equip = event.slot == "feet";
        }
    }
    struct Cobbler(FeetOnly);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Cobbler {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.0.clone());
        }
    }

    /// An iron helmet with enchantments, a name and its own `equippable`: a stack whose
    /// conversion for the parts allocates.
    fn rich_helmet() -> ItemStack {
        use horizon_core::nbt::Tag;
        let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
        helmet.set_crafting_enchantments(vec![("minecraft:protection".into(), 4)]);
        let data = helmet.data_mut();
        data.custom_name = Some(Tag::String("Crown".into()));
        let component = Tag::compound([("slot", Tag::String("head".into()))]);
        data.set_other("minecraft:equippable", Some(component));
        helmet
    }

    /// Runs `test` on a game with the cobbler's part installed, given what it counted.
    fn with_cobbler(test: impl FnOnce(&mut Game<'_>, &AtomicUsize)) {
        installed(|asked| {
            crate::game::native_entity_refs::tests::with_game(|game| {
                // The player in place, admitted as a joined one is, so its handle resolves.
                let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
                let mut body = horizon_client::motion::PlayerMotion::new(game.motion, at).unwrap();
                body.entity_id = 7;
                game.player = Some(body);
                game.bind_local_player_residency();
                test(game, asked)
            });
        });
    }

    /// Runs `test` with the cobbler's mod installed, given what its part counted.
    fn installed(test: impl FnOnce(&AtomicUsize)) {
        let part = FeetOnly::default();
        let cobbler = mod_api::ModEntry::new("cobbler", Box::new(Cobbler(part.clone())));
        let mods: mod_api::Mods = vec![cobbler].into();
        crate::entry::install_client_hooks();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || test(&part.0));
    }

    /// Asked once per candidate slot, a warmed `can_equip` allocates nothing, for a stack with
    /// components, on every path: a menu the game is lent to (the player's, a guest's), and the
    /// dispenser's and a pickup's prepared stack; nor does an item no part listens to. Each
    /// answer is the part's.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn warmed_can_equip_questions_allocate_nothing() {
        with_cobbler(|game, asked| {
            let (helmet, other) = (
                horizon_client::mobs::equip::EquipInput::new(1, rich_helmet()),
                ItemStack::new("minecraft:diamond_helmet", 1),
            );
            game.armor[0] = Some(rich_helmet());
            let (player, kind) = (game.own_entity_id() as u32, "minecraft:player");
            let slots = ["head", "chest", "legs", "feet"];
            let feet_only = [false, false, false, true];
            let lent = game.menu_can_equip();
            assert!(lent.prepare(&helmet, player, kind));
            assert!(!lent.can_equip(&helmet, "head", player, kind, true));
            let (answers, allocations) = horizon_threads::allocation_count::measure(|| {
                slots.map(|slot| lent.can_equip(&helmet, slot, player, kind, slot == "head"))
            });
            assert_eq!((answers, allocations), (feet_only, 0));
            let (answers, allocations) = horizon_threads::allocation_count::measure(|| {
                slots.map(|slot| lent.can_equip(&other, slot, player, kind, slot == "head"))
            });
            assert_eq!((answers, allocations), ([true, false, false, false], 0));
            let prepared = game.equip_stack(&helmet, &["minecraft:player"], &[player]);
            assert!(
                prepared.reads[0].equipment[5]
                    .as_ref()
                    .unwrap()
                    .components()
                    .is_some()
            );
            let (answers, allocations) = horizon_threads::allocation_count::measure(|| {
                slots.map(|slot| game.can_equip(&prepared, slot, player, kind, slot == "head"))
            });
            assert_eq!((answers, allocations), (feet_only, 0));
            assert_eq!(asked.load(Ordering::Relaxed), 1 + 4 + 4);
        });
    }

    /// A guest's right click with a bundle on the cursor on an empty armour slot takes the
    /// bundle's next stack out into it (`BundleItem.overrideStackedOnOther`), and the host's
    /// authoritative menu asks the parts about that inner stack: the cobbler's part keeps the iron
    /// helmet inside off the head, and the bundle, the armour and the inventory stay as they were;
    /// it lets it onto the feet, and the bundle is empty after.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_guests_bundle_puts_on_what_the_part_allows() {
        use horizon_client::bundle::Contents;
        use horizon_protocol::packets::play::{ContainerClick, Serverbound};
        installed(|asked| {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.reset_entity_residency();
                let mut bundle = ItemStack::new("minecraft:bundle", 1);
                let mut contents = Contents::of(game.items(), &bundle).unwrap();
                let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
                assert_eq!(contents.try_insert(game.items(), &mut helmet), 1);
                contents.store(&mut bundle);
                let held = |game: &Game<'_>| {
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    let name = |s: &Option<ItemStack>| s.as_ref().map(|s| s.item.to_string());
                    let inside = (guest.carried.as_ref())
                        .and_then(|b| Contents::of(game.items(), b))
                        .map(|c| {
                            c.items()
                                .iter()
                                .map(|s| s.item.to_string())
                                .collect::<Vec<_>>()
                        });
                    let inventory = guest.inventory.iter().filter(|s| s.is_some()).count();
                    (guest.armor.each_ref().map(name), inside, inventory)
                };
                // Right clicks (button 1) of the pickup mode on menu slots 5 (head) and 8 (feet).
                let click = |game: &mut Game<'_>, slot: i16| {
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.carried = Some(bundle.clone());
                    });
                    let click = ContainerClick {
                        container: 0,
                        state: 0,
                        slot,
                        button: 1,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    };
                    game.lan_guest_packet(12, Serverbound::ContainerClick(click));
                };
                let helmet = "minecraft:iron_helmet".to_owned();
                take_equip_conversions();
                click(game, 5);
                assert_eq!(take_equip_conversions(), 1);
                assert_eq!(
                    held(game),
                    ([None, None, None, None], Some(vec![helmet.clone()]), 0)
                );
                click(game, 8);
                assert_eq!(take_equip_conversions(), 1);
                assert_eq!(
                    held(game),
                    ([None, None, None, Some(helmet)], Some(Vec::new()), 0)
                );
                // The part was asked about the helmet taken out of the bundle.
                assert!(asked.load(Ordering::Relaxed) > 0);
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn an_unpreparable_listening_stack_refuses_instead_of_using_the_data_answer() {
        with_cobbler(|game, asked| {
            let player = game.own_entity_id() as u32;
            let empty = horizon_client::mobs::equip::EquipInput::new(
                42,
                ItemStack::new("minecraft:iron_helmet", 0),
            );
            let prepared = game.equip_stack(&empty, &["minecraft:player"], &[player]);
            assert!(!game.can_equip(&prepared, "head", player, "minecraft:player", true));
            assert!(!game.equip_can_apply(&prepared, player));
            assert_eq!(asked.load(Ordering::Relaxed), 0);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn local_shift_click_prepares_once_and_preserves_a_refused_helmet() {
        with_cobbler(|game, asked| {
            use horizon_client::menu::{Input, Kind, Menu};
            let lent = game.menu_can_equip();
            let mut env = game.menu_env();
            env.can_equip = Some(&lent);
            let mut menu = Menu::new(Kind::Inventory);
            let helmet = rich_helmet();
            menu.set_slot(&env, 9, Some(helmet.clone()));
            take_equip_conversions();
            menu.click(&env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(take_equip_conversions(), 1);
            assert_eq!(asked.load(Ordering::Relaxed), 1);
            assert_eq!(menu.slot(9), Some(&helmet));
            assert!(menu.slot(5).is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn menu_replacement_with_an_unsubscribed_item_stays_stale() {
        use horizon_client::mobs::equip::EquipDecision;
        with_cobbler(|game, asked| {
            let player = game.own_entity_id() as u32;
            let lent = game.menu_can_equip();
            let mut slot = Some(rich_helmet());
            let mut source = EquipDecision::new(42, &mut slot);
            take_equip_conversions();
            assert!(lent.prepare(&source, player, "minecraft:player"));
            assert!(lent.can_equip(&source, "feet", player, "minecraft:player", false));
            assert_eq!(take_equip_conversions(), 1);
            source.replace(Some(ItemStack::new("minecraft:diamond_helmet", 3)));
            assert!(!lent.prepare(&source, player, "minecraft:player"));
            assert!(!lent.can_equip(&source, "head", player, "minecraft:player", true));
            assert!(!lent.can_apply(&source, player));
            let mut env = game.menu_env();
            env.can_equip = Some(&lent);
            let menu = horizon_client::menu::Menu::new(horizon_client::menu::Kind::Inventory);
            assert!(!menu.takes_one(&env, 5, &source));
            assert_eq!(asked.load(Ordering::Relaxed), 1);
            assert_eq!(take_equip_conversions(), 0);
            // A separate unsubscribed input still keeps the missing-target data answer.
            let other = ItemStack::new("minecraft:diamond_helmet", 1);
            assert!(lent.can_equip(&other, "head", 9999, "minecraft:player", true));
            assert!(menu.takes_one(&env, 5, &other));
            assert_eq!(take_equip_conversions(), 0);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn source_mutations_refuse_prepared_equip_without_reconversion_or_callback() {
        use horizon_client::mobs::equip::EquipDecision;
        with_cobbler(|game, asked| {
            let player = game.own_entity_id() as u32;
            for mutation in 0..5 {
                let mut helmet = rich_helmet();
                helmet.count = 3;
                let mut slot = Some(helmet);
                let mut source = EquipDecision::new(42, &mut slot);
                take_equip_conversions();
                let prepared = game.equip_stack(&source, &["minecraft:player"], &[player]);
                assert_eq!(take_equip_conversions(), 1);
                assert!(game.can_equip(&prepared, "feet", player, "minecraft:player", false));
                let before = asked.load(Ordering::Relaxed);
                match mutation {
                    0 => *source.count_mut().unwrap() = 2,
                    1 => {
                        source.data_mut().unwrap().custom_name =
                            Some(horizon_core::nbt::Tag::String("Changed crown".into()))
                    }
                    2 => {
                        source.replace(Some(rich_helmet()));
                    }
                    3 => {
                        source.remove();
                    }
                    4 => {
                        *source.count_mut().unwrap() = 2;
                        *source.count_mut().unwrap() = 3;
                    }
                    _ => unreachable!(),
                }
                assert!(!game.can_equip(&prepared, "feet", player, "minecraft:player", true));
                assert_eq!(asked.load(Ordering::Relaxed), before);
                assert_eq!(take_equip_conversions(), 0);
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn missing_despawned_and_readmitted_entities_refuse_but_no_listener_keeps_data() {
        use horizon_client::{mobs::Mob, mobs::equip::EquipInput};
        with_cobbler(|game, asked| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let at = horizon_core::geometry::Vec3::new(6.5, 64.0, 5.5);
            let index = game.mobs.len();
            game.mobs.push(Mob::new(21, kind.clone(), at, 0.0));
            let source = EquipInput::new(1, rich_helmet());
            let prepared = game.equip_stack(&source, &["minecraft:zombie"], &[21]);
            assert!(game.can_equip(&prepared, "feet", 21, "minecraft:zombie", false));
            let before = asked.load(Ordering::Relaxed);
            assert!(!game.can_equip(&prepared, "feet", 9999, "minecraft:zombie", true));
            game.mobs.remove(index);
            assert!(!game.can_equip(&prepared, "feet", 21, "minecraft:zombie", true));
            game.mobs.push(Mob::new(21, kind, at, 0.0));
            assert!(!game.can_equip(&prepared, "feet", 21, "minecraft:zombie", true));
            assert_eq!(asked.load(Ordering::Relaxed), before);
            take_equip_conversions();
            let other = ItemStack::new("minecraft:diamond_helmet", 3);
            let data = game.equip_stack(&other, &[], &[]);
            assert_eq!(take_equip_conversions(), 0);
            assert!(game.can_equip(&data, "head", 9999, "minecraft:zombie", true));
        });
    }
}
