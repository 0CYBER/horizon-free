impl Game<'_> {
    pub(super) fn unscoped_level_all_entities(&self, level: LevelRef) -> Option<Vec<EntityRef>> {
        if !self.is_logical_server()
            || !self.level_handle_current(level)
            || level.dimension() != self.dim.key()
        {
            return None;
        }
        let ids = self
            .resident_entities
            .table
            .borrow_mut()
            .level_entities(level.dimension());
        ids.into_iter()
            .filter(|id| {
                !self
                    .persist
                    .removals
                    .iter()
                    .chain(self.persist.destroying_vehicles.iter())
                    .any(|target| matches!(*target, Target::Boat(n) | Target::Cart(n) if n == *id))
            })
            .map(|id| {
                let entity = EntityRef::new(level.epoch(), id);
                self.native_entity_target(entity).map(|_| entity)
            })
            .collect()
    }

    #[cold]
    #[inline(never)]
    fn scoped_level_all_entities(&self, level: LevelRef) -> Option<Vec<EntityRef>> {
        if !self.is_logical_server()
            || !self.level_handle_current(level)
            || level.dimension() != self.dim.key()
        {
            return None;
        }
        let ids = self
            .resident_entities
            .table
            .borrow_mut()
            .level_entities(level.dimension());
        Some(
            ids.into_iter()
                .filter(|id| {
                    !self
                        .persist
                        .removals
                        .iter()
                        .chain(self.persist.destroying_vehicles.iter())
                        .any(|target| matches!(*target, Target::Boat(n) | Target::Cart(n) if n == *id))
                })
                .filter_map(|id| {
                    let entity = EntityRef::new(level.epoch(), id);
                    self.native_entity_target(entity).map(|_| entity)
                })
                .collect(),
        )
    }
}
