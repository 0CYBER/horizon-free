/// Install a part on glass that, when a neighbour changes, reads its inputs and
/// schedules its own tick 2 ticks later, as a diode does, for real-game tests.
pub fn signal_reader_game_thread() {
    struct Reader(BlockKey);
    #[mod_api::horizon_part]
    impl BlockPart for Reader {
        fn neighbor_changed(
            &self,
            level: &mut dyn crate::LevelTicksHost,
            event: &mut crate::NeighborChangedEvent,
        ) {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            let read = SignalRead::Changed(
                pos,
                level.has_neighbor_signal(pos),
                level.best_neighbor_signal(pos),
                level.schedule_block_tick(pos, self.0, 2),
            );
            SIGNAL_READS.with(|r| r.borrow_mut().push(read));
        }
        fn tick_block(
            &self,
            level: &mut dyn crate::LevelWriteHost,
            event: &mut crate::TickBlockEvent,
        ) -> Flow {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            let south = [pos[0], pos[1], pos[2] + 1];
            let read = SignalRead::Ticked(pos, level.signal(south, Direction::South));
            SIGNAL_READS.with(|r| r.borrow_mut().push(read));
            Flow::Continue
        }
    }
    struct ReaderMod;
    #[mod_api::__subscriptions]
    impl Hooks for ReaderMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let block_key = r.block_key("minecraft:glass");
            r.block_part(block_key, Reader(block_key));
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("signal_reader_test", Box::new(ReaderMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
