//! The track under a minecart: `BaseRailBlock.isRail` and the shape a cart reads,
//! which the rail's parts may answer (`rail_direction`).
use super::Game;
use super::api_values::TryIntoApi;
use super::minecarts::{Rail, Shape};
use super::native_level_signals::ReadLevel;
use horizon_core::position::BlockPos;
use horizon_world::level::classes::Class;
use mod_api::RailShape;

impl Game<'_> {
    /// The rail at (x, y, z) as `cart` (none when the rail is read for itself) sees it: a block
    /// in `#minecraft:rails` built on `BaseRailBlock` (`BaseRailBlock.isRail`), with the shape
    /// its parts answer (`IBaseRailBlockExtension.getRailDirection`).
    pub(super) fn rail(&self, cart: Option<u32>, x: i32, y: i32, z: i32) -> Option<Rail> {
        let state = self.world.block(x, y, z);
        let name = self.res.facts().get(state).name.clone();
        let tagged = self
            .items()
            .block_tags
            .get("minecraft:rails")
            .is_some_and(|rails| rails.contains(&*name));
        if !tagged || !self.res.behaviours().data(state).is(Class::BaseRail) {
            return None;
        }
        let s = self.pack.assets.registry.state(state)?;
        let text = s.properties.get("shape").map_or("", String::as_str);
        let powered = s.properties.get("powered").is_some_and(|v| v == "true");
        let shape = self.cart_rail_direction(cart, state, BlockPos::new(x, y, z), text);
        Some(Rail {
            name,
            shape,
            powered,
        })
    }

    /// The shape named `text` of the rail `state` at `pos`, after the parts of the state.
    fn cart_rail_direction(
        &self,
        cart: Option<u32>,
        state: horizon_world::registry::BlockStateId,
        pos: BlockPos,
        text: &str,
    ) -> Shape {
        let vanilla = Shape::parse(text);
        if !super::api_values::block_state::block_listens(mod_api::HookId::RailDirection, state) {
            return vanilla;
        }
        let (Some(level), Some(epoch), Ok(api_state)) = (
            self.level(self.dim.id()),
            self.native_lifecycle.epoch(),
            state.try_into_api(),
        ) else {
            return vanilla;
        };
        let read = ReadLevel { game: self, level };
        let side = if self.is_logical_server() {
            mod_api::LevelSide::Server(&read)
        } else {
            mod_api::LevelSide::Client(&read)
        };
        let mut shape = RailShape::from_name(text).unwrap_or(RailShape::NorthSouth);
        let cart = cart.map(|id| mod_api::EntityRef::new(epoch, id));
        mod_api::dispatch_rail_direction(side, self.dim.level(), api_state, pos, cart, &mut shape);
        Shape::parse(shape.name())
    }
}
