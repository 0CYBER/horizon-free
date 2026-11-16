#[cfg(test)]
use super::*;

#[cfg(test)]
use crate::testing::{FakeHost, FakeTank};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::ItemsHost;
    use crate::{FLUIDS, FluidResource, Hand, HostTransaction, ItemResource, move_resources};

    fn water(amount: u32) -> FluidStack {
        FluidStack::new(
            mod_vocab::__private::fluid_key("minecraft:water").unwrap(),
            amount,
        )
    }

    #[test]
    fn a_fluid_moves_between_tanks_of_the_same_fluid_and_an_abort_puts_both_back() {
        crate::__private::part_tests::game_thread();
        let mut host = FakeHost::new();
        let level = host.level.level();
        host.level.block_entity([0, 64, 0]).tanks = vec![{
            let mut tank = FakeTank::new(1000);
            tank.held = Some(water(800));
            tank
        }];
        host.level.block_entity([1, 64, 0]).tanks = vec![{
            let mut tank = FakeTank::new(1000);
            tank.held = Some(water(700));
            tank
        }];
        host.level.block_entity([2, 64, 0]).tanks = vec![{
            let mut tank = FakeTank::new(1000);
            tank.held = Some(FluidStack::new(
                mod_vocab::__private::fluid_key("minecraft:lava").unwrap(),
                100,
            ));
            tank
        }];
        let server: &mut dyn ServerHost = &mut host;
        let tank = |host: &mut dyn ServerHost, x| {
            level.capability(host, &FLUIDS, [x, 64, 0], None).unwrap()
        };
        let (from, into, lava) = (tank(server, 0), tank(server, 1), tank(server, 2));
        let any = |_: &FluidResource| true;
        let aborted = server.transaction(|host, tx| {
            let moved = move_resources(host, &from, &into, any, 1000, tx);
            Err::<(), _>(moved)
        });
        assert_eq!(aborted, Err(Ok(300)));
        // Lava takes no water; water fills only up to the target's room.
        let refused =
            server.transaction(|host, tx| move_resources(host, &from, &lava, any, 1000, tx));
        assert_eq!(refused, Ok(0));
        let moved =
            server.transaction(|host, tx| move_resources(host, &from, &into, any, 1000, tx));
        assert_eq!(moved, Ok(300));
        let amount = |x| host.level.block_entity_at([x, 64, 0]).unwrap().tanks[0].amount();
        assert_eq!([amount(0), amount(1), amount(2)], [500, 1000, 100]);
    }

    #[test]
    fn what_a_players_hand_cannot_hold_is_given_to_them_and_an_abort_takes_it_back() {
        let mut host = FakeHost::new();
        host.join(7, 41).selected_slot = 2;
        host.registries
            .item_component("minecraft:egg", "minecraft:max_stack_size", Tag::Int(16));
        let hand = ItemPlace::PlayerHand {
            player: 7,
            hand: Hand::Main,
        };
        let egg = crate::Identifier::parse("minecraft:egg").unwrap();
        host.registries.resolve_items([egg.clone()]).unwrap();
        let eggs = ItemResource::of(&ItemStack::of(host.item_key(&egg).unwrap()));
        let server: &mut dyn ServerHost = &mut host;
        let aborted = server.transaction(|host, tx| Err::<(), _>(hand.insert(host, tx, &eggs, 20)));
        assert_eq!(aborted, Err(20));
        assert!(host.players.get(7).unwrap().slots.is_empty());
        let server: &mut dyn ServerHost = &mut host;
        let went = server.transaction(|host, tx| Ok::<_, ()>(hand.insert(host, tx, &eggs, 20)));
        assert_eq!(went, Ok(20));
        let player = host.players.get(7).unwrap();
        assert_eq!(player.main_hand().map(ItemStack::count), Some(16));
        assert_eq!(player.slots.keys().collect::<Vec<_>>(), [&2]);
        let given: Vec<u32> = player.given.iter().map(ItemStack::count).collect();
        assert_eq!(given, [4]);
    }
}
