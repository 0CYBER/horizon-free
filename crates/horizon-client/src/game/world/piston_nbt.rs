use super::Game;
use horizon_core::{nbt::Tag, position::BlockPos};
use horizon_world::level::MovingPiston;

impl Game<'_> {
    /// `PistonMovingBlockEntity.saveAdditional`.
    pub(super) fn moving_piston_to_nbt(&self, pos: BlockPos, piston: &MovingPiston) -> Tag {
        let flag = |b: bool| Tag::Byte(i8::from(b));
        Tag::compound([
            ("id", Tag::String("minecraft:piston".into())),
            ("x", Tag::Int(pos.x)),
            ("y", Tag::Int(pos.y)),
            ("z", Tag::Int(pos.z)),
            ("blockState", self.state_to_nbt(piston.moved_state)),
            // Direction.LEGACY_ID_CODEC: the 3D data value as a byte.
            ("facing", Tag::Byte(piston.direction as i8)),
            ("progress", Tag::Float(piston.progress_o.to_bits())),
            ("extending", flag(piston.extending)),
            ("source", flag(piston.source)),
        ])
    }
}
