impl super::Mob {
    pub(super) fn collision_subject(&self) -> crate::collisions::PowderSnowSubject<'_> {
        use horizon_world::collision_context::CollisionItem;
        crate::collisions::PowderSnowSubject {
            fall_distance: self.fall_distance,
            walkable_type: self.kind.flags.powder_snow_walkable,
            leather_boots: self
                .extra
                .slot_stack(horizon_items::equipment::FEET)
                .is_some_and(|stack| {
                    stack.count > 0 && stack.item.as_ref() == "minecraft:leather_boots"
                }),
            main_hand: Some(
                self.extra
                    .main_hand_stack()
                    .map_or(CollisionItem::EMPTY, |stack| CollisionItem {
                        item: &stack.item,
                        count: stack.count.min(i32::MAX as u32) as i32,
                    }),
            ),
            lava_walker: self.kind.flags.lava_walker,
            ..Default::default()
        }
    }
}
