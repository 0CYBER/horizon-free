#[cfg(test)]
mod tests {
    use crate::testing::{FakeHost, FakeTank};
    use crate::{
        Capability, FluidResource, FluidStack, Hand, HostTransaction, ITEM_FLUIDS, Identifier,
        ItemPlace, ItemStack, ResourceHandler, ServerHost, fluid_util,
    };

    const SOURCE: [i32; 3] = [3, 64, 0];
    const TANK: [i32; 3] = [0, 64, 0];
    const PLAYER: u128 = 7;

    /// A host with vanilla's buckets and fluids, an empty 8000 mB tank at `TANK`, and player
    /// `PLAYER` holding `count` of `item` in the main hand.
    fn host(item: &str, count: u32) -> FakeHost {
        let mut host = FakeHost::new();
        let items = [
            "minecraft:bucket",
            "minecraft:water_bucket",
            "minecraft:lava_bucket",
        ];
        host.registries
            .resolve_items(items.map(|id| Identifier::parse(id).unwrap()))
            .unwrap();
        host.registries
            .resolve_fluids(["minecraft:empty", "minecraft:water", "minecraft:lava"])
            .unwrap();
        host.registries.fluid_sounds(
            "minecraft:lava",
            "minecraft:item.bucket.fill_lava",
            "minecraft:item.bucket.empty_lava",
        );
        host.level.block_entity(TANK).tanks = vec![FakeTank::new(8000)];
        host.join(PLAYER, 70).slots.insert(0, stack(item, count));
        host
    }

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack::new(crate::__private::item_key(item).unwrap(), count).unwrap()
    }

    fn fluid(id: &str, amount: u32) -> FluidStack {
        FluidStack::new(crate::__private::fluid_key(id).unwrap(), amount)
    }

    fn hand(host: &FakeHost) -> Option<ItemStack> {
        host.players.get(PLAYER).unwrap().main_hand().cloned()
    }

    fn place() -> ItemPlace {
        ItemPlace::PlayerHand {
            player: PLAYER,
            hand: Hand::Main,
        }
    }

    /// The fluid container `PLAYER` holds.
    fn held(server: &mut dyn ServerHost) -> Capability<dyn ResourceHandler<FluidResource>> {
        place().capability(server, &ITEM_FLUIDS).unwrap()
    }

    fn pickup(host: &mut FakeHost, side_tanks: bool) -> Option<FluidStack> {
        let server: &mut dyn ServerHost = host;
        let player = server.player(PLAYER).unwrap();
        let level = player.level(server).unwrap();
        let bucket = held(server);
        match side_tanks {
            true => fluid_util::try_pickup_fluid(server, &bucket, Some(player), level, TANK, None),
            false => {
                fluid_util::try_pickup_world_fluid(server, &bucket, Some(player), level, SOURCE)
            }
        }
    }

    fn put(host: &mut FakeHost) -> Option<FluidStack> {
        let server: &mut dyn ServerHost = host;
        let player = server.player(PLAYER).unwrap();
        let level = player.level(server).unwrap();
        let bucket = held(server);
        fluid_util::try_place_fluid(server, &bucket, Some(player), level, SOURCE, true)
    }

    #[test]
    fn an_empty_bucket_takes_a_world_source_whole() {
        let mut host = host("minecraft:bucket", 1);
        host.level.set_block(SOURCE, "minecraft:water");
        assert_eq!(
            pickup(&mut host, false),
            Some(fluid("minecraft:water", 1000))
        );
        assert_eq!(hand(&host), Some(stack("minecraft:water_bucket", 1)));
        // The source is gone: a second pickup gets nothing.
        host.players
            .get_mut(PLAYER)
            .unwrap()
            .slots
            .insert(0, stack("minecraft:bucket", 1));
        assert_eq!(pickup(&mut host, false), None);
        assert_eq!(hand(&host), Some(stack("minecraft:bucket", 1)));
    }

    #[test]
    fn a_container_that_cannot_take_the_whole_bucket_leaves_the_source() {
        let mut host = host("minecraft:lava_bucket", 1);
        host.level.set_block(SOURCE, "minecraft:water");
        assert_eq!(pickup(&mut host, false), None);
        assert_eq!(hand(&host), Some(stack("minecraft:lava_bucket", 1)));
        // The aborted pickup left the source where it was.
        host.players
            .get_mut(PLAYER)
            .unwrap()
            .slots
            .insert(0, stack("minecraft:bucket", 1));
        assert_eq!(
            pickup(&mut host, false),
            Some(fluid("minecraft:water", 1000))
        );
    }

    #[test]
    fn a_block_with_no_source_gives_from_its_tanks_with_the_fill_sound() {
        let mut host = host("minecraft:bucket", 1);
        host.level.block_entity(TANK).tanks[0].held = Some(fluid("minecraft:lava", 2500));
        assert_eq!(pickup(&mut host, true), Some(fluid("minecraft:lava", 1000)));
        assert_eq!(hand(&host), Some(stack("minecraft:lava_bucket", 1)));
        let tank = &host.level.block_entity_at(TANK).unwrap().tanks[0];
        assert_eq!(tank.held, Some(fluid("minecraft:lava", 1500)));
        assert_eq!(host.sounds.len(), 1);
        assert_eq!(host.sounds[0].sound, "minecraft:item.bucket.fill_lava");
        assert_eq!(host.sounds[0].at, [0.5, 64.5, 0.5]);
        assert_eq!(host.game_events.len(), 1);
        assert_eq!(host.game_events[0].event, "minecraft:fluid_pickup");
    }

    #[test]
    fn a_tank_fallback_in_an_aborted_transaction_leaves_no_sound_or_event() {
        let mut host = host("minecraft:bucket", 1);
        host.level.block_entity(TANK).tanks[0].held = Some(fluid("minecraft:lava", 2500));
        let server: &mut dyn ServerHost = &mut host;
        let aborted: Result<(), ()> = server.transaction(|host, _| {
            let player = host.player(PLAYER).unwrap();
            let level = player.level(host).unwrap();
            let bucket = held(host);
            let picked =
                fluid_util::try_pickup_fluid(host, &bucket, Some(player), level, TANK, None);
            assert_eq!(picked, Some(fluid("minecraft:lava", 1000)));
            Err(())
        });
        assert!(aborted.is_err());
        let tank = &host.level.block_entity_at(TANK).unwrap().tanks[0];
        assert_eq!(tank.held, Some(fluid("minecraft:lava", 2500)));
        assert_eq!(hand(&host), Some(stack("minecraft:bucket", 1)));
        assert!(host.sounds.is_empty() && host.game_events.is_empty());
    }

    #[test]
    fn a_tank_fallback_in_a_committed_transaction_plays_once_at_the_commit() {
        let mut host = host("minecraft:bucket", 1);
        host.level.block_entity(TANK).tanks[0].held = Some(fluid("minecraft:lava", 2500));
        let server: &mut dyn ServerHost = &mut host;
        let committed: Result<(), ()> = server.transaction(|host, _| {
            let player = host.player(PLAYER).unwrap();
            let level = player.level(host).unwrap();
            let bucket = held(host);
            fluid_util::try_pickup_fluid(host, &bucket, Some(player), level, TANK, None)
                .ok_or(())?;
            Ok(())
        });
        assert!(committed.is_ok());
        assert_eq!(host.sounds.len(), 1);
        assert_eq!(host.sounds[0].sound, "minecraft:item.bucket.fill_lava");
        assert_eq!(host.game_events.len(), 1);
        assert_eq!(host.game_events[0].event, "minecraft:fluid_pickup");
    }

    #[test]
    fn a_full_bucket_places_its_fluid_and_empties() {
        let mut host = host("minecraft:water_bucket", 1);
        assert_eq!(put(&mut host), Some(fluid("minecraft:water", 1000)));
        assert_eq!(hand(&host), Some(stack("minecraft:bucket", 1)));
        // What it placed is a source a bucket takes back.
        assert_eq!(
            pickup(&mut host, false),
            Some(fluid("minecraft:water", 1000))
        );
    }

    #[test]
    fn a_bucket_placing_where_the_block_refuses_keeps_its_fluid() {
        let mut full = host("minecraft:water_bucket", 1);
        full.level.set_block(SOURCE, "minecraft:stone");
        assert_eq!(put(&mut full), None);
        assert_eq!(hand(&full), Some(stack("minecraft:water_bucket", 1)));
        let mut empty = host("minecraft:bucket", 1);
        assert_eq!(put(&mut empty), None);
        assert_eq!(hand(&empty), Some(stack("minecraft:bucket", 1)));
    }

    #[test]
    fn the_first_stack_contained_ignores_the_stack_count() {
        let mut full = host("minecraft:lava_bucket", 3);
        assert_eq!(
            fluid_util::first_stack_contained(&mut full, &place()),
            Some(fluid("minecraft:lava", 1000))
        );
        let mut empty = host("minecraft:bucket", 1);
        assert_eq!(
            fluid_util::first_stack_contained(&mut empty, &place()),
            None
        );
    }
}
