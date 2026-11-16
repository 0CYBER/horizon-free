//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl PartRegistrar {
    /// Adds `provider` for capability `name` and `id` to `table`. Refused for a capability name
    /// or an id with no namespace, and for a capability name another provider answers with
    /// another handler type (two mods each declaring their own key under one name, rather than
    /// sharing the one the defining mod's `[api]` crate publishes): asked by one type, the other's
    /// providers would never answer.
    /// `shares_names` is the table whose capability names are the same capabilities as
    /// `table`'s (a block's and a block entity type's, asked together at a position), so one
    /// name has one handler type across both.
    pub(crate) fn capability<P>(
        &mut self,
        table: fn(&mut Parts) -> &mut Capabilities<P>,
        shares_names: Option<fn(&Parts) -> &Capabilities<P>>,
        name: &'static str,
        target: ProviderTarget<'_>,
        before_game: bool,
        (provider, handler): (P, impl Into<Arc<str>>),
    ) {
        let handler = handler.into();
        let me = self.mod_id;
        if !namespaced(name) {
            self.refused.push(format!(
                "{me} answered capability {name:?}, which is not a namespaced name"
            ));
            return;
        }
        let block_entity_id;
        let id = match target {
            ProviderTarget::BlockEntity(kind) => {
                block_entity_id = self
                    .registry_keys
                    .borrow()
                    .block_entity_types
                    .id(mod_vocab::__private::parts_installation::block_entity_type_key_slot(*kind))
                    .map(str::to_owned);
                let Some(id) = block_entity_id.as_deref() else {
                    return;
                };
                id
            }
            ProviderTarget::Id(id) => {
                if !namespaced(id) {
                    self.refused.push(format!(
                        "{me} answered {name} for {id:?}, which is not a namespaced id"
                    ));
                    return;
                }
                id
            }
        };
        let answered = |table: &Capabilities<P>| {
            let other = table.get(name)?.values().flatten().next()?;
            (other.type_name != handler).then(|| format!("{} by {}", other.type_name, other.mod_id))
        };
        let other = answered(table(&mut self.parts))
            .or_else(|| shares_names.and_then(|shares| answered(shares(&self.parts))));
        if let Some(other) = other {
            self.refused.push(format!(
                "capability {name} is answered with two handler types, {other} and {handler} by {me}",
            ));
            return;
        }
        if shares_names.is_some() && !self.note_block_capability(name, handler.clone()) {
            return;
        }
        let providers = table(&mut self.parts)
            .entry(name)
            .or_default()
            .entry(id.into())
            .or_default();
        // A position's lookup keeps its place in a block's and a block entity type's lists in
        // one cursor, which bounds each.
        if shares_names.is_some()
            && providers.len() >= mod_vocab::__private::parts_installation::max_providers_at()
        {
            self.refused.push(format!(
                "{me} answered capability {name} for {id}, which already has the most providers \
                 one block or block entity type may have ({})",
                mod_vocab::__private::parts_installation::max_providers_at()
            ));
            return;
        }
        self.registered += 1;
        let order = self.registered;
        // Those asked ahead of the game first, each kind in registration order.
        let at = if before_game {
            providers.partition_point(|part| part.before_game)
        } else {
            providers.len()
        };
        providers.insert(
            at,
            CapabilityPart {
                mod_id: me,
                mod_index: self.mod_index,
                type_name: handler,
                before_game,
                order,
                provider,
            },
        );
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::testing::{FakeContainer, FakeHost};

    #[cfg(test)]
    /// `test:ordered` block entities from x = 7 to 10, where the game has slots of its own
    /// from x = 8 on.
    pub(crate) fn ordered_world() -> FakeHost {
        let mut host = FakeHost::new();
        for x in 7..=10 {
            let ordered = host.level.block_entity([x, 64, 0]).of_type("test:ordered");
            if x >= 8 {
                ordered.with_items(27);
            }
        }
        host
    }

    #[cfg(test)]
    /// A pipe at (0, 64, 0), and east of it a vanilla chest of one slot that holds one item,
    /// answered by the game.
    pub(crate) fn pipe_world() -> FakeHost {
        let mut host = FakeHost::new();
        host.level.block_entity(PIPE).of_type("test:pipe");
        host.level.block_entity(CHEST).items = Some({
            let mut value = FakeContainer::new(0);
            value.slots = vec![None];
            value.limit = 1;
            value
        });
        host
    }

    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::parts::tests::*;
    use crate::*;

    /// A chemical, as Mekanism's: the test mods' own resource, which the game knows nothing of.
    #[derive(Clone, Debug, PartialEq)]
    // exhaustive: the game builds it with a literal; mods only read it
    // no wire: testing-feature fixture state, not a mod boundary value.
    pub struct Chemical(pub String);

    /// The test mods' own capability, as Mekanism's `Capabilities.CHEMICAL`: the game has no
    /// answer for it, so only their providers do.
    pub const CHEMICALS: CapabilityKey<dyn ResourceHandler<Chemical>> =
        CapabilityKey::new("test:chemical");

    /// A mod block entity's one tank of 1000 of a chemical, kept in its saved data (`chemical`
    /// and `amount`), whose writes the game's journal keeps for a transaction.
    // exhaustive: the game builds it with a literal; mods only read it
    // no wire: testing-feature fixture state, not a mod boundary value.
    pub struct ChemTank(pub BlockSide);
    impl ChemTank {
        fn entity(&self, host: &dyn Host) -> Option<BlockEntityRef> {
            host.block_entity(self.0.level, self.0.pos)
        }
        fn held(&self, host: &dyn Host) -> Option<(Chemical, u32)> {
            let data = self.entity(host)?.data(host)?;
            let Some(Tag::String(chemical)) = data.get(&"chemical".into()) else {
                return None;
            };
            let Some(&Tag::Int(amount)) = data.get(&"amount".into()) else {
                return None;
            };
            let amount = u32::try_from(amount).ok().filter(|&a| a > 0)?;
            Some((Chemical(chemical.to_string_lossy()), amount))
        }
        fn set(&self, host: &mut dyn ServerHost, chemical: &Chemical, amount: u32) {
            let Some(entity) = self.entity(host) else {
                return;
            };
            let mut data = Compound::new();
            data.insert("chemical".into(), Tag::String(chemical.0.as_str().into()));
            data.insert("amount".into(), Tag::Int(amount as i32));
            let _ = entity.merge_data(host, data);
        }
    }
    impl ResourceHandler<Chemical> for ChemTank {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<Chemical> {
            self.held(host).map(|(chemical, _)| chemical)
        }
        fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
            self.held(host).map_or(0, |(_, amount)| amount)
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &Chemical) -> u32 {
            1000
        }
        fn insert(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> u32 {
            let held = match self.held(host) {
                Some((other, _)) if other != *chemical => return 0,
                Some((_, held)) => held,
                None => 0,
            };
            let went = amount.min(1000 - held);
            if went > 0 {
                self.set(host, chemical, held + went);
            }
            went
        }
        fn extract(
            &self,
            host: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            chemical: &Chemical,
            amount: u32,
        ) -> u32 {
            let Some((held_chemical, held)) = self.held(host).filter(|(c, _)| c == chemical) else {
                return 0;
            };
            let came = amount.min(held);
            self.set(host, &held_chemical, held - came);
            came
        }
    }

    pub(crate) const ROBOT_ENERGY: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "energy",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    /// A fluid attachment type for the tests.
    pub const ROBOT_FLUID: AttachmentType<u32> = AttachmentType {
        namespace: "test",
        name: "fluid",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    /// A second test mod, sharing `CHEMICALS` and `Chemical` with the first, as a mod depending
    /// on another's `[api]` crate does: its vats answer the same capability.
    pub(crate) struct Chemist;
    #[mod_api::__subscriptions]
    impl Hooks for Chemist {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let chem_vat = r.block_entity_type_key("test:chem_vat");
            r.block_entity_capability(&CHEMICALS, &chem_vat, |_, at| Some(Box::new(ChemTank(at))));
        }
    }

    pub(crate) const CAPABILITY_TYPES: &[&str] = &[
        "pipe",
        "greedy",
        "item_overstates",
        "ordered",
        "tank",
        "gives_more",
        "takes_more",
        "battery_gives_more",
        "battery_takes_more",
        "chem_tank",
        "chem_vat",
    ];

    #[test]
    pub(crate) fn a_level_not_in_place_asks_no_provider() {
        game_thread();
        let mut world = pipe_world();
        let host: &mut dyn ServerHost = &mut world;
        // The pipe's type has a provider, which answers in the level in place; parked while
        // the server ticks another, the same block entity's level gives none.
        let parked = LevelRef::new(WorldEpoch::new(1), crate::DimensionKey::NETHER);
        assert!(here().capability(host, &ITEMS, PIPE, None).is_some());
        assert!(parked.capability(host, &ITEMS, PIPE, None).is_none());
    }

    #[test]
    pub(crate) fn a_mod_reads_a_vanilla_chests_items_from_the_game_with_no_provider_for_it() {
        game_thread();
        let mut world = pipe_world();
        let chest = world.level.block_entity(CHEST).items.as_mut().unwrap();
        chest.slots[0] = Some(stick());
        let host: &mut dyn ServerHost = &mut world;
        // No mod registered an items provider for the chest's type: the game's hoppers have no
        // mod's handler to look for there, yet a mod reading it gets the game's own slots.
        assert!(block_providers(&ITEMS, host, on(CHEST, None)).is_none());
        let chest = here().capability(host, &ITEMS, CHEST, None).unwrap();
        assert_eq!(chest.owner(), "the game");
        assert_eq!(chest.size(host), 1);
        assert_eq!(chest.resource(host, 0), Some(ItemResource::of(&stick())));
    }

    #[test]
    pub(crate) fn a_capability_part_answers_by_side_and_forwards_to_its_neighbours_capability() {
        game_thread();
        let mut world = pipe_world();
        let host: &mut dyn ServerHost = &mut world;
        let up = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::Up))
            .unwrap();
        assert_eq!(up.size(host), 0);
        let pipe = here()
            .capability(host, &ITEMS, PIPE, Some(Direction::North))
            .expect("the pipe's part");
        assert_eq!(pipe.size(host), 1);
        let stick_ = ItemResource::of(&stick());
        // The pipe calls the chest's capability, the game's own, with the host it was given:
        // in an aborted transaction the stick goes in, and comes back out.
        let aborted: Result<(), u32> =
            host.transaction(|host, tx| Err(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(aborted, Err(1));
        assert!(
            backend_of(&*host)
                .and_then(|backend| backend.container_stack(on(CHEST, None), 0))
                .is_none()
        );
        let went = host.transaction(|host, tx| Ok::<_, ()>(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(went, Ok(1));
        assert_eq!(
            backend_of(&*host).and_then(|backend| backend.container_stack(on(CHEST, None), 0)),
            Some(stick())
        );
        let went = host.transaction(|host, tx| Ok::<_, ()>(pipe.insert(host, tx, 0, &stick_, 1)));
        assert_eq!(went, Ok(0));
        // No block entity with slots: no answer. A mod's own capability has no game answer.
        assert!(here().capability(host, &ITEMS, [5, 64, 0], None).is_none());
        const CHEMICALS: CapabilityKey<dyn ResourceHandler<ItemResource>> =
            CapabilityKey::new("test:chemicals");
        assert!(here().capability(host, &CHEMICALS, CHEST, None).is_none());
    }

    #[cfg(test)]
    /// Puts `amount` of `item` into slot 0 of [`ITEM_ITEMS`]' handler for the item at `place`, in a
    /// transaction committed when all went in: how much did.
    pub(crate) fn fill(
        host: &mut dyn ServerHost,
        place: &ItemPlace,
        item: ItemStack,
        amount: u32,
    ) -> u32 {
        let handler = place
            .capability(host, &ITEM_ITEMS)
            .expect("the item's part");
        let resource = ItemResource::of(&item);
        let went =
            host.transaction(
                |host, tx| match handler.insert(host, tx, 0, &resource, amount) {
                    went if went == amount => Ok(went),
                    went => Err(went),
                },
            );
        went.unwrap_or_else(|went| went)
    }

    /// A mod declaring its own key under a name another mod's providers answer with another
    /// handler type is refused at load, naming both mods and both types, as is a key whose name
    /// has no namespace; the same name with the same type, from another mod, is not.
    #[test]
    pub(crate) fn a_capability_name_answered_with_two_handler_types_is_refused() {
        const NOT_CHEMICALS: CapabilityKey<dyn ResourceHandler<FluidResource>> =
            CapabilityKey::new("test:chemical");
        const UNNAMESPACED: CapabilityKey<dyn ResourceHandler<Chemical>> =
            CapabilityKey::new("chemical");
        let mut r = super::PartRegistrar::new("first");
        let chem_tank = r.block_entity_type_key("test:chem_tank");
        (&mut r as &mut dyn Registrar)
            .block_entity_capability(&CHEMICALS, &chem_tank, |_, at| Some(Box::new(ChemTank(at))));
        r.mod_id = "second";
        r.mod_index = 1;
        let chem_vat = r.block_entity_type_key("test:chem_vat");
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &NOT_CHEMICALS,
            &chem_vat,
            |_, at| Some(Box::new(ModTank { at, lie: None })),
        );
        (&mut r as &mut dyn Registrar)
            .block_entity_capability(&CHEMICALS, &chem_vat, |_, at| Some(Box::new(ChemTank(at))));
        (&mut r as &mut dyn Registrar).block_entity_capability(
            &UNNAMESPACED,
            &chem_vat,
            |_, at| Some(Box::new(ChemTank(at))),
        );
        assert_eq!(r.refused.len(), 2, "{:?}", r.refused);
        let two_types = &r.refused[0];
        assert!(
            two_types.starts_with("capability test:chemical is answered with two handler types")
        );
        let chemicals = type_name::<dyn ResourceHandler<Chemical>>();
        let fluids = type_name::<dyn ResourceHandler<FluidResource>>();
        assert!(
            two_types.contains(&format!("{chemicals} by first")),
            "{two_types}"
        );
        assert!(
            two_types.contains(&format!("{fluids} by second")),
            "{two_types}"
        );
        assert!(r.refused[1].contains("capability \"chemical\", which is not a namespaced name"));
    }

    /// A handler that only tells which provider made it, by its slot count.
    pub(crate) struct Marked(pub(crate) usize);
    impl ResourceHandler<ItemResource> for Marked {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            self.0
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

    /// A trash can's one slot, which takes any amount of anything and keeps none of it.
    pub(crate) struct VoidBin;
    impl ResourceHandler<ItemResource> for VoidBin {
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
            u32::MAX
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &Transaction,
            _: usize,
            _: &ItemResource,
            amount: u32,
        ) -> u32 {
            amount
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
    pub(crate) fn a_block_with_no_block_entity_answers_through_its_block_provider() {
        game_thread();
        let bin = [0, 64, 0];
        let mut world = FakeHost::new();
        world.level.set_block(bin, "test:void_bin");
        world.level.set_block([1, 64, 0], "minecraft:stone");
        let host: &mut dyn ServerHost = &mut world;
        let voids = here()
            .capability(host, &ITEMS, bin, Some(Direction::Up))
            .expect("the bin's provider");
        assert_eq!(voids.owner(), "doubler");
        let stick_ = ItemResource::of(&stick());
        let went: Result<u32, ()> =
            host.transaction(|host, tx| Ok(voids.insert(host, tx, 0, &stick_, 64)));
        assert_eq!(went, Ok(64));
        assert_eq!(voids.amount(host, 0), 0);
        // A block no provider is registered for has none, as before.
        assert!(here().capability(host, &ITEMS, [1, 64, 0], None).is_none());
        // A hopper's lookup finds the bin where the game has no slots of its own.
        let hopper = block_providers(&ITEMS, host, on(bin, Some(Direction::Up)))
            .expect("the bin's providers")
            .ask(host, false)
            .expect("the bin's handler");
        assert_eq!(hopper.size(host), 1);
        // Providers chosen for the bin don't answer once the block there is another (an earlier
        // provider replaced it, say).
        let chosen = block_providers(&ITEMS, host, on(bin, None)).expect("the bin's providers");
        world.level.set_block(bin, "minecraft:stone");
        let host: &mut dyn ServerHost = &mut world;
        assert!(chosen.ask(host, false).is_none());
        world.level.set_block(bin, "test:void_bin");
        // The provider is given the block entity standing there, and this one declines then.
        world.level.block_entity(bin);
        let host: &mut dyn ServerHost = &mut world;
        assert!(here().capability(host, &ITEMS, bin, None).is_none());
    }

    /// A position's providers, of the block and of the block entity type standing there, are one
    /// list in registration order across both kinds: those ahead of the game, the game's own,
    /// then the others.
    #[test]
    pub(crate) fn a_positions_block_and_block_entity_type_providers_follow_registration_order() {
        game_thread();
        let mut world = FakeHost::new();
        for x in [3, 4, 5, 6, 11, 12] {
            world.level.set_block([x, 64, 0], "test:ordered");
        }
        for x in [4, 5, 6, 11, 12] {
            world.level.block_entity([x, 64, 0]).of_type("test:ordered");
        }
        world.level.block_entity([4, 64, 0]).with_items(27);
        let host: &mut dyn ServerHost = &mut world;
        let size = |host: &mut dyn ServerHost, x| {
            let found = here().capability(host, &ITEMS, [x, 64, 0], None).unwrap();
            found.size(host)
        };
        // Ahead of the game, the block's (8), registered before the block entity type's (4).
        assert_eq!(size(host, 12), 8);
        // Ahead of the game, the block entity type's (4) before the block's registered later (5).
        assert_eq!(size(host, 11), 4);
        // The block's ahead of the game (5) before every provider after it.
        assert_eq!(size(host, 6), 5);
        // After the game, the block's (7), registered before the block entity type's (1).
        assert_eq!(size(host, 5), 7);
        // The game's 27 slots before every provider after it.
        assert_eq!(size(host, 4), 27);
        // No block entity: the block's alone, the first that answers there (6).
        assert_eq!(size(host, 3), 6);
        // A hopper's lookup asks them in the same order.
        let ask = |host: &mut dyn ServerHost, x, game| {
            let found = block_providers(&ITEMS, host, on([x, 64, 0], None))?.ask(host, game)?;
            Some(found.size(host))
        };
        assert_eq!(ask(host, 12, true), Some(8));
        assert_eq!(ask(host, 5, false), Some(7));
        assert_eq!(ask(host, 6, true), Some(5));
        assert_eq!(ask(host, 4, true), None);
    }

    /// One capability name has one handler type across a block's and a block entity type's
    /// providers, as they answer the same lookup: the second type is refused at load, whichever
    /// kind registered first.
    #[test]
    pub(crate) fn a_block_and_a_block_entity_type_answering_one_name_with_two_types_are_refused() {
        const SAME_ITEMS: CapabilityKey<dyn ResourceHandler<ItemResource>> =
            CapabilityKey::new("test:same");
        const SAME_ENERGY: CapabilityKey<dyn EnergyHandler> = CapabilityKey::new("test:same");
        let items = type_name::<dyn ResourceHandler<ItemResource>>();
        let energy = type_name::<dyn EnergyHandler>();
        let battery = |at| {
            Some(Box::new(ModBattery {
                at,
                lie: Lie::GivesMore,
            }) as _)
        };

        let mut r = super::PartRegistrar::new("first");
        let chest = r.block_entity_type_key("test:ordered");
        let bin = r.block_key("test:void_bin");
        (&mut r as &mut dyn Registrar)
            .block_entity_capability(&SAME_ITEMS, &chest, |_, _| Some(Box::new(Marked(1))));
        r.mod_id = "second";
        r.mod_index = 1;
        (&mut r as &mut dyn Registrar)
            .block_capability(&SAME_ENERGY, &bin, move |_, at, _, _| battery(at));
        assert_eq!(r.refused.len(), 1, "{:?}", r.refused);
        assert!(
            r.refused[0].contains(&format!("{items} by first and {energy} by second")),
            "{}",
            r.refused[0]
        );
        assert!(!r.parts.block_capabilities.contains_key("test:same"));

        let mut r = super::PartRegistrar::new("first");
        let chest = r.block_entity_type_key("test:ordered");
        let bin = r.block_key("test:void_bin");
        (&mut r as &mut dyn Registrar)
            .block_capability(&SAME_ENERGY, &bin, move |_, at, _, _| battery(at));
        (&mut r as &mut dyn Registrar)
            .block_entity_capability(&SAME_ITEMS, &chest, |_, _| Some(Box::new(Marked(1))));
        assert_eq!(r.refused.len(), 1, "{:?}", r.refused);
        assert!(
            r.refused[0].contains(&format!("{energy} by first and {items} by first")),
            "{}",
            r.refused[0]
        );
        assert!(!r.parts.capabilities.contains_key("test:same"));
    }

    #[test]
    fn block_provider_limits_accept_65535_and_refuse_the_next() {
        for block_entity in [false, true] {
            let mut r = super::PartRegistrar::new("limit_mod");
            let block = r.block_key("test:target");
            let entity = r.block_entity_type_key("test:target");
            for _ in 0..65_535 {
                let registrar = &mut r as &mut dyn Registrar;
                if block_entity {
                    registrar.block_entity_capability(&ITEMS, &entity, |_, _| None);
                } else {
                    registrar.block_capability(&ITEMS, &block, |_, _, _, _| None);
                }
            }
            assert!(r.refused.is_empty(), "{:?}", r.refused);
            if block_entity {
                (&mut r as &mut dyn Registrar)
                    .block_entity_capability(&ITEMS, &entity, |_, _| None);
            } else {
                (&mut r as &mut dyn Registrar).block_capability(&ITEMS, &block, |_, _, _, _| None);
            }
            assert_eq!(r.refused.len(), 1);
            for expected in ["limit_mod", "test:target", "65535", "horizon:items"] {
                assert!(r.refused[0].contains(expected), "{}", r.refused[0]);
            }
        }
    }

    #[test]
    pub(crate) fn disabled_capability_providers_are_skipped_and_resume() {
        struct Later;
        #[mod_api::__subscriptions]
        impl Hooks for Later {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(99))));
                r.item_capability(&ITEM_ITEMS, "test:ordered_item", |_, _| {
                    Some(Box::new(Marked(99)))
                });
                r.entity_capability(&ENTITY_ITEMS, "test:robot", |_, _, ()| {
                    Some(Box::new(Marked(99)))
                });
            }
        }
        let mods: crate::Mods = vec![
            ModEntry::new("doubler", Box::new(Doubler)),
            ModEntry::new("later", Box::new(Later)),
        ]
        .into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();

        // These lookup objects keep their indexed registrations, not a snapshot of enabled mods.
        let at = BlockSide {
            level: here(),
            pos: [7, 64, 0],
            side: None,
        };
        let direct_block = || block_providers(&ITEMS, &ordered_world(), at).unwrap();
        let robot = EntityRef::new(WorldEpoch::new(1), 3);
        let mut player = player_world(None);
        let direct_entity =
            || entity_providers(&ENTITY_ITEMS, &player_world(None), robot, ()).unwrap();
        let block_before_disable = direct_block();
        let entity_before_disable = direct_entity();
        let check = |without_game: Option<usize>, with_game: usize, robot_size: Option<usize>| {
            let host: &mut dyn ServerHost = &mut ordered_world();
            let block = here().capability(host, &ITEMS, [7, 64, 0], None);
            assert_eq!(block.map(|found| found.size(host)), without_game);
            let block = here().capability(host, &ITEMS, [9, 64, 0], None).unwrap();
            assert_eq!(block.size(host), with_game);
            assert_eq!(
                direct_block()
                    .ask(host, false)
                    .map(|found| found.size(host)),
                without_game
            );
            assert_eq!(
                direct_block().ask(host, true).map(|found| found.size(host)),
                None
            );
            for (damage, expected) in [(0, without_game), (1, Some(with_game))] {
                let host: &mut dyn ServerHost =
                    &mut HeldItem(stack("test:ordered_item", 1, damage));
                let item = HAND.capability(host, &ITEM_ITEMS);
                assert_eq!(item.map(|found| found.size(host)), expected);
            }
            let host: &mut dyn ServerHost = &mut player_world(None);
            let entity = robot.capability(host, &ENTITY_ITEMS, ());
            assert_eq!(entity.map(|found| found.size(host)), robot_size);
            assert_eq!(
                direct_entity()
                    .ask(host, false)
                    .map(|found| found.size(host)),
                robot_size
            );
        };
        // At x = 9 Doubler replaces the game's block provider, while the item with damage 1
        // uses the game's handler (27), so check that special block answer separately.
        assert_eq!(
            here()
                .capability(&mut ordered_world(), &ITEMS, [9, 64, 0], None)
                .unwrap()
                .size(&mut ordered_world()),
            3
        );
        assert!(crate::__private::set_mod_enabled("doubler", false));
        check(Some(99), 27, Some(99));
        assert_eq!(
            block_before_disable
                .ask(&mut ordered_world(), false)
                .unwrap()
                .size(&mut ordered_world()),
            99
        );
        let entity = entity_before_disable.ask(&mut player, false).unwrap();
        assert_eq!(entity.size(&mut player), 99);
        assert!(crate::__private::set_mod_enabled("later", false));
        check(None, 27, None);
        assert!(crate::__private::set_mod_enabled("later", true));
        check(Some(99), 27, Some(99));
        assert!(crate::__private::set_mod_enabled("doubler", true));
        assert_eq!(
            here()
                .capability(&mut ordered_world(), &ITEMS, [7, 64, 0], None)
                .unwrap()
                .size(&mut ordered_world()),
            1
        );
        assert_eq!(
            robot
                .capability(&mut player, &ENTITY_ITEMS, ())
                .unwrap()
                .size(&mut player),
            3
        );
        let host: &mut dyn ServerHost = &mut HeldItem(stack("test:ordered_item", 1, 0));
        assert_eq!(HAND.capability(host, &ITEM_ITEMS).unwrap().size(host), 1);
    }

    #[test]
    pub(crate) fn disabling_a_provider_during_lookup_does_not_skip_the_next_mod() {
        struct SwitchOff;
        #[mod_api::__subscriptions]
        impl Hooks for SwitchOff {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _| {
                    assert!(crate::__private::set_mod_enabled("switch", false));
                    None
                });
                r.block_entity_capability(&ITEMS, &ordered, |_, _| {
                    panic!("a disabled mod's later provider must not run")
                });
            }
        }
        struct Later;
        #[mod_api::__subscriptions]
        impl Hooks for Later {
            fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
                let ordered = r.block_entity_type_key("test:ordered");
                r.block_entity_capability(&ITEMS, &ordered, |_, _| Some(Box::new(Marked(99))));
            }
        }
        let mods: crate::Mods = vec![
            ModEntry::new("switch", Box::new(SwitchOff)),
            ModEntry::new("later", Box::new(Later)),
        ]
        .into();
        crate::__private::install(mods.clone());
        install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        mark_game_thread();
        let found = here()
            .capability(&mut ordered_world(), &ITEMS, [7, 64, 0], None)
            .unwrap();
        assert_eq!(
            (found.owner(), found.size(&mut ordered_world())),
            ("later".into(), 99)
        );
    }

    #[test]
    pub(crate) fn providers_are_asked_ahead_of_the_game_then_the_game_then_the_mods_in_order() {
        game_thread();
        let host: &mut dyn ServerHost = &mut ordered_world();
        let answer = |host: &mut dyn ServerHost, x| {
            let found = here().capability(host, &ITEMS, [x, 64, 0], None).unwrap();
            (
                mod_vocab::__private::parts_installation::capability_is_owned(&found),
                found.size(host),
            )
        };
        // No game slots: the first mod provider that answers; the one before it declined.
        assert_eq!(answer(host, 7), (true, 1));
        // The game's slots come before the mods' providers.
        assert_eq!(answer(host, 8), (false, 27));
        // Providers registered before the game come first where they answer, in registration
        // order: at x = 9 both answer, at x = 10 only the second.
        assert_eq!(answer(host, 9), (true, 3));
        assert_eq!(answer(host, 10), (true, 4));
        // The game's own lookup takes a mod's handler only where the game has none or a mod
        // comes first.
        let mod_only = |host: &mut dyn ServerHost, x| {
            let at = BlockSide {
                level: here(),
                pos: [x, 64, 0],
                side: None,
            };
            let game_slots = backend_of(&*host)
                .and_then(|backend| backend.container_size(at))
                .is_some();
            let providers = block_providers(&ITEMS, host, at)?;
            providers
                .ask(host, game_slots)
                .map(|found| found.size(host))
        };
        assert_eq!(mod_only(host, 7), Some(1));
        assert_eq!(mod_only(host, 8), None);
        assert_eq!(mod_only(host, 9), Some(3));
        // With no provider for the block entity's type, or no type (a vanilla one), there are
        // no providers to ask, so the game needn't work out its own answer: a hopper's lookup
        // costs one table check.
        let at = BlockSide {
            level: here(),
            pos: [8, 64, 0],
            side: None,
        };
        let mut vanilla = FakeHost::new();
        vanilla.level.block_entity(at.pos);
        assert!(block_providers(&ITEMS, &vanilla, at).is_none());
        let mut unregistered = FakeHost::new();
        unregistered
            .level
            .block_entity(at.pos)
            .of_type("test:unregistered");
        assert!(block_providers(&ITEMS, &unregistered, at).is_none());

        // Item providers in the same order, for the item in a player's hand: with no game slots
        // (damage 0), the first mod's that answers; with them (damage 1), the game's; for two
        // items, the one ahead of the game.
        let item = |damage, count| {
            let mut host = HeldItem(stack("test:ordered_item", count, damage));
            let host: &mut dyn ServerHost = &mut host;
            let found = HAND.capability(host, &ITEM_ITEMS).unwrap();
            (
                mod_vocab::__private::parts_installation::capability_is_owned(&found),
                found.size(host),
            )
        };
        assert_eq!(item(0, 1), (true, 1));
        assert_eq!(item(1, 1), (false, 27));
        assert_eq!(item(1, 2), (true, 3));
    }
}

pub(crate) enum ProviderTarget<'a> {
    BlockEntity(&'a crate::BlockEntityTypeKey),
    Id(&'a str),
}
