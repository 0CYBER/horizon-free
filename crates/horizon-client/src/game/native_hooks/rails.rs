use super::*;

impl GameLevel<'_> {
    /// `IBaseRailBlockExtension.getRailDirection`: `shape` starts at the rail's `shape`
    /// property, and the parts of `state` may change it. `cart` is the minecart on it, if any.
    #[inline(always)]
    pub(in crate::game) fn native_rail_direction(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        cart: Option<EntityRef>,
        shape: &mut RailShape,
    ) {
        self.fire_rail_direction(dimension, state, pos, cart, shape);
    }

    /// `IBaseRailBlockExtension.canMakeSlopes`: `can` starts with vanilla's answer.
    #[inline(always)]
    pub(in crate::game) fn native_rail_can_make_slopes(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        can: &mut bool,
    ) {
        self.fire_rail_can_make_slopes(dimension, state, pos, can);
    }

    /// `IBaseRailBlockExtension.isFlexibleRail`: `flexible` starts with vanilla's answer.
    #[inline(always)]
    pub(in crate::game) fn native_rail_flexible(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        pos: BlockPos,
        flexible: &mut bool,
    ) {
        self.fire_rail_flexible(dimension, state, pos, flexible);
    }

    /// `IBaseRailBlockExtension.isValidRailShape`: `valid` starts with vanilla's answer.
    #[inline(always)]
    pub(in crate::game) fn native_rail_valid_shape(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        shape: RailShape,
        valid: &mut bool,
    ) {
        self.fire_rail_valid_shape(dimension, state, shape, valid);
    }
}

/// A world rail shape as the API names it.
fn api_shape(shape: horizon_world::level::RailShape) -> RailShape {
    RailShape::from_name(shape.name()).unwrap_or(RailShape::NorthSouth)
}

impl GameLevel<'_> {
    /// `LevelAccess::has_rail_hook`: a part listens to one of the rail questions.
    pub(in crate::game) fn rail_hooked(&self) -> bool {
        [
            mod_api::HookId::RailDirection,
            mod_api::HookId::RailCanMakeSlopes,
            mod_api::HookId::RailFlexible,
            mod_api::HookId::RailValidShape,
        ]
        .into_iter()
        .any(mod_api::__private::has_scoped_hook)
    }

    /// `LevelAccess::adjust_rail_direction`: the parts of `state` answer, with no cart.
    pub(in crate::game) fn part_rail_direction(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        vanilla: horizon_world::level::RailShape,
    ) -> horizon_world::level::RailShape {
        let mut shape = api_shape(vanilla);
        self.native_rail_direction(self.dimension, state, pos, None, &mut shape);
        horizon_world::level::RailShape::parse(shape.name())
    }

    /// `LevelAccess::adjust_rail_can_make_slopes`: the parts of `state` answer on an
    /// authoritative server, and `vanilla` stands elsewhere.
    pub(in crate::game) fn part_rail_can_make_slopes(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        vanilla: bool,
    ) -> bool {
        let mut can = vanilla;
        if self.native_authoritative {
            self.native_rail_can_make_slopes(self.dimension, state, pos, &mut can);
        }
        can
    }

    /// `LevelAccess::adjust_rail_flexible`, as `part_rail_can_make_slopes`.
    pub(in crate::game) fn part_rail_flexible(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        vanilla: bool,
    ) -> bool {
        let mut flexible = vanilla;
        if self.native_authoritative {
            self.native_rail_flexible(self.dimension, state, pos, &mut flexible);
        }
        flexible
    }

    /// `LevelAccess::adjust_rail_valid_shape`, as `part_rail_can_make_slopes`.
    pub(in crate::game) fn part_rail_valid_shape(
        &self,
        state: BlockStateId,
        shape: horizon_world::level::RailShape,
        vanilla: bool,
    ) -> bool {
        let mut valid = vanilla;
        if self.native_authoritative {
            self.native_rail_valid_shape(self.dimension, state, api_shape(shape), &mut valid);
        }
        valid
    }
}
