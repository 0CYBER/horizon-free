//! The same listener source runs natively, in wire loopback and in the compiled guest.
use mod_api::{BlockPart, LevelDimension, LevelSide, OnBlockStateChangeEvent};

pub struct NoticePart;
#[mod_api::horizon_part]
impl BlockPart for NoticePart {
    fn on_block_state_change(&self, level: LevelSide<'_>, event: &mut OnBlockStateChangeEvent) {
        let LevelSide::Client(level) = level else {
            panic!("expected the client level")
        };
        assert_eq!(event.dimension, LevelDimension::Overworld);
        assert_eq!(event.pos, mod_api::BlockPos::new(1, 64, 1));
        assert_eq!(event.state, event.old);
        assert_eq!(level.state_block(event.old), Some("minecraft:stone"));
        assert_eq!(level.state_block(event.new), Some("minecraft:air"));
        // Return the field through an actual guest host call, not host-side event writeback.
        assert!(level.is_loaded([i32::from(event.predicted), 64, 1]));
    }
}
pub fn register(r: &mut dyn mod_api::Registrar, cx: &mod_api::RegistrationContext) {
    if cx.mods().get("block_notice_probe").is_none() {
        return;
    }
    let stone = r.block_key("minecraft:stone");
    r.block_key("minecraft:air");
    r.block_part(stone, NoticePart);
}
