//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::testing::{FakeContainer, FakeHost, FakeJournal, FakePlayer};

    #[cfg(test)]
    /// The slots of the chest at `pos`.
    pub(crate) fn chest(host: &mut FakeHost, pos: [i32; 3]) -> &mut FakeContainer {
        host.level.block_entity(pos).items.as_mut().unwrap()
    }

    #[cfg(test)]
    /// Two three-slot game chests, `FROM`, holding `from`, and an empty `INTO`.
    pub(crate) fn two_chests(from: [Option<ItemStack>; 3]) -> FakeHost {
        let mut host = FakeHost::new();
        host.level.block_entity(FROM).items = Some({
            let mut value = FakeContainer::new(0);
            value.slots = from.into();
            value.limit = 64;
            value
        });
        host.level.block_entity(INTO).with_items(3);
        host
    }
    #[cfg(test)]
    /// Player 1's items, counted with what they hold, and with what was given to them.
    pub(crate) fn held_items(host: &FakeHost) -> u32 {
        let player = player_of(host);
        items(player.slots.values().chain(&player.given).cloned())
    }

    #[cfg(test)]
    /// Puts `stack` in player 1's main hand.
    pub(crate) fn set_hand(host: &mut FakeHost, stack: ItemStack) {
        player_mut(host).slots.insert(5, stack);
    }
    #[cfg(test)]
    /// What player 1 of a [`player_world`] holds in their main hand.
    pub(crate) fn main_hand(host: &FakeHost) -> Option<ItemStack> {
        player_of(host).main_hand().cloned()
    }
    #[cfg(test)]
    /// Player 1 of a [`player_world`], to change.
    pub(crate) fn player_mut(host: &mut FakeHost) -> &mut FakePlayer {
        host.players.get_mut(1).unwrap()
    }
    #[cfg(test)]
    /// Player 1 of a [`player_world`].
    pub(crate) fn player_of(host: &FakeHost) -> &FakePlayer {
        host.players.get(1).unwrap()
    }
    #[cfg(test)]
    /// Player 1, whose main hand is hotbar slot 5, holding `main_hand`; entity 2 a pig, 3 a mod's
    /// robot and 4 a player in another level. An iron helmet stacks to one and goes on the head.
    pub(crate) fn player_world(main_hand: Option<ItemStack>) -> FakeHost {
        let mut host = FakeHost::new();
        let player = host.join(1, 1);
        player.selected_slot = 5;
        player.slots.extend(main_hand.map(|stack| (5, stack)));
        host.level.entity(2, "minecraft:pig");
        host.level.entity(3, "test:robot");
        host.resolve_entity_types().unwrap();
        let nether = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::NETHER);
        host.players.join(4, 4, nether);
        let head = [("slot".into(), Tag::String("head".into()))];
        host.registries
            .item_component(
                "minecraft:iron_helmet",
                "minecraft:max_stack_size",
                Tag::Int(1),
            )
            .item_component(
                "minecraft:iron_helmet",
                "minecraft:equippable",
                Tag::Compound(head.into_iter().collect()),
            );
        host
    }

    use super::*;
    use crate::parts::tests::*;

    #[test]
    pub(crate) fn item_keys_validate_forward_references_and_reused_slots() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        struct Keys(Arc<std::sync::Mutex<Vec<crate::ItemKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Keys {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let vanilla = r.item_key("stick");
                assert_eq!(vanilla, r.item_key("minecraft:stick"));
                let later = r.item_key("later:widget");
                assert_eq!(
                    mod_vocab::__private::parts_installation::validate_item_key(
                        mod_vocab::__private::parts_installation::item_key_slot(vanilla)
                    ),
                    Ok(vanilla)
                );
                assert!(
                    mod_vocab::__private::parts_installation::validate_item_key(
                        mod_vocab::__private::parts_installation::item_key_slot(later) + 1
                    )
                    .is_err()
                );
                *self.0.lock().unwrap() = vec![vanilla, later];
            }
        }
        let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new("requester", Box::new(Keys(keys.clone())))].into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        let check = |known: &dyn Fn(&str) -> bool| {
            resolve_parts(
                &HashMap::new(),
                &HashMap::new(),
                KnownIds {
                    block: &|_| false,
                    item: known,
                },
            )
        };
        check(&|id| ["minecraft:stick", "later:widget"].contains(&id)).unwrap();
        let keys = keys.lock().unwrap();
        assert_eq!(
            mod_vocab::__private::parts_installation::item_id(keys[0]),
            Some("minecraft:stick")
        );
        assert_eq!(
            mod_vocab::__private::parts_installation::item_id(keys[1]),
            Some("later:widget")
        );
        let super::ResolvePartsError::ItemKeys(errors) =
            check(&|id| id == "minecraft:stick").unwrap_err()
        else {
            panic!("wrong refusal")
        };
        assert_eq!(
            errors,
            [mod_vocab::__private::parts_installation::item_key_error(
                "requester",
                "later:widget".into()
            )]
        );
    }

    /// A pipe's item capability, reached through `.0`: one slot from every side but its top,
    /// and what goes in is handed to the container east of it, through that container's west
    /// side.
    pub(crate) struct Pipe(pub(crate) BlockSide);
    impl ResourceHandler<ItemResource> for Pipe {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            usize::from(self.0.side != Some(Direction::Up))
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let [x, y, z] = self.0.pos;
            let east = [x + 1, y, z];
            let Some(next) = self
                .0
                .level
                .capability(host, &ITEMS, east, Some(Direction::West))
            else {
                return 0;
            };
            next.insert(host, tx, 0, resource, amount)
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    /// A pipe that hands what it's offered on as [`Pipe`] does, and answers one more than went.
    pub(crate) struct Greedy(pub(crate) BlockSide);
    impl ResourceHandler<ItemResource> for Greedy {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            Pipe(self.0).insert(host, tx, index, resource, amount) + 1
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    #[cfg(test)]
    pub(crate) const PIPE: [i32; 3] = [0, 64, 0];
    #[cfg(test)]
    pub(crate) const CHEST: [i32; 3] = [1, 64, 0];

    pub(crate) fn stick() -> ItemStack {
        stack("minecraft:stick", 1, 0)
    }

    pub(crate) fn fixture_item_key(id: &str) -> ItemKey {
        if !mod_vocab::__private::parts_installation::parts_installed() {
            install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        }
        mod_vocab::__private::resolve_items([
            "minecraft:stick",
            "minecraft:iron_helmet",
            "minecraft:iron_sword",
            "minecraft:diamond",
            "minecraft:emerald",
            "minecraft:potion",
            "minecraft:stone",
            "minecraft:apple",
            "minecraft:bread",
            "minecraft:iron_ingot",
            "minecraft:cod",
            "minecraft:salmon",
            "minecraft:gold_ingot",
            "test:pouch",
            "test:bag",
            "test:case",
            "test:cell",
            "test:outer",
            "test:ordered_item",
            "test:pipe",
            "test:item",
            "test:item_overstates",
            "test:lying",
            "test:none",
            "test:robot",
        ])
        .unwrap();
        mod_vocab::__private::item_key(id).expect("explicit fixture item")
    }

    pub(crate) fn stack(item: &str, count: u32, damage: u32) -> ItemStack {
        let mut stack =
            ItemStack::new(fixture_item_key(item), count).expect("fixture stacks are nonempty");
        if damage != 0 {
            stack.set_damage(damage);
        }
        stack
    }

    /// A pouch of sticks at `.0`: one slot, its stick count kept in the pouch's own stack (its
    /// damage, for the test), so the handler holds nothing.
    pub(crate) struct Pouch(pub(crate) ItemPlace);
    impl ResourceHandler<ItemResource> for Pouch {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
            (self.amount(host, index) > 0).then(|| ItemResource::of(&stick()))
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            self.0.stack(host).map_or(0, |pouch| pouch.damage())
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, resource: &ItemResource) -> u32 {
            if resource.matches(&stick()) { 64 } else { 0 }
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let Some(pouch) = self.0.stack(host).filter(|_| resource.matches(&stick())) else {
                return 0;
            };
            let filled = ItemResource::of(&stack("test:pouch", 1, pouch.damage() + amount));
            match self.0.exchange(host, tx, &filled, 1) {
                1 => amount,
                _ => 0,
            }
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    /// A bag at `.0` holding one pouch: the pouch's sticks plus one kept in the bag's damage, 0
    /// when empty.
    pub(crate) struct Bag(pub(crate) ItemPlace);
    impl ResourceHandler<ItemResource> for Bag {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            let held = self.0.stack(host)?.damage();
            (held > 0).then(|| ItemResource::of(&stack("test:pouch", 1, held - 1)))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            u32::from(self.resource(host, index).is_some())
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, resource: &ItemResource) -> u32 {
            u32::from(resource.item() == fixture_item_key("test:pouch"))
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let pouch = resource.to_stack(1).unwrap();
            match self.0.stack(host) {
                Some(bag)
                    if bag.damage() == 0
                        && amount > 0
                        && resource.item() == fixture_item_key("test:pouch") =>
                {
                    let full = ItemResource::of(&stack("test:bag", 1, pouch.damage() + 1));
                    self.0.exchange(host, tx, &full, 1)
                }
                _ => 0,
            }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            if amount == 0 || self.resource(host, index).as_ref() != Some(resource) {
                return 0;
            }
            let empty = ItemResource::of(&stack("test:bag", 1, 0));
            self.0.exchange(host, tx, &empty, 1)
        }
    }

    /// A case at `.0` with two slots kept in its damage: diamonds in the hundreds, sticks in
    /// the units. Before taking sticks it tries emptying its diamond slot in a transaction of
    /// its own and aborts that, as a handler checking what a write would do may.
    pub(crate) struct Case(pub(crate) ItemPlace);
    impl Case {
        fn held(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            let damage = self.0.stack(host).map_or(0, |case| case.damage());
            [damage / 100, damage % 100][index]
        }
        fn rewrite(&self, host: &mut dyn ServerHost, tx: &Transaction, damage: u32) -> bool {
            let case = ItemResource::of(&stack("test:case", 1, damage));
            self.0.exchange(host, tx, &case, 1) == 1
        }
    }
    impl ResourceHandler<ItemResource> for Case {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            2
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
            let item = ["minecraft:diamond", "minecraft:stick"][index];
            (self.held(host, index) > 0).then(|| ItemResource::of(&stack(item, 1, 0)))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            self.held(host, index)
        }
        fn capacity(&self, _: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> u32 {
            let item = ["minecraft:diamond", "minecraft:stick"][index];
            if resource.item() == fixture_item_key(item) {
                99
            } else {
                0
            }
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            if index != 1 || resource.item() != fixture_item_key("minecraft:stick") {
                return 0;
            }
            let damage = self.0.stack(host).map_or(0, |case| case.damage());
            match self.rewrite(host, tx, damage + amount) {
                true => amount,
                false => 0,
            }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            if index != 1 || resource.item() != fixture_item_key("minecraft:stick") {
                return 0;
            }
            let damage = self.0.stack(host).map_or(0, |case| case.damage());
            let tried: Result<(), ()> = host.transaction(|host, tx| {
                self.rewrite(host, tx, damage % 100);
                Err(())
            });
            assert!(tried.is_err());
            let damage = self.0.stack(host).unwrap().damage();
            assert_eq!(damage / 100, 1, "diamond restored after inner abort");
            let took = amount.min(damage % 100);
            match self.rewrite(host, tx, damage - took) {
                true => took,
                false => 0,
            }
        }
    }

    /// An accessory mod's charm slots: charm `key` is kept in player 1's inventory slot 20 +
    /// `key`. A charm slot has nowhere to put what doesn't stack with its charm, so it refuses
    /// that.
    pub(crate) struct Charms;
    impl PlaceProvider for Charms {
        fn stack(&self, host: &mut dyn ServerHost, key: u64) -> Option<ItemStack> {
            backend_mut_of(host).and_then(|backend| backend.game_stack(&charm_slot(key)))
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            key: u64,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let place = charm_slot(key);
            match place.stack(host) {
                Some(held) if !resource.matches(&held) => 0,
                held => {
                    let room = 64 - held.map_or(0, |held| held.count());
                    place.insert(host, tx, resource, amount.min(room))
                }
            }
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            key: u64,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            charm_slot(key).extract(host, tx, resource, amount)
        }
    }
    pub(crate) fn charm_slot(key: u64) -> ItemPlace {
        ItemPlace::PlayerSlot {
            player: 1,
            slot: 20 + key as i32,
        }
    }

    pub(crate) const HAND: ItemPlace = ItemPlace::PlayerHand {
        player: 1,
        hand: Hand::Main,
    };
    /// A mod robot's three empty slots.
    pub(crate) struct Cargo;
    impl ResourceHandler<ItemResource> for Cargo {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            3
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            0
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    #[test]
    pub(crate) fn a_players_items_are_their_inventory_and_each_index_takes_only_what_fits_there() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let entity = |id| EntityRef::new(WorldEpoch::new(1), id);
        let helmet = || stack("minecraft:iron_helmet", 1, 0);
        let mut player = player_world(Some(stack("minecraft:stick", 60, 0)));
        player_mut(&mut player)
            .slots
            .insert(3, stack("minecraft:stick", 10, 0));
        let host: &mut dyn ServerHost = &mut player;
        let items = entity(1).capability(host, &ENTITY_ITEMS, ()).unwrap();
        assert_eq!(items.owner(), "the game");
        assert_eq!(items.size(host), 41);
        let (sticks, helmet_) = (ItemResource::of(&stick()), ItemResource::of(&helmet()));
        // The main inventory takes a stack of anything; an armour slot only what goes there,
        // a helmet on the head (index 39), not the feet (36).
        assert_eq!(items.capacity(host, 0, &sticks), 64);
        assert_eq!(items.capacity(host, 36, &sticks), 0);
        assert_eq!(items.capacity(host, 39, &helmet_), 1);
        assert_eq!(items.capacity(host, 36, &helmet_), 0);
        assert_eq!(items.capacity(host, 40, &helmet_), 1);
        assert_eq!(items.capacity(host, 39, &sticks), 0);
        assert_eq!(items.amount(host, 3), 10);
        // Into one index, no further: 54 of 60 fill slot 3, and nothing goes elsewhere.
        let insert = |host: &mut dyn ServerHost, index, resource: &ItemResource, amount| {
            let went = host.transaction(|host, tx| {
                Ok::<_, ()>(items.insert(host, tx, index, resource, amount))
            });
            went.unwrap()
        };
        assert_eq!(insert(host, 3, &sticks, 60), 54);
        assert_eq!(insert(host, 38, &sticks, 1), 0);
        assert_eq!(insert(host, 39, &helmet_, 1), 1);
        assert_eq!(player_of(&player).slots[&3].count(), 64);
        assert_eq!(player_of(&player).slots[&103], helmet());
        assert!(player_of(&player).given.is_empty());
        // Without an index, as `Inventory.add`: the selected slot's sticks first, then an
        // empty slot.
        let host: &mut dyn ServerHost = &mut player;
        let aborted = host.transaction(|host, tx| {
            assert_eq!(items.insert_any(host, tx, &sticks, 10), 10);
            Err::<(), ()>(())
        });
        assert!(aborted.is_err());
        assert_eq!(
            player_of(&player).slots[&5],
            stack("minecraft:stick", 60, 0)
        );
        let host: &mut dyn ServerHost = &mut player;
        let went =
            host.transaction(|host, tx| Ok::<_, ()>(items.insert_any(host, tx, &sticks, 10)));
        assert_eq!(went, Ok(10));
        assert_eq!(
            player_of(&player).slots[&5],
            stack("minecraft:stick", 64, 0)
        );
        assert_eq!(player_of(&player).slots[&0], stack("minecraft:stick", 6, 0));
        assert!(player_of(&player).given.is_empty());
    }

    #[test]
    pub(crate) fn an_entitys_items_come_from_its_types_provider_and_none_outside_the_level_in_place()
     {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let entity = |id| EntityRef::new(WorldEpoch::new(1), id);
        let mut player = player_world(None);
        let host: &mut dyn ServerHost = &mut player;
        // A pig has no items; a mod's robot has its provider's; a player in a parked level
        // is asked nothing.
        assert!(entity(2).capability(host, &ENTITY_ITEMS, ()).is_none());
        let robot = entity(3).capability(host, &ENTITY_ITEMS, ()).unwrap();
        assert_eq!((robot.owner(), robot.size(host)), ("doubler".to_owned(), 3));
        assert!(entity(4).capability(host, &ENTITY_ITEMS, ()).is_none());
    }

    #[cfg(test)]
    /// Items counted with what they hold: a pouch's sticks, a bag's pouch and its sticks.
    pub(crate) fn items(stacks: impl IntoIterator<Item = ItemStack>) -> u32 {
        stacks
            .into_iter()
            .map(
                |s| match mod_vocab::__private::parts_installation::item_id(s.item()).unwrap() {
                    "test:pouch" | "test:bag" => s.count() * (1 + s.damage()),
                    _ => s.count(),
                },
            )
            .sum()
    }
    #[cfg(test)]
    /// Three shelf slots at (2, 64, 0), answered by the game's handler, else by
    /// `ItemOverstates`. A slot takes up to `limit` of one item, the `output` slot none, and
    /// gives at most one item per extract, unless `whole_extract`; with `one_per_insert`, it
    /// takes at most one per insert.
    #[derive(Clone)]
    pub(crate) struct Shelf {
        slots: [Option<ItemStack>; 3],
        limit: u32,
        output: Option<usize>,
        one_per_insert: bool,
        whole_extract: bool,
        overstates: bool,
        journal: FakeJournal<[Option<ItemStack>; 3]>,
    }
    #[cfg(test)]
    pub(crate) fn shelf(slots: [Option<ItemStack>; 3]) -> Shelf {
        Shelf {
            slots,
            limit: 64,
            output: None,
            one_per_insert: false,
            whole_extract: false,
            overstates: false,
            journal: FakeJournal::default(),
        }
    }
    pub(crate) const SHELF: [i32; 3] = [2, 64, 0];
    #[cfg(test)]
    pub(crate) fn shelf_slot(slot: usize) -> ItemPlace {
        ItemPlace::BlockEntity {
            level: here(),
            pos: SHELF,
            side: None,
            slot,
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::LevelsBackend for Shelf {
        fn level_in_place(&self, level: LevelRef) -> bool {
            level == here()
        }
        /// The shelf stands in no named block, so no block's providers answer for it.
        fn level_block(&self, _: LevelRef, _: [i32; 3]) -> Option<crate::BlockKey> {
            None
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::BlockEntitiesBackend for Shelf {
        fn block_entity_kind(&self, _: BlockEntityRef) -> Option<BlockEntityTypeKey> {
            self.overstates.then(|| type_key("test:item_overstates"))
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::ItemsBackend for Shelf {
        fn container_capacity(&self, _: BlockSide, slot: usize, _: &ItemStack) -> u32 {
            if self.output == Some(slot) {
                0
            } else {
                self.limit
            }
        }
        fn container_size(&self, BlockSide { pos, .. }: BlockSide) -> Option<usize> {
            (pos == SHELF).then_some(3)
        }
        fn container_stack(&self, _: BlockSide, slot: usize) -> Option<ItemStack> {
            self.slots.get(slot)?.clone()
        }
    }
    #[cfg(test)]
    impl Backend for Shelf {}
    #[cfg(test)]
    mod_api::impl_backend_areas!(Shelf, except(block_entities, items, levels));
    #[cfg(test)]
    impl mod_api::prelude::ItemsBackendMut for Shelf {
        fn container_insert(
            &mut self,
            _: BlockSide,
            slot: usize,
            resource: &ItemResource,
            amount: u32,
            _: &Transaction,
        ) -> Result<Option<ItemStack>, crate::StackWriteError> {
            let Some(stack) = resource.to_stack(amount) else {
                return Ok(None);
            };
            let Some(held) = self.slots.get(slot).cloned() else {
                return Ok(Some(stack));
            };
            let room = match &held {
                _ if self.output == Some(slot) => 0,
                None => self.limit,
                Some(held) if held.is_stackable_with(&stack) => self.limit - held.count(),
                Some(_) => 0,
            };
            let most = if self.one_per_insert { 1 } else { u32::MAX };
            let put = room.min(stack.count()).min(most);
            if put > 0 {
                let mut inserted = stack.clone();
                inserted
                    .set_count(held.map_or(0, |h| h.count()) + put)
                    .unwrap();
                self.slots[slot] = Some(inserted);
            }
            Ok(stack.shrink(put))
        }
        fn container_extract(
            &mut self,
            _: BlockSide,
            slot: usize,
            max: u32,
            _: &Transaction,
        ) -> Option<ItemStack> {
            let most = if self.whole_extract { max } else { 1 };
            let split = self.slots.get_mut(slot)?.take()?.split(most);
            self.slots[slot] = split.remainder;
            split.taken
        }
    }
    #[cfg(test)]
    impl BackendMut for Shelf {}
    #[cfg(test)]
    mod_api::impl_backend_mut_areas!(Shelf, except(items));
    #[cfg(test)]
    impl mod_api::prelude::ItemsHost for Shelf {
        fn item_default_component(&self, item: ItemKey, component: &str) -> Option<Tag> {
            // An iron sword's, as its item report gives them.
            let default = match component {
                "minecraft:max_damage" => Tag::Int(250),
                "minecraft:damage" => Tag::Int(0),
                "minecraft:enchantable" => {
                    Tag::Compound([("value".into(), Tag::Int(14))].into_iter().collect())
                }
                _ => return None,
            };
            (item == fixture_item_key("minecraft:iron_sword")).then_some(default)
        }
    }
    #[cfg(test)]
    impl mod_api::prelude::BlockEntitiesHost for Shelf {
        fn block_entity(&self, level: LevelRef, position: [i32; 3]) -> Option<BlockEntityRef> {
            Some(crate::__private::game::block_entity_ref(level, position, 1))
        }
    }
    #[cfg(test)]
    impl Host for Shelf {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_ref(self))
        }
    }
    #[cfg(test)]
    mod_api::impl_host_areas!(Shelf, except(block_entities, items));
    #[cfg(test)]
    impl ServerHost for Shelf {
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_mut_ref(
                self,
            ))
        }
    }
    #[cfg(test)]
    mod_api::impl_server_host_areas!(Shelf, except(mod_state));
    #[cfg(test)]
    impl crate::prelude::ModStateServerHost for Shelf {
        fn transaction_open(&mut self) -> TransactionSeal {
            self.journal.open(self.slots.clone())
        }
        fn transaction_close(&mut self, seal: &TransactionSeal, commit: bool) {
            if let Some(slots) = self.journal.close(seal, commit) {
                self.slots = slots;
            }
        }
    }

    #[cfg(test)]
    impl Shelf {
        #[cfg(test)]
        fn items(&self) -> u32 {
            items(self.slots.iter().flatten().cloned())
        }
    }

    /// A mod's handler for the shelf whose calls overstate: an extract takes and answers 5 more
    /// than asked, and an insert keeps nothing and answers 5 more than was offered.
    pub(crate) struct ItemOverstates;
    impl ResourceHandler<ItemResource> for ItemOverstates {
        fn size(&self, host: &mut dyn ServerHost) -> usize {
            backend_of(&*host)
                .and_then(|backend| backend.container_size(on(SHELF, None)))
                .unwrap_or(0)
        }
        fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
            Some(ItemResource::of(
                &backend_of(&*host)?.container_stack(on(SHELF, None), index)?,
            ))
        }
        fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
            backend_of(&*host)
                .and_then(|backend| backend.container_stack(on(SHELF, None), index))
                .map_or(0, |held| held.count())
        }
        fn capacity(
            &self,
            host: &mut dyn ServerHost,
            index: usize,
            resource: &ItemResource,
        ) -> u32 {
            backend_of(&*host).map_or(0, |backend| {
                backend.container_capacity(on(SHELF, None), index, &resource.to_stack(1).unwrap())
            })
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            amount: u32,
        ) -> u32 {
            amount + 5
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            index: usize,
            _: &ItemResource,
            amount: u32,
        ) -> u32 {
            let Some(backend) = backend_mut_of(host) else {
                return 0;
            };
            backend
                .container_extract(on(SHELF, None), index, amount + 5, tx)
                .map_or(0, |out| out.count())
        }
    }

    #[test]
    pub(crate) fn an_item_handler_writes_its_stack_back_where_it_lives_splitting_one_off() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let mut player = player_world(Some(stack("test:pouch", 2, 0)));
        // Aborted, nothing changes.
        let host: &mut dyn ServerHost = &mut player;
        let pouch = HAND.capability(host, &ITEM_ITEMS).unwrap();
        let sticks = ItemResource::of(&stick());
        let aborted: Result<(), u32> =
            host.transaction(|host, tx| Err(pouch.insert(host, tx, 0, &sticks, 3)));
        assert_eq!(aborted, Err(3));
        assert_eq!(main_hand(&player), Some(stack("test:pouch", 2, 0)));
        assert!(player_of(&player).given.is_empty());
        // One of the two pouches is filled: it leaves the hand for the player, the other stays.
        let host: &mut dyn ServerHost = &mut player;
        assert_eq!(fill(host, &HAND, stick(), 3), 3);
        assert_eq!(main_hand(&player), Some(stack("test:pouch", 1, 0)));
        assert_eq!(player_of(&player).given, [stack("test:pouch", 1, 3)]);
        assert_eq!(held_items(&player), 2 + 3);
        // An item with no part and no game slots has no answer.
        set_hand(&mut player, stack("minecraft:stone", 1, 0));
        let host: &mut dyn ServerHost = &mut player;
        assert!(HAND.capability(host, &ITEM_ITEMS).is_none());
    }

    /// A mod's energy cell at `.0`: up to 1000, kept in the cell's own stack (its damage, for the
    /// test), so the handler holds nothing.
    pub(crate) struct Cell(pub(crate) ItemPlace);
    impl Cell {
        /// The cell here rewritten as holding `energy`: `moved` when that goes through, else 0.
        fn rewrite(
            &self,
            host: &mut dyn ServerHost,
            tx: &Transaction,
            energy: u32,
            moved: u32,
        ) -> u32 {
            let charged = ItemResource::of(&stack("test:cell", 1, energy));
            match self.0.exchange(host, tx, &charged, 1) {
                1 => moved,
                _ => 0,
            }
        }
    }
    impl EnergyHandler for Cell {
        fn amount(&self, host: &mut dyn ServerHost) -> u32 {
            self.0.stack(host).map_or(0, |cell| cell.damage())
        }
        fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
            1000
        }
        fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            match amount.min(1000 - held) {
                0 => 0,
                went => self.rewrite(host, tx, held + went, went),
            }
        }
        fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let held = self.amount(host);
            match amount.min(held) {
                0 => 0,
                came => self.rewrite(host, tx, held - came, came),
            }
        }
    }

    #[test]
    pub(crate) fn an_items_energy_moves_through_its_place_and_a_charged_one_splits_off_its_stack() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let slot = ItemPlace::PlayerSlot { player: 1, slot: 9 };
        let mut player = player_world(Some(stack("test:cell", 2, 600)));
        player_mut(&mut player)
            .slots
            .insert(9, stack("test:cell", 1, 900));
        let host: &mut dyn ServerHost = &mut player;
        let hand = HAND.capability(host, &ITEM_ENERGY).unwrap();
        let other = slot.capability(host, &ITEM_ENERGY).unwrap();
        // The other cell has room for 100: one of the two in the hand gives that and splits off.
        let moved = host.transaction(|host, tx| move_energy(host, &hand, &other, 400, tx));
        assert_eq!(moved, Ok(100));
        assert_eq!(player_of(&player).slots[&9], stack("test:cell", 1, 1000));
        assert_eq!(main_hand(&player), Some(stack("test:cell", 1, 600)));
        assert_eq!(player_of(&player).given, [stack("test:cell", 1, 500)]);
        // Aborted, a move leaves both as they were.
        let host: &mut dyn ServerHost = &mut player;
        let aborted = host.transaction(|host, tx| {
            let moved = move_energy(host, &other, &hand, 400, tx);
            Err::<(), _>(moved)
        });
        assert_eq!(aborted, Err(Ok(400)));
        assert_eq!(player_of(&player).slots[&9], stack("test:cell", 1, 1000));
        assert_eq!(main_hand(&player), Some(stack("test:cell", 1, 600)));
        // An item no mod answers for has no energy.
        set_hand(&mut player, stick());
        let host: &mut dyn ServerHost = &mut player;
        assert!(HAND.capability(host, &ITEM_ENERGY).is_none());
    }

    #[test]
    pub(crate) fn a_component_battery_charges_one_of_its_stack_and_an_abort_leaves_it() {
        game_thread();
        let energy = crate::ComponentId::new("test:energy").unwrap();
        let battery = crate::ItemComponentEnergy::new(HAND, energy.clone(), 1000, 250, 1000);
        let mut player = player_world(Some(stack("test:cell", 3, 0)));
        let host: &mut dyn ServerHost = &mut player;
        let aborted = host.transaction(|host, tx| Err::<(), _>(battery.insert(host, tx, 300)));
        assert_eq!(aborted, Err(250));
        assert_eq!(main_hand(&player), Some(stack("test:cell", 3, 0)));
        assert!(player_of(&player).given.is_empty());
        // The limit moves 250 of 300: one of the three is charged and splits off.
        let host: &mut dyn ServerHost = &mut player;
        let went = host.transaction(|host, tx| Ok::<_, ()>(battery.insert(host, tx, 300)));
        assert_eq!(went, Ok(250));
        assert_eq!(main_hand(&player), Some(stack("test:cell", 2, 0)));
        let given = &player_of(&player).given;
        assert_eq!(given.len(), 1);
        assert_eq!(given[0].count(), 1);
        let charged = ItemResource::of(&given[0]).component(&energy);
        assert_eq!(charged, Some(Tag::Int(250)));
    }

    #[test]
    pub(crate) fn a_component_canteen_fills_one_of_its_stack_and_an_abort_leaves_it() {
        game_thread();
        let content = crate::ComponentId::new("test:fluid").unwrap();
        let canteen = crate::ItemComponentFluidTank::new(HAND, content.clone(), 4000);
        let water = crate::FluidResource::of(&crate::FluidStack::new(
            mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
            1,
        ));
        let mut player = player_world(Some(stack("test:case", 3, 0)));
        let host: &mut dyn ServerHost = &mut player;
        let aborted =
            host.transaction(|host, tx| Err::<(), _>(canteen.insert(host, tx, 0, &water, 1000)));
        assert_eq!(aborted, Err(1000));
        assert_eq!(main_hand(&player), Some(stack("test:case", 3, 0)));
        assert!(player_of(&player).given.is_empty());
        let host: &mut dyn ServerHost = &mut player;
        let went =
            host.transaction(|host, tx| Ok::<_, ()>(canteen.insert(host, tx, 0, &water, 5000)));
        assert_eq!(went, Ok(4000));
        assert_eq!(main_hand(&player), Some(stack("test:case", 2, 0)));
        let given = &player_of(&player).given;
        assert_eq!(given.len(), 1);
        let mut filled = crate::Compound::new();
        filled.insert("id".into(), Tag::String("minecraft:water".into()));
        filled.insert("amount".into(), Tag::Int(4000));
        let held = ItemResource::of(&given[0]).component(&content);
        assert_eq!(held, Some(Tag::Compound(filled)));
    }

    #[test]
    pub(crate) fn a_canteen_holding_an_unknown_fluid_refuses_an_insert_and_keeps_it() {
        game_thread();
        let content = crate::ComponentId::new("test:fluid").unwrap();
        let canteen = crate::ItemComponentFluidTank::new(HAND, content, 4000);
        let water = crate::FluidResource::of(&crate::FluidStack::new(
            mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
            1,
        ));
        let mut unknown = crate::Compound::new();
        unknown.insert("id".into(), Tag::String("test:unknown_fluid".into()));
        unknown.insert("amount".into(), Tag::Int(500));
        let mut player = player_world(Some(stack("test:case", 1, 0)));
        player
            .registries
            .item_component("test:case", "test:fluid", Tag::Compound(unknown));
        let host: &mut dyn ServerHost = &mut player;
        let went =
            host.transaction(|host, tx| Ok::<_, ()>(canteen.insert(host, tx, 0, &water, 1000)));
        assert_eq!(went, Ok(0));
        assert_eq!(main_hand(&player), Some(stack("test:case", 1, 0)));
        assert!(player_of(&player).given.is_empty());
    }

    #[test]
    pub(crate) fn a_zero_extract_from_a_canteen_changes_nothing() {
        game_thread();
        let content = crate::ComponentId::new("test:fluid").unwrap();
        let canteen = crate::ItemComponentFluidTank::new(HAND, content, 4000);
        let water = crate::FluidResource::of(&crate::FluidStack::new(
            mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
            1,
        ));
        let mut full = crate::Compound::new();
        full.insert("id".into(), Tag::String("minecraft:water".into()));
        full.insert("amount".into(), Tag::Int(1000));
        let mut player = player_world(Some(stack("test:case", 2, 0)));
        player
            .registries
            .item_component("test:case", "test:fluid", Tag::Compound(full));
        let host: &mut dyn ServerHost = &mut player;
        let came =
            host.transaction(|host, tx| Ok::<_, ()>(canteen.extract(host, tx, 0, &water, 0)));
        assert_eq!(came, Ok(0));
        assert_eq!(main_hand(&player), Some(stack("test:case", 2, 0)));
        assert!(player_of(&player).given.is_empty());
    }

    #[test]
    pub(crate) fn a_stack_inside_an_item_and_a_mods_place_are_reached_through_their_handlers() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let mut player = player_world(Some(stack("test:bag", 1, 1)));
        player_mut(&mut player)
            .slots
            .insert(20 + 4, stack("test:pouch", 1, 2));
        let host: &mut dyn ServerHost = &mut player;
        // The empty pouch in the bag in the hand takes 3 sticks: the pouch is rewritten in the
        // bag, and the bag in the hand.
        let in_bag = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 0,
        };
        assert_eq!(fill(host, &in_bag, stick(), 3), 3);
        assert_eq!(in_bag.stack(host), Some(stack("test:pouch", 1, 3)));
        assert_eq!(HAND.stack(host), Some(stack("test:bag", 1, 4)));
        // A charm slot is a mod's place, read and written by its provider.
        let charm = ItemPlace::Mod {
            provider: "test:charms",
            key: 4,
        };
        assert_eq!(fill(host, &charm, stick(), 1), 1);
        assert_eq!(charm.stack(host), Some(stack("test:pouch", 1, 3)));
        let unknown = ItemPlace::Mod {
            provider: "test:none",
            key: 4,
        };
        assert!(unknown.stack(host).is_none());
        assert_eq!(insert(host, &unknown, stick()), 0);
    }

    #[cfg(test)]
    /// `place.insert` of `stack` in a transaction of its own: how much went.
    pub(crate) fn insert(host: &mut dyn ServerHost, place: &ItemPlace, stack: ItemStack) -> u32 {
        let resource = ItemResource::of(&stack);
        let went = host
            .transaction(|host, tx| Ok::<_, ()>(place.insert(host, tx, &resource, stack.count())));
        went.unwrap_or(0)
    }

    #[cfg(test)]
    /// `place.exchange` of `stack.count()` of what is there for `stack`, in a transaction of its
    /// own: how many.
    pub(crate) fn exchange(host: &mut dyn ServerHost, place: &ItemPlace, stack: ItemStack) -> u32 {
        let resource = ItemResource::of(&stack);
        let swapped = host.transaction(|host, tx| {
            Ok::<_, ()>(place.exchange(host, tx, &resource, stack.count()))
        });
        swapped.unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) const FROM: [i32; 3] = [3, 64, 0];
    #[cfg(test)]
    pub(crate) const INTO: [i32; 3] = [4, 64, 0];
    #[cfg(test)]
    /// A target that answers one more than it was given, and keeps nothing.
    pub(crate) struct TakesMore;
    #[cfg(test)]
    impl ResourceHandler<ItemResource> for TakesMore {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            None
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            0
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            64
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            n: u32,
        ) -> u32 {
            n + 1
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            _: u32,
        ) -> u32 {
            0
        }
    }

    #[test]
    pub(crate) fn a_move_takes_what_the_filter_accepts_up_to_max_and_leaves_what_the_target_refuses()
     {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let sticks = |n| Some(stack("minecraft:stick", n, 0));
        let apples = Some(stack("minecraft:apple", 3, 0));
        let mut world = two_chests([sticks(5), apples, sticks(4)]);
        let only_sticks = |r: &ItemResource| r.item() == fixture_item_key("minecraft:stick");
        let moved = |world: &mut FakeHost, max| {
            let host: &mut dyn ServerHost = world;
            let from = here().capability(host, &ITEMS, FROM, None).unwrap();
            let to = here().capability(host, &ITEMS, INTO, None).unwrap();
            host.transaction(|host, tx| move_resources(host, &from, &to, only_sticks, max, tx))
                .unwrap()
        };
        // Up to 7 sticks: all of the first slot's, two of the third's; the apples stay.
        assert_eq!(moved(&mut world, 7), 7);
        assert_eq!(chest(&mut world, FROM).count("minecraft:stick"), 2);
        assert_eq!(chest(&mut world, INTO).count("minecraft:stick"), 7);
        assert_eq!(chest(&mut world, FROM).count("minecraft:apple"), 3);
        // A target with room for one more of the two (its other slots hold apples): that one
        // moves, the other stays put.
        chest(&mut world, INTO).limit = 8;
        chest(&mut world, INTO).slots[1] = Some(stack("minecraft:apple", 1, 0));
        chest(&mut world, INTO).slots[2] = Some(stack("minecraft:apple", 1, 0));
        assert_eq!(moved(&mut world, 64), 1);
        assert_eq!(chest(&mut world, FROM).count("minecraft:stick"), 1);
        assert_eq!(chest(&mut world, INTO).count("minecraft:stick"), 8);
        assert_eq!(moved(&mut world, 64), 0);
        assert_eq!(chest(&mut world, FROM).count("minecraft:stick"), 1);
    }

    #[test]
    pub(crate) fn a_move_into_a_handler_answering_more_than_it_was_given_is_undone() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let mut world = two_chests([Some(stack("minecraft:stick", 5, 0)), None, None]);
        let host: &mut dyn ServerHost = &mut world;
        let from = here().capability(host, &ITEMS, FROM, None).unwrap();
        let liar: Capability<dyn ResourceHandler<ItemResource>> =
            mod_vocab::__private::parts_installation::test_capability(
                Box::new(TakesMore),
                Some("doubler"),
            );
        let moved =
            host.transaction(|host, tx| move_resources(host, &from, &liar, |_| true, 64, tx));
        let owner = "doubler".to_owned();
        assert_eq!(
            moved,
            Err(mod_vocab::__private::parts_installation::overanswered(
                owner
            ))
        );
        assert_eq!(
            chest(&mut world, FROM).slots[0],
            Some(stack("minecraft:stick", 5, 0))
        );
        assert!(crate::parts::warned_about("doubler"));
    }

    #[test]
    pub(crate) fn a_changed_component_is_an_exchange_of_one_that_splits_it_off_and_an_abort_puts_it_back()
     {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let sword = |count, damage| stack("minecraft:iron_sword", count, damage);
        let mut shelf_ = shelf([Some(sword(2, 0)), None, None]);
        let host: &mut dyn ServerHost = &mut shelf_;
        let place = shelf_slot(0);
        let plain = place.resource(host).unwrap();
        let damage = ComponentId::new("minecraft:damage").unwrap();
        assert_eq!(plain.component(&damage), None);
        let worn = plain.with_component(host, &damage, Tag::Int(5)).unwrap();
        assert_eq!(worn.component(&damage), Some(Tag::Int(5)));
        // Aborted, nothing changed; committed, one sword of the two is worn and splits off.
        let aborted = host.transaction(|host, tx| {
            assert_eq!(place.exchange(host, tx, &worn, 1), 1);
            Err::<(), ()>(())
        });
        assert!(aborted.is_err());
        assert_eq!(shelf_.slots, [Some(sword(2, 0)), None, None]);
        let host: &mut dyn ServerHost = &mut shelf_;
        assert_eq!(exchange(host, &place, sword(1, 5)), 1);
        assert_eq!(shelf_.slots, [Some(sword(1, 0)), Some(sword(1, 5)), None]);
    }

    #[test]
    pub(crate) fn a_removed_default_component_is_recorded_in_the_patch_and_a_default_set_one_is_dropped()
     {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let shelf_ = shelf([None, None, None]);
        let host: &dyn Host = &shelf_;
        let sword = ItemResource::of(&stack("minecraft:iron_sword", 1, 0));
        let patch = |resource: &ItemResource| {
            let stack = resource.to_stack(1).unwrap();
            let Some(patch) = stack.components() else {
                return Vec::new();
            };
            patch
                .as_nbt()
                .keys()
                .map(|key| key.to_string_lossy())
                .collect::<Vec<_>>()
        };
        // An iron sword has `max_damage` by default: removing it is a removal in its patch,
        // so the sword is no longer the plain one.
        let max_damage = ComponentId::new("minecraft:max_damage").unwrap();
        let unbreakable = sword.without_component(host, &max_damage);
        assert_eq!(patch(&unbreakable), ["!minecraft:max_damage"]);
        assert_ne!(unbreakable, sword);
        assert_eq!(unbreakable.component(&max_damage), None);
        // Set again, the removal goes and the value is the patch's own.
        let set = unbreakable
            .with_component(host, &max_damage, Tag::Int(10))
            .unwrap();
        assert_eq!(patch(&set), ["minecraft:max_damage"]);
        assert_eq!(set.component(&max_damage), Some(Tag::Int(10)));
        // A component the item hasn't by default only loses the stack's own value.
        let data = ComponentId::new("minecraft:custom_data").unwrap();
        let tagged = sword
            .with_component(host, &data, Tag::Compound(Compound::new()))
            .unwrap();
        assert_ne!(tagged, sword);
        assert_eq!(tagged.without_component(host, &data), sword);
        // Set to the item's default, a component keeps no value of the stack's own, as vanilla's
        // `set` drops it: the stack is the plain item, whatever NBT type the mod wrote.
        assert_eq!(
            set.with_component(host, &max_damage, Tag::Int(250))
                .unwrap(),
            sword
        );
        assert_eq!(
            unbreakable
                .with_component(host, &max_damage, Tag::Short(250))
                .unwrap(),
            sword
        );
        let enchantable = ComponentId::new("minecraft:enchantable").unwrap();
        let value = |v| Tag::Compound([("value".into(), v)].into_iter().collect());
        assert_eq!(
            sword
                .with_component(host, &enchantable, value(Tag::Byte(14)))
                .unwrap(),
            sword
        );
        assert_ne!(
            sword
                .with_component(host, &enchantable, value(Tag::Int(15)))
                .unwrap(),
            sword
        );
    }

    #[test]
    pub(crate) fn a_component_set_keeps_a_change_its_codec_reads_and_raw_nbt_keeps_its_types() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        // A stick's report: `custom_data` `{x: 1}`, a huge `max_damage` and a mod's `test:long`.
        let mut report = FakeHost::new();
        let one = Tag::Compound([("x".into(), Tag::Int(1))].into_iter().collect());
        report
            .registries
            .item_component("minecraft:stick", "minecraft:custom_data", one)
            .item_component(
                "minecraft:stick",
                "minecraft:max_damage",
                Tag::Long(1 << 53),
            )
            .item_component("minecraft:stick", "test:long", Tag::Long(1 << 53));
        let stick = ItemResource::of(&stick());
        let set = |id, value: Tag| {
            let id = ComponentId::new(id).unwrap();
            stick
                .with_component(&report, &id, value)
                .unwrap()
                .component(&id)
        };
        // `custom_data` is kept as the NBT given (`CustomData.equals`): a byte where the report
        // has an int is a change, while the same int is the default.
        let x = |v| Tag::Compound([("x".into(), v)].into_iter().collect());
        assert_eq!(
            set("minecraft:custom_data", x(Tag::Byte(1))),
            Some(x(Tag::Byte(1)))
        );
        assert_eq!(set("minecraft:custom_data", x(Tag::Int(1))), None);
        // Integers compare exactly, past where an `f64` keeps them apart.
        let past = Tag::Long((1 << 53) + 1);
        assert_eq!(
            set("minecraft:max_damage", past.clone()),
            Some(past.clone())
        );
        assert_eq!(set("test:long", past.clone()), Some(past));
        assert_eq!(set("minecraft:max_damage", Tag::Int(0)), Some(Tag::Int(0)));
        // A vanilla component's codec reads any integer width: the default's value is it.
        assert_eq!(set("minecraft:max_damage", Tag::Long(1 << 53)), None);
        // A mod's component, whose codec the game doesn't know, is equal type for type.
        let double = Tag::Double(((1u64 << 53) as f64).to_bits());
        assert_eq!(set("test:long", double.clone()), Some(double));
    }

    #[test]
    pub(crate) fn a_component_id_is_a_namespaced_identifier() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        for id in ["minecraft:damage", "mymod:charge/level"] {
            assert_eq!(
                ComponentId::new(id).map(|id| id.as_str().to_owned()),
                Some(id.into())
            );
        }
        for id in [
            "damage",
            "!minecraft:food",
            "not a namespaced id!",
            "Mymod:charge",
            ":path",
            "mymod:",
        ] {
            assert!(ComponentId::new(id).is_none(), "{id}");
        }
    }

    #[test]
    pub(crate) fn a_write_that_cant_go_through_changes_nothing_and_no_item_is_made_or_lost() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let bread = |count| stack("minecraft:bread", count, 0);
        let stone = || Some(stack("minecraft:stone", 64, 0));
        // An output slot, as a furnace's, beside full ones: its iron comes out, but the bread
        // can go nowhere, so the iron goes back.
        let iron = || Some(stack("minecraft:iron_ingot", 1, 0));
        let mut shelf_ = shelf([iron(), stone(), stone()]);
        shelf_.output = Some(0);
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread(1)), 0);
        assert_eq!(shelf_.slots, [iron(), stone(), stone()]);
        // One of ten apples becomes the bread, which goes into the next slot.
        let mut shelf_ = shelf([Some(stack("minecraft:apple", 10, 0)), None, None]);
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread(1)), 1);
        assert_eq!(
            shelf_.slots,
            [Some(stack("minecraft:apple", 9, 0)), Some(bread(1)), None]
        );
        assert_eq!(shelf_.items(), 10);

        // Two empty pouches on a full shelf: a filled one has nowhere to go, so neither changes
        // and the sticks stay out.
        let mut shelf = shelf([Some(stack("test:pouch", 2, 0)), stone(), stone()]);
        let before = shelf.items();
        assert_eq!(fill(&mut shelf, &shelf_slot(0), stick(), 3), 0);
        assert_eq!(shelf.items(), before);
        assert_eq!(shelf.slots[0], Some(stack("test:pouch", 2, 0)));
        // With a slot free, one pouch is filled there.
        shelf.slots[2] = None;
        let before = shelf.items();
        assert_eq!(fill(&mut shelf, &shelf_slot(0), stick(), 3), 3);
        assert_eq!(
            shelf.slots,
            [
                Some(stack("test:pouch", 1, 0)),
                stone(),
                Some(stack("test:pouch", 1, 3))
            ]
        );
        assert_eq!(shelf.items(), before + 3);

        // Two pouches in a charm slot, which can't split: nothing changes. So with a bag in
        // that slot holding the pouch filled.
        let mut player = player_world(None);
        player_mut(&mut player)
            .slots
            .insert(20 + 1, stack("test:pouch", 2, 0));
        player_mut(&mut player)
            .slots
            .insert(20 + 2, stack("test:bag", 2, 1));
        let before = held_items(&player);
        for charm in [
            ItemPlace::Mod {
                provider: "test:charms",
                key: 1,
            },
            ItemPlace::Nested {
                parent: Box::new(ItemPlace::Mod {
                    provider: "test:charms",
                    key: 2,
                }),
                slot: 0,
            },
        ] {
            assert_eq!(fill(&mut player, &charm, stick(), 3), 0);
        }
        assert_eq!(held_items(&player), before);
        assert_eq!(player_of(&player).slots[&21], stack("test:pouch", 2, 0));
        assert_eq!(player_of(&player).slots[&22], stack("test:bag", 2, 1));
    }

    #[test]
    pub(crate) fn a_truthful_handler_moving_one_per_call_is_moved_in_passes_and_never_named() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        // A slot that gives one item per extract and takes one per insert: the 3 apples come
        // out, and the 3 bread go in, one pass each.
        let mut shelf_ = shelf([Some(stack("minecraft:apple", 3, 0)), None, None]);
        shelf_.one_per_insert = true;
        let bread = stack("minecraft:bread", 3, 0);
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread.clone()), 3);
        assert_eq!(shelf_.slots, [Some(bread), None, None]);
        assert!(!crate::parts::warned_about("the game"));

        // A transfer amount is not a resident stack: 128 offered, one accepted,
        // and a 127-item remainder are valid even though this slot's maximum is 64.
        let resource = ItemResource::of(&stack("minecraft:bread", 1, 0));
        let inspected = shelf_.transaction(|host, tx| {
            let remainder = backend_mut_of(host)
                .unwrap()
                .container_insert(on(SHELF, None), 1, &resource, 128, tx)
                .unwrap()
                .unwrap();
            assert_eq!(remainder.count(), 127);
            Err::<(), ()>(())
        });
        assert!(inspected.is_err());
        assert!(shelf_.slots[1].is_none());
        let handler = here().capability(&mut shelf_, &ITEMS, SHELF, None).unwrap();
        assert_eq!(handler.capacity(&mut shelf_, 1, &resource), 64);
        let moved =
            shelf_.transaction(|host, tx| Ok::<_, ()>(handler.insert(host, tx, 1, &resource, 128)));
        assert_eq!(moved, Ok(1));
        assert_eq!(shelf_.slots[1], Some(stack("minecraft:bread", 1, 0)));
    }

    #[test]
    pub(crate) fn an_item_handler_answering_more_than_it_was_asked_or_offered_moves_nothing_and_is_named()
     {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let apples = |count| stack("minecraft:apple", count, 0);
        let bread = stack("minecraft:bread", 1, 0);
        let mut shelf_ = shelf([Some(apples(10)), None, None]);
        shelf_.whole_extract = true;
        shelf_.overstates = true;
        // Asked for one apple to swap for the bread, it takes and answers 6: the move is undone,
        // and the shelf keeps its 10.
        assert_eq!(exchange(&mut shelf_, &shelf_slot(0), bread), 0);
        assert_eq!(shelf_.slots, [Some(apples(10)), None, None]);
        // Offered 2 apples, it keeps none and answers 7 taken: undone, and the slot stays empty.
        assert_eq!(insert(&mut shelf_, &shelf_slot(1), apples(2)), 0);
        assert_eq!(shelf_.slots, [Some(apples(10)), None, None]);
        assert!(crate::parts::warned_about("doubler"));
    }

    #[test]
    pub(crate) fn a_change_inside_one_of_two_bags_splits_that_bag_off_whole() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        // Two bags in the hand, each holding an empty pouch.
        let mut player = player_world(Some(stack("test:bag", 2, 1)));
        let before = held_items(&player);
        let in_bag = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 0,
        };
        assert_eq!(fill(&mut player, &in_bag, stick(), 3), 3);
        // One bag leaves the hand holding the filled pouch; the other stays as it was, and no
        // pouch is loose.
        assert_eq!(main_hand(&player), Some(stack("test:bag", 1, 1)));
        assert_eq!(player_of(&player).given, [stack("test:bag", 1, 4)]);
        assert_eq!(held_items(&player), before + 3);
    }

    #[test]
    pub(crate) fn an_aborted_write_inside_a_nested_item_puts_every_level_back() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        // The pouch in the bag in the hand is filled in a transaction that then aborts: the
        // bag and its pouch are as they were, and nothing was given.
        let mut player = player_world(Some(stack("test:bag", 2, 1)));
        let in_bag = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 0,
        };
        let host: &mut dyn ServerHost = &mut player;
        let pouch = in_bag.capability(host, &ITEM_ITEMS).unwrap();
        let sticks = ItemResource::of(&stick());
        let aborted: Result<(), u32> =
            host.transaction(|host, tx| Err(pouch.insert(host, tx, 0, &sticks, 3)));
        assert_eq!(aborted, Err(3));
        assert_eq!(main_hand(&player), Some(stack("test:bag", 2, 1)));
        assert!(player_of(&player).given.is_empty());
    }

    #[test]
    pub(crate) fn an_aborted_inner_write_of_a_handler_inside_an_item_changes_nothing() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        // A case of one diamond and one stick in the hand: its stick is taken and two put in;
        // on the way it tries emptying its diamond slot in a transaction it aborts. The
        // diamond stays, and no item is made or lost.
        let mut player = player_world(Some(stack("test:case", 1, 101)));
        let sticks = ItemPlace::Nested {
            parent: Box::new(HAND),
            slot: 1,
        };
        let host: &mut dyn ServerHost = &mut player;
        let resource = ItemResource::of(&stick());
        host.transaction(|host, tx| {
            assert_eq!(sticks.extract(host, tx, &resource, 1), 1);
            assert_eq!(sticks.insert(host, tx, &resource, 2), 2);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(main_hand(&player), Some(stack("test:case", 1, 102)));
        assert!(player_of(&player).given.is_empty());
    }

    pub(crate) mod deep_places {
        use super::*;

        /// An outer bag at `.0` holding one `test:bag`, kept in its damage as `Bag` keeps its
        /// pouch: 0 when empty.
        pub(crate) struct Wrap(pub ItemPlace);
        impl ResourceHandler<ItemResource> for Wrap {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                1
            }
            fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                let n = self.0.stack(host)?.damage();
                (n > 0).then(|| ItemResource::of(&stack("test:bag", 1, n - 1)))
            }
            fn amount(&self, host: &mut dyn ServerHost, i: usize) -> u32 {
                u32::from(self.resource(host, i).is_some())
            }
            fn capacity(&self, _: &mut dyn ServerHost, _: usize, r: &ItemResource) -> u32 {
                u32::from(r.item() == fixture_item_key("test:bag"))
            }
            fn insert(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                _: usize,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                if n == 0
                    || r.item() != fixture_item_key("test:bag")
                    || self.0.stack(host).unwrap().damage() != 0
                {
                    return 0;
                }
                let new =
                    ItemResource::of(&stack("test:outer", 1, r.to_stack(1).unwrap().damage() + 1));
                self.0.exchange(host, tx, &new, 1)
            }
            fn extract(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                i: usize,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                if n == 0 || self.resource(host, i).as_ref() != Some(r) {
                    return 0;
                }
                let new = ItemResource::of(&stack("test:outer", 1, 0));
                self.0.exchange(host, tx, &new, 1)
            }
        }

        /// A mod place that is the player's hand, and answers one more than it moved.
        pub(crate) struct LyingPlace;
        impl PlaceProvider for LyingPlace {
            fn stack(&self, host: &mut dyn ServerHost, _: u64) -> Option<ItemStack> {
                HAND.stack(host)
            }
            fn insert(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                _: u64,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                HAND.insert(host, tx, r, n) + 1
            }
            fn extract(
                &self,
                host: &mut dyn ServerHost,
                tx: &Transaction,
                _: u64,
                r: &ItemResource,
                n: u32,
            ) -> u32 {
                HAND.extract(host, tx, r, n) + 1
            }
        }

        #[test]
        fn a_pouch_in_a_bag_in_a_bag_is_put_back_by_an_abort_at_any_depth() {
            game_thread();
            let in_outer = ItemPlace::Nested {
                parent: Box::new(HAND),
                slot: 0,
            };
            let in_bag = ItemPlace::Nested {
                parent: Box::new(in_outer.clone()),
                slot: 0,
            };
            for abort_depth in 0..3 {
                let initial = stack("test:outer", 1, 2);
                let mut player = player_world(Some(initial.clone()));
                let host: &mut dyn ServerHost = &mut player;
                let r = ItemResource::of(&stick());
                let result = host.transaction(|host, _| {
                    let middle = host.transaction(|host, _| {
                        let inner = host.transaction(|host, tx| {
                            let handler = in_bag.capability(host, &ITEM_ITEMS).unwrap();
                            assert_eq!(
                                handler.insert(host, tx, 0, &r, 3),
                                3,
                                "depth {abort_depth}"
                            );
                            if abort_depth == 2 { Err(()) } else { Ok(()) }
                        });
                        if abort_depth == 1 {
                            Err(())
                        } else {
                            inner.or(Ok(()))
                        }
                    });
                    if abort_depth == 0 {
                        Err(())
                    } else {
                        middle.or(Ok(()))
                    }
                });
                assert_eq!(result.is_err(), abort_depth == 0);
                assert_eq!(main_hand(&player), Some(initial), "depth {abort_depth}");
                assert!(player_of(&player).given.is_empty());
                // Committed, the outer bag is rewritten whole.
                let host: &mut dyn ServerHost = &mut player;
                assert_eq!(fill(host, &in_bag, stick(), 3), 3);
                assert_eq!(main_hand(&player), Some(stack("test:outer", 1, 5)));
                assert!(player_of(&player).given.is_empty());
            }
        }

        #[test]
        fn a_write_three_levels_down_in_one_of_two_stacked_bags_splits_that_one_off() {
            game_thread();
            let in_outer = ItemPlace::Nested {
                parent: Box::new(HAND),
                slot: 0,
            };
            let in_bag = ItemPlace::Nested {
                parent: Box::new(in_outer),
                slot: 0,
            };
            let mut player = player_world(Some(stack("test:outer", 2, 2)));
            assert_eq!(fill(&mut player, &in_bag, stick(), 3), 3);
            assert_eq!(main_hand(&player), Some(stack("test:outer", 1, 2)));
            assert_eq!(player_of(&player).given, [stack("test:outer", 1, 5)]);
        }

        #[test]
        fn an_index_less_move_goes_on_to_the_next_index() {
            game_thread();
            let mut s = shelf([
                Some(stack("minecraft:stick", 63, 0)),
                Some(stack("minecraft:stick", 63, 0)),
                None,
            ]);
            s.whole_extract = true;
            let host: &mut dyn ServerHost = &mut s;
            let h = here().capability(host, &ITEMS, SHELF, None).unwrap();
            let r = ItemResource::of(&stick());
            host.transaction(|host, tx| {
                assert_eq!(h.insert_any(host, tx, &r, 3), 3);
                assert_eq!(h.extract_any(host, tx, &r, 129), 129);
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(s.slots, [None, None, None]);
        }

        #[test]
        fn a_mod_place_answering_more_than_it_was_asked_has_its_write_undone() {
            game_thread();
            let mut p = player_world(Some(stack("minecraft:stick", 2, 0)));
            let at = ItemPlace::Mod {
                provider: "test:lying-place",
                key: 0,
            };
            let host: &mut dyn ServerHost = &mut p;
            let r = ItemResource::of(&stick());
            host.transaction(|host, tx| {
                assert_eq!(at.insert(host, tx, &r, 1), 0);
                assert_eq!(at.extract(host, tx, &r, 1), 0);
                assert_eq!(HAND.stack(host), Some(stack("minecraft:stick", 2, 0)));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(player_of(&p).given.is_empty());
            assert!(crate::parts::warned_about("doubler"));
        }
    }

    #[test]
    pub(crate) fn a_place_provider_given_twice_is_refused() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        let mut r = super::PartRegistrar::new("first");
        (&mut r as &mut dyn Registrar).place_provider("test:charms", Arc::new(Charms));
        r.mod_id = "second";
        r.mod_index = 1;
        (&mut r as &mut dyn Registrar).place_provider("test:charms", Arc::new(Charms));
        (&mut r as &mut dyn Registrar).place_provider("charms", Arc::new(Charms));
        let invalid = r.block_entity_type_key("minecraft:");
        (&mut r as &mut dyn Registrar)
            .block_entity_capability(&ITEMS, &invalid, |_, at| Some(Box::new(Pipe(at))));
        assert_eq!(r.refused.len(), 3, "{:?}", r.refused);
        assert!(
            r.refused[0]
                .contains("item places test:charms are provided twice, by first and second")
        );
        assert!(r.refused[1].contains("not a namespaced name"));
        assert!(r.refused[2].contains("invalid registry key \"minecraft:\""));
    }

    #[test]
    pub(crate) fn disabled_place_providers_refuse_reads_and_writes_and_resume() {
        install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        game_thread();
        let mut player = player_world(None);
        let charm = ItemPlace::Mod {
            provider: "test:charms",
            key: 4,
        };
        assert_eq!(
            insert(&mut player, &charm, stack("minecraft:stick", 3, 0)),
            3
        );
        assert!(crate::__private::set_mod_enabled("chemist", false));
        assert_eq!(
            charm.amount(&mut player),
            3,
            "another mod's flag is independent"
        );
        assert!(crate::__private::set_mod_enabled("doubler", false));
        assert!(charm.stack(&mut player).is_none());
        assert_eq!(insert(&mut player, &charm, stick()), 0);
        let resource = ItemResource::of(&stick());
        assert_eq!(
            player.transaction(|host, tx| Ok::<_, ()>(charm.extract(host, tx, &resource, 1))),
            Ok(0)
        );
        assert_eq!(
            player_of(&player).slots[&24].count(),
            3,
            "disabled writes leave the place unchanged"
        );
        assert!(crate::__private::set_mod_enabled("doubler", true));
        assert_eq!(charm.amount(&mut player), 3);
        assert_eq!(insert(&mut player, &charm, stick()), 1);
        assert_eq!(
            player.transaction(|host, tx| Ok::<_, ()>(charm.extract(host, tx, &resource, 1))),
            Ok(1)
        );
        assert_eq!(charm.amount(&mut player), 3);
    }

    #[cfg(test)]
    /// Player 1 holding one stack in their hand, which has game slots (`item_container`) when
    /// its damage is 1.
    pub(crate) struct HeldItem(pub(crate) ItemStack);
    #[cfg(test)]
    impl mod_api::prelude::ItemsBackend for HeldItem {
        fn item_bundle_slots(&self, _: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
            None
        }
        fn item_container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
            (stack.damage() == 1).then(|| vec![None; 27])
        }
    }
    #[cfg(test)]
    impl Backend for HeldItem {}
    #[cfg(test)]
    mod_api::impl_backend_areas!(HeldItem, except(items));
    #[cfg(test)]
    impl mod_api::prelude::ItemsBackendMut for HeldItem {
        fn game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
            (*place == HAND).then(|| self.0.clone())
        }
    }
    #[cfg(test)]
    impl BackendMut for HeldItem {}
    #[cfg(test)]
    mod_api::impl_backend_mut_areas!(HeldItem, except(items));
    #[cfg(test)]
    impl Host for HeldItem {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_ref(self))
        }
    }
    #[cfg(test)]
    mod_api::impl_host_areas!(HeldItem);
    #[cfg(test)]
    impl ServerHost for HeldItem {
        fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
            Some(mod_vocab::__private::parts_installation::backend_mut_ref(
                self,
            ))
        }
    }
    #[cfg(test)]
    mod_api::impl_server_host_areas!(HeldItem);
}

impl ItemsRegistrar for PartRegistrar {
    fn item_key(&mut self, id: &str) -> crate::ItemKey {
        match self
            .registry_keys
            .borrow_mut()
            .items
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::item_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::item_key(u32::MAX)
            }
        }
    }

    fn attach_item_part(
        &mut self,
        item: crate::ItemKey,
        kind: &'static str,
        part: Arc<dyn ItemPart>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .items
            .id(mod_vocab::__private::parts_installation::item_key_slot(
                item,
            ))
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {}: invalid item key slot {}",
                self.mod_id,
                mod_vocab::__private::parts_installation::item_key_slot(item)
            ));
            return;
        };
        self.attach(|p| &mut p.items, kind, &id, part);
    }

    fn attach_item_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn ItemPart>) {
        let part = self.tagged(kind, tag, part);
        self.tags.items.extend(part);
    }

    fn provide_item_capability(
        &mut self,
        name: &'static str,
        item: &str,
        before_game: bool,
        provider: ItemProviderOf,
    ) {
        self.capability(
            |p| &mut p.item_capabilities,
            None,
            name,
            ProviderTarget::Id(item),
            before_game,
            {
                let handler =
                    mod_vocab::__private::parts_installation::item_provider_handler(&provider);
                (provider, handler)
            },
        );
    }

    /// Read and write the item places `ItemPlace::Mod { provider: name, .. }` with `provider`,
    /// as an accessory mod exposes its slots. One provider a name.
    fn place_provider(&mut self, name: &'static str, provider: Arc<dyn PlaceProvider>) {
        if !namespaced(name) {
            self.refused.push(format!(
                "{} provides item places {name:?}, which is not a namespaced name",
                self.mod_id
            ));
            return;
        }
        if let Some(other) = self.parts.place_providers.get(name) {
            self.refused.push(format!(
                "item places {name} are provided twice, by {} and {}",
                other.mod_id, self.mod_id
            ));
            return;
        }
        self.parts.place_providers.insert(
            name,
            PlaceProviderPart {
                mod_id: self.mod_id,
                mod_index: self.mod_index,
                provider,
            },
        );
    }

    fn register_tooltip_appender(
        &mut self,
        component: &str,
        placement: crate::TooltipPlacement,
        appender: Arc<dyn crate::TooltipAppender>,
    ) {
        let refusal = if !namespaced(component) {
            Some(format!(
                "{} registers a tooltip appender for {component:?}, which is not a namespaced data component id",
                self.mod_id
            ))
        } else {
            match &placement {
                crate::TooltipPlacement::Before(anchor)
                | crate::TooltipPlacement::After(anchor)
                    if !TOOLTIP_DETAILS.contains(&anchor.as_str()) =>
                {
                    Some(format!(
                        "{} anchors a tooltip appender for {component:?} to {anchor:?}, which is not a component detail Horizon lists",
                        self.mod_id
                    ))
                }
                _ => None,
            }
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.parts.tooltip_appenders.push(TooltipAppenderPart {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            component: component.into(),
            placement,
            appender,
        });
    }
}

/// The component details a tooltip lists, in the order the game lists them after the item's own
/// lines. `TooltipPlacement::Before` and `After` name one of these.
pub(crate) const TOOLTIP_DETAILS: &[&str] = &[
    "minecraft:tropical_fish/pattern",
    "minecraft:instrument",
    "minecraft:map_id",
    "minecraft:bees",
    "minecraft:container",
    "minecraft:banner_patterns",
    "minecraft:written_book_content",
    "minecraft:charged_projectiles",
    "minecraft:fireworks",
    "minecraft:potion_contents",
    "minecraft:jukebox_playable",
    "minecraft:trim",
    "minecraft:stored_enchantments",
    "minecraft:enchantments",
    "minecraft:dyed_color",
    "minecraft:lore",
    "minecraft:attribute_modifiers",
    "minecraft:unbreakable",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:suspicious_stew_effects",
    "minecraft:block_state",
    "minecraft:entity_data",
    "minecraft:block_entity_data",
];
