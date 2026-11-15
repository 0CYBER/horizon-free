//! Concrete authoring extracted verbatim from the vocabulary layer.

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::testing::{FakeBattery, FakeHost};
    #[cfg(test)]
    /// What the batteries at `at` hold.
    pub(crate) fn energy(host: &FakeHost, at: &[[i32; 3]]) -> Vec<u32> {
        let held = |pos| host.level.block_entity_at(pos)?.energy.as_ref();
        at.iter().map(|&pos| held(pos).unwrap().amount).collect()
    }

    #[cfg(test)]
    /// Puts a battery of capacity 1000 at `pos`, as the game answers it, holding `amount`, of
    /// which one insert moves at most `limit`.
    pub(crate) fn battery(host: &mut FakeHost, pos: [i32; 3], amount: u32, limit: u32) {
        host.level.block_entity(pos).energy = Some({
            let mut value: FakeBattery = FakeBattery::new(amount, 1000);
            value.max_insert = limit;
            value
        });
    }

    use crate::parts::tests::*;
    use crate::*;

    /// A mod's battery: the game's at its position, misanswering by 100 as `lie` says.
    pub(crate) struct ModBattery {
        pub(crate) at: BlockSide,
        pub(crate) lie: Lie,
    }
    impl EnergyHandler for ModBattery {
        fn amount(&self, host: &mut dyn ServerHost) -> u32 {
            crate::backend_of(host)
                .and_then(|backend| backend.energy_amount(self.at))
                .unwrap_or(0)
        }
        fn capacity(&self, host: &mut dyn ServerHost) -> u32 {
            crate::backend_of(host).map_or(0, |backend| backend.energy_capacity(self.at))
        }
        fn insert(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let went = crate::backend_mut_of(host)
                .map_or(0, |backend| backend.energy_insert(self.at, amount, tx));
            match self.lie {
                Lie::TakesMore => went + 100,
                Lie::GivesMore => went,
            }
        }
        fn extract(&self, host: &mut dyn ServerHost, tx: &Transaction, amount: u32) -> u32 {
            let more = match self.lie {
                Lie::GivesMore => 100,
                Lie::TakesMore => 0,
            };
            crate::backend_mut_of(host).map_or(0, |backend| {
                backend.energy_extract(self.at, amount + more, tx)
            })
        }
    }

    #[test]
    pub(crate) fn an_energy_move_takes_what_the_targets_room_and_limit_let_in_and_loses_none() {
        game_thread();
        let mut world = FakeHost::new();
        battery(&mut world, SOURCE, 900, 1000);
        battery(&mut world, TANK, 700, 1000);
        battery(&mut world, OTHER, 0, 250);
        let mut moved = |from, to, max| {
            let host: &mut dyn ServerHost = &mut world;
            let source = here().capability(host, &ENERGY, from, None).unwrap();
            let target = here().capability(host, &ENERGY, to, None).unwrap();
            host.transaction(|host, tx| move_energy(host, &source, &target, max, tx))
        };
        // The target's room, then its limit per insert, each by a move tried again with what
        // it took; and no more than the source holds.
        assert_eq!(moved(SOURCE, TANK, 1000), Ok(300));
        assert_eq!(moved(SOURCE, OTHER, 1000), Ok(250));
        assert_eq!(moved(OTHER, SOURCE, 1000), Ok(250));
        assert_eq!(energy(&world, &[SOURCE, TANK, OTHER]), [600, 1000, 0]);
    }

    #[test]
    pub(crate) fn a_battery_answering_more_than_it_was_asked_aborts_the_whole_move_and_loses_no_energy()
     {
        game_thread();
        for (liar, at) in [
            ("test:battery_gives_more", SOURCE),
            ("test:battery_takes_more", TANK),
        ] {
            let mut world = FakeHost::new();
            battery(&mut world, SOURCE, 500, 1000);
            battery(&mut world, OTHER, 500, 1000);
            battery(&mut world, TANK, 0, 1000);
            world.level.block_entity(at).of_type(liar);
            let host: &mut dyn ServerHost = &mut world;
            let ends = |host: &mut dyn ServerHost, pos| here().capability(host, &ENERGY, pos, None);
            let (source, other, tank) = (
                ends(host, SOURCE).unwrap(),
                ends(host, OTHER).unwrap(),
                ends(host, TANK).unwrap(),
            );
            // Two moves into the battery at `TANK` in one transaction, the second from the
            // source: a move through the liar fails, and its error aborts the transaction.
            let moved = host.transaction(|host, tx| {
                let first = move_energy(host, &other, &tank, 200, tx)?;
                let second = move_energy(host, &source, &tank, 200, tx)?;
                Ok(first + second)
            });
            let owner = "doubler".to_owned();
            assert_eq!(
                moved,
                Err(mod_vocab::__private::parts_installation::overanswered(
                    owner
                )),
                "{liar}"
            );
            assert_eq!(
                energy(&world, &[SOURCE, OTHER, TANK]),
                [500, 500, 0],
                "{liar}"
            );
        }
    }
}
