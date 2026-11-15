//! Minecart handles for native server mods.
use super::{
    Game,
    commands::Target,
    minecarts::{CartKind, Minecart, display_offset},
};
use horizon_world::registry::BlockStateId;
use mod_api::{EntityRef, MinecartKind, MinecartRef};

impl Game<'_> {
    fn native_minecart_index(&self, cart: MinecartRef) -> Option<usize> {
        let Target::Cart(id) = self.native_entity_target(cart.entity())? else {
            return None;
        };
        self.carts.iter().position(|c| c.id == id)
    }

    fn native_minecart(&self, cart: MinecartRef) -> Option<&Minecart> {
        self.carts.get(self.native_minecart_index(cart)?)
    }

    /// Edits a cart, of one kind if `kind` is given. False for another kind
    /// or a handle that no longer resolves, including while held out of storage.
    fn native_minecart_edit(
        &mut self,
        cart: MinecartRef,
        kind: Option<CartKind>,
        edit: impl FnOnce(&mut Minecart),
    ) -> bool {
        let Some(index) = self.native_minecart_index(cart) else {
            return false;
        };
        if kind.is_some_and(|kind| self.carts[index].kind != kind) {
            return false;
        }
        self.carts.update(index, edit);
        true
    }

    /// Narrows an entity to a minecart of any kind, or None for another kind of
    /// entity, a stale handle, a client, or an entity held out of storage.
    pub(super) fn minecart(&self, entity: EntityRef) -> Option<MinecartRef> {
        let cart = MinecartRef::new(entity);
        self.native_minecart_index(cart).map(|_| cart)
    }

    /// Which `AbstractMinecart` subclass the cart is.
    pub(super) fn minecart_kind(&self, cart: MinecartRef) -> Option<MinecartKind> {
        Some(match self.native_minecart(cart)?.kind {
            CartKind::Rideable => MinecartKind::Rideable,
            CartKind::Chest => MinecartKind::Chest,
            CartKind::Furnace => MinecartKind::Furnace,
            CartKind::Tnt => MinecartKind::Tnt,
            CartKind::Hopper => MinecartKind::Hopper,
            CartKind::CommandBlock => MinecartKind::CommandBlock,
            CartKind::Spawner => MinecartKind::Spawner,
        })
    }

    /// `AbstractMinecart.getMaxSpeed`, in blocks per tick: the old behaviour's
    /// 0.4 (0.2 in water), or with the `minecart_improvements` experiment the
    /// `max_minecart_speed` rule over 20 (halved in water). A furnace minecart
    /// takes half of that, three quarters in water (`MinecartFurnace`).
    pub(super) fn minecart_max_speed(&self, cart: MinecartRef) -> Option<f64> {
        Some(self.cart_max_speed(self.native_minecart(cart)?))
    }

    /// `AbstractMinecart.isOnRails`.
    pub(super) fn minecart_is_on_rails(&self, cart: MinecartRef) -> Option<bool> {
        Some(self.native_minecart(cart)?.on_rails)
    }

    /// `AbstractMinecart.isFlipped`.
    pub(super) fn minecart_flipped(&self, cart: MinecartRef) -> Option<bool> {
        Some(self.native_minecart(cart)?.flipped)
    }

    /// `AbstractMinecart.setFlipped`; saved as `FlippedRotation`, not synced.
    pub(super) fn minecart_set_flipped(&mut self, cart: MinecartRef, flipped: bool) -> bool {
        self.native_minecart_edit(cart, None, |c| c.flipped = flipped)
    }

    /// `VehicleEntity.getDamage`, the wobble damage (`DATA_ID_DAMAGE`).
    pub(super) fn minecart_damage(&self, cart: MinecartRef) -> Option<f32> {
        Some(self.native_minecart(cart)?.damage)
    }

    /// `VehicleEntity.setDamage`: synced to clients. The tick counts it down
    /// only while positive.
    pub(super) fn minecart_set_damage(&mut self, cart: MinecartRef, damage: f32) -> bool {
        self.native_minecart_edit(cart, None, |c| c.damage = damage)
    }

    /// `VehicleEntity.getHurtTime`, the wobble ticks (`DATA_ID_HURT`).
    pub(super) fn minecart_hurt_time(&self, cart: MinecartRef) -> Option<i32> {
        Some(self.native_minecart(cart)?.hurt_time)
    }

    /// `VehicleEntity.setHurtTime`: synced to clients.
    pub(super) fn minecart_set_hurt_time(&mut self, cart: MinecartRef, ticks: i32) -> bool {
        self.native_minecart_edit(cart, None, |c| c.hurt_time = ticks)
    }

    /// `AbstractMinecart.customDisplayBlockState`: the block the cart shows in
    /// place of its kind's default, None (inner) when it has none.
    pub(super) fn minecart_custom_display_block(
        &self,
        cart: MinecartRef,
    ) -> Option<Option<BlockStateId>> {
        let cart = self.native_minecart(cart)?;
        Some(
            cart.custom_display
                .as_ref()
                .and_then(|t| self.state_from_nbt(t)),
        )
    }

    /// Sets the block the cart shows, or with None its kind's default again:
    /// saved as `DisplayState` and synced to clients. False for a state the
    /// registry doesn't know, as the loader drops one that does not decode.
    pub(super) fn minecart_set_custom_display_block(
        &mut self,
        cart: MinecartRef,
        state: Option<BlockStateId>,
    ) -> bool {
        if state.is_some_and(|s| self.pack.assets.registry.state(s).is_none()) {
            return false;
        }
        let display = state.map(|s| self.state_to_nbt(s));
        self.native_minecart_edit(cart, None, |c| c.custom_display = display)
    }

    /// `AbstractMinecart.getDisplayOffset`, in sixteenths of a block: the set
    /// offset, else the kind's default.
    pub(super) fn minecart_display_offset(&self, cart: MinecartRef) -> Option<i32> {
        Some(display_offset(self.native_minecart(cart)?) as i32)
    }

    /// `AbstractMinecart.setDisplayOffset`, in sixteenths of a block: saved as
    /// `DisplayOffset` (only off the kind's default) and synced to clients.
    pub(super) fn minecart_set_display_offset(&mut self, cart: MinecartRef, offset: i32) -> bool {
        self.native_minecart_edit(cart, None, |c| c.custom_offset = Some(offset))
    }

    /// `MinecartFurnace`'s fuel ticks; None for another kind of cart.
    pub(super) fn minecart_fuel(&self, cart: MinecartRef) -> Option<i32> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Furnace).then_some(cart.fuel)
    }

    /// Sets a furnace minecart's fuel ticks, any int as vanilla's field holds
    /// it (saved as a short, the low 16 bits). False for another kind of cart.
    /// The lit flag (`DATA_ID_FUEL`) follows with fuel above zero.
    pub(super) fn minecart_set_fuel(&mut self, cart: MinecartRef, fuel: i32) -> bool {
        self.native_minecart_edit(cart, Some(CartKind::Furnace), |c| c.fuel = fuel)
    }

    /// A furnace minecart's push (`MinecartFurnace.push`): the way its fuel drives it on rails,
    /// x and z; its y part stays between ticks and sets the push's length. None for another
    /// kind of cart.
    pub(super) fn minecart_push(&self, cart: MinecartRef) -> Option<[f64; 3]> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Furnace).then_some([cart.push.x, cart.push.y, cart.push.z])
    }

    /// Sets a furnace minecart's push, as a player's click with fuel sets it toward them
    /// (`MinecartFurnace.interact`): a cart a mod drives runs that way while it has fuel. False
    /// for another kind of cart or a push that isn't finite.
    pub(super) fn minecart_set_push(&mut self, cart: MinecartRef, push: [f64; 3]) -> bool {
        if !push.iter().all(|v| v.is_finite()) {
            return false;
        }
        let [x, y, z] = push;
        self.native_minecart_edit(cart, Some(CartKind::Furnace), |c| {
            c.push = horizon_core::geometry::Vec3::new(x, y, z)
        })
    }

    /// `MinecartTNT`'s fuse ticks, -1 when it is not primed; None for another
    /// kind of cart.
    pub(super) fn minecart_fuse(&self, cart: MinecartRef) -> Option<i32> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Tnt).then_some(cart.fuse)
    }

    /// `MinecartTNT.primeFuse(null)`: an 80 tick fuse, the primed sound and
    /// entity event 70 for clients. False for another kind of cart, or with
    /// the `tnt_explodes` rule off.
    pub(super) fn minecart_prime_fuse(&mut self, cart: MinecartRef) -> bool {
        let Some(index) = self.native_minecart_index(cart) else {
            return false;
        };
        if self.carts[index].kind != CartKind::Tnt
            || !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES)
        {
            return false;
        }
        let mut primed = self.carts[index].clone();
        self.prime_cart(&mut primed);
        self.native_minecart_edit(cart, Some(CartKind::Tnt), |c| c.fuse = primed.fuse)
    }

    /// `MinecartHopper.isEnabled`; None for another kind of cart.
    pub(super) fn minecart_hopper_enabled(&self, cart: MinecartRef) -> Option<bool> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::Hopper).then_some(cart.enabled)
    }

    /// `MinecartHopper.setEnabled`: saved as `Enabled`, not synced.
    pub(super) fn minecart_set_hopper_enabled(&mut self, cart: MinecartRef, enabled: bool) -> bool {
        self.native_minecart_edit(cart, Some(CartKind::Hopper), |c| c.enabled = enabled)
    }

    /// A command block minecart's command (`MinecartCommandBlock.getCommandBlock().getCommand()`).
    /// None for another kind of cart.
    pub(super) fn minecart_command(&self, cart: MinecartRef) -> Option<String> {
        let cart = self.native_minecart(cart)?;
        (cart.kind == CartKind::CommandBlock).then(|| cart.command.command.clone())
    }

    /// Sets a command block minecart's command (`BaseCommandBlock.setCommand`, which resets the
    /// success count), then `MinecartCommandBase.onUpdated`, as the edit screen's save does: the
    /// synced command and last output follow, so clients' editors show the new command. The
    /// output and its tracking are kept, and no chat note is sent. An activator rail then runs
    /// the new command. False for another kind of cart.
    pub(super) fn minecart_set_command(&mut self, cart: MinecartRef, command: &str) -> bool {
        let Some(id) = self
            .native_minecart(cart)
            .filter(|c| c.kind == CartKind::CommandBlock)
            .map(|c| c.id)
        else {
            return false;
        };
        self.native_minecart_edit(cart, None, |c| c.command.set_command(command));
        self.cart_command_updated(id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::entities::ArmorStand;
    use horizon_core::geometry::Vec3;
    use horizon_core::nbt::Tag;
    use mod_api::WorldEpoch;

    fn spawn(game: &mut Game<'_>, id: u32, kind: CartKind) -> MinecartRef {
        game.carts.push(Minecart::new(id, kind, Vec3::ZERO));
        game.minecart(game.entity(id).unwrap()).unwrap()
    }

    #[test]
    fn furnace_fuel_adds_with_java_int_arithmetic() {
        assert_eq!(super::super::minecarts::furnace_fuel_after(0), Some(3600));
        assert_eq!(
            super::super::minecarts::furnace_fuel_after(28_400),
            Some(32_000)
        );
        assert_eq!(super::super::minecarts::furnace_fuel_after(28_401), None);
        assert_eq!(
            super::super::minecarts::furnace_fuel_after(i32::MAX),
            Some(i32::MIN + 3599)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_furnace_carts_push_is_read_and_set_through_its_handle() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let furnace = spawn(game, 50, CartKind::Furnace);
            let chest = spawn(game, 51, CartKind::Chest);
            assert_eq!(game.minecart_push(furnace), Some([0.0, 0.0, 0.0]));
            assert!(game.minecart_set_push(furnace, [0.5, 0.0, -0.5]));
            assert_eq!(game.minecart_push(furnace), Some([0.5, 0.0, -0.5]));
            assert_eq!(game.carts[0].push, Vec3::new(0.5, 0.0, -0.5));
            // Not a furnace cart, or a push that isn't finite: refused.
            assert_eq!(game.minecart_push(chest), None);
            assert!(!game.minecart_set_push(chest, [0.5, 0.0, 0.0]));
            assert!(!game.minecart_set_push(furnace, [f64::NAN, 0.0, 0.0]));
            assert_eq!(game.minecart_push(furnace), Some([0.5, 0.0, -0.5]));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_carts_display_block_and_offset_round_trip_through_its_handle() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let cart = spawn(game, 60, CartKind::Chest);
            assert_eq!(cart.custom_display_block(game), Some(None));
            assert_eq!(cart.display_offset(game), Some(8));
            let lit = game
                .block_state_id("minecraft:furnace[facing=east,lit=true]")
                .unwrap()
                .try_into_api()
                .unwrap();
            assert!(cart.set_custom_display_block(game, Some(lit)));
            assert!(cart.set_display_offset(game, 3));
            assert_eq!(cart.custom_display_block(game), Some(Some(lit)));
            assert_eq!(cart.display_offset(game), Some(3));

            // Saved and loaded back as the same state and offset.
            let saved = game.carts[0].to_nbt([0; 4], None);
            let mut loaded = Minecart::from_nbt(&saved, 61, CartKind::Chest).unwrap();
            super::super::minecarts::forget_unreadable_display(&mut loaded, |t| {
                game.state_from_nbt(t).is_some()
            });
            game.carts.push(loaded);
            let loaded = game.minecart(game.entity(61).unwrap()).unwrap();
            assert_eq!(loaded.custom_display_block(game), Some(Some(lit)));
            assert_eq!(loaded.display_offset(game), Some(3));

            // A state the registry doesn't know is refused; None clears.
            let invalid = mod_api::BlockStateRecord::new(lit.block(), u16::MAX).into();
            assert!(!cart.set_custom_display_block(game, Some(invalid)));
            assert_eq!(cart.custom_display_block(game), Some(Some(lit)));
            assert!(cart.set_custom_display_block(game, None));
            assert_eq!(cart.custom_display_block(game), Some(None));
            assert!(
                game.carts[0]
                    .to_nbt([0; 4], None)
                    .get("DisplayState")
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn minecart_handles_narrow_read_and_write_by_kind() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kinds = [
                (CartKind::Rideable, MinecartKind::Rideable),
                (CartKind::Chest, MinecartKind::Chest),
                (CartKind::Furnace, MinecartKind::Furnace),
                (CartKind::Tnt, MinecartKind::Tnt),
                (CartKind::Hopper, MinecartKind::Hopper),
                (CartKind::CommandBlock, MinecartKind::CommandBlock),
                (CartKind::Spawner, MinecartKind::Spawner),
            ];
            let carts: Vec<_> = kinds
                .iter()
                .zip(10..)
                .map(|((kind, _), id)| spawn(game, id, *kind))
                .collect();
            for (cart, (_, expected)) in carts.iter().zip(kinds) {
                assert_eq!(cart.kind(game), Some(expected));
            }
            let [rideable, _, furnace, tnt, hopper, ..] = carts[..] else {
                unreachable!()
            };
            game.armor_stands.push(ArmorStand::new(99, Vec3::ZERO, 0.0));
            assert!(game.minecart(game.entity(99).unwrap()).is_none());

            // Max speed: old behaviour, water, furnace, then the experiment's rule.
            assert_eq!(rideable.max_speed(game), Some(0.4));
            assert_eq!(furnace.max_speed(game), Some(0.2));
            game.carts.update(2, |c| c.in_water = true);
            assert_eq!(furnace.max_speed(game), Some(0.2 * 0.75));
            game.carts.update(2, |c| c.in_water = false);
            let mut features = game.level.enabled_features.to_vec();
            features.push("minecraft:minecart_improvements".into());
            game.level.enabled_features = features.into();
            game.ui.rules =
                horizon_client::gamerules::GameRules::with_features(&game.level.enabled_features);
            assert_eq!(rideable.max_speed(game), Some(8.0 / 20.0));
            assert_eq!(furnace.max_speed(game), Some(8.0 / 20.0 * 0.5));
            game.carts.update(0, |c| c.in_water = true);
            assert_eq!(rideable.max_speed(game), Some(8.0 / 20.0 * 0.5));

            assert_eq!(rideable.is_on_rails(game), Some(false));
            game.carts.update(0, |c| c.on_rails = true);
            assert_eq!(rideable.is_on_rails(game), Some(true));

            assert!(rideable.set_flipped(game, true));
            assert_eq!(rideable.flipped(game), Some(true));
            assert!(rideable.set_damage(game, 12.5));
            assert!(rideable.set_hurt_time(game, 7));
            assert_eq!(
                (rideable.damage(game), rideable.hurt_time(game)),
                (Some(12.5), Some(7))
            );
            let saved = game.carts[0].to_nbt([0; 4], None);
            assert_eq!(saved.get("FlippedRotation"), Some(&Tag::Byte(1)));

            // Furnace fuel, saved and synced as the lit flag.
            assert_eq!(furnace.fuel(game), Some(0));
            assert!(furnace.set_fuel(game, 3600));
            assert_eq!(furnace.fuel(game), Some(3600));
            assert!(!tnt.set_fuel(game, 5));
            assert_eq!(tnt.fuel(game), None);
            let saved = game.carts[2].to_nbt([0; 4], None);
            assert_eq!(saved.get("Fuel"), Some(&Tag::Short(3600)));
            // The field is an int; only the save casts to a short.
            assert!(furnace.set_fuel(game, 40_000));
            assert_eq!(furnace.fuel(game), Some(40_000));
            assert_eq!(
                game.carts[2].to_nbt([0; 4], None).get("Fuel"),
                Some(&Tag::Short(40_000_i32 as i16))
            );
            assert!(furnace.set_fuel(game, i32::MIN));
            assert_eq!(furnace.fuel(game), Some(i32::MIN));
            assert!(furnace.set_fuel(game, 3600));

            // TNT fuse.
            assert_eq!(tnt.fuse(game), Some(-1));
            game.audio.capture = Some(Vec::new());
            assert!(tnt.prime_fuse(game));
            assert_eq!(tnt.fuse(game), Some(80));
            // MinecartTNT.primeFuse broadcasts entity event 70 for clients.
            assert!(
                game.lan_entity_events
                    .iter()
                    .any(|e| (e.0, e.1) == (tnt.entity().id() as i32, 70))
            );
            game.audio.capture = None;
            assert_eq!(
                game.carts[3].to_nbt([0; 4], None).get("fuse"),
                Some(&Tag::Int(80))
            );
            assert!(!furnace.prime_fuse(game));
            game.ui.rules.set(
                "tnt_explodes",
                horizon_client::gamerules::Value::Bool(false),
            );
            game.carts.update(3, |c| c.fuse = -1);
            assert!(!tnt.prime_fuse(game));
            assert_eq!(tnt.fuse(game), Some(-1));

            // The tick counts wobble down only while positive.
            assert!(rideable.set_hurt_time(game, i32::MIN));
            assert!(rideable.set_damage(game, 0.0));
            game.tick_carts();
            assert_eq!(rideable.hurt_time(game), Some(i32::MIN));
            assert!(rideable.set_hurt_time(game, -1));
            assert!(rideable.set_damage(game, -0.5));
            game.tick_carts();
            assert_eq!(
                (rideable.hurt_time(game), rideable.damage(game)),
                (Some(-1), Some(-0.5))
            );
            assert!(rideable.set_hurt_time(game, 3));
            assert!(rideable.set_damage(game, 2.5));
            game.tick_carts();
            assert_eq!(
                (rideable.hurt_time(game), rideable.damage(game)),
                (Some(2), Some(1.5))
            );

            // Hopper.
            assert_eq!(hopper.hopper_enabled(game), Some(true));
            assert!(hopper.set_hopper_enabled(game, false));
            assert_eq!(
                game.carts[4].to_nbt([0; 4], None).get("Enabled"),
                Some(&Tag::Byte(0))
            );
            assert!(!rideable.set_hopper_enabled(game, false));
            assert_eq!(rideable.hopper_enabled(game), None);

            // Command block: the write resets the success count and syncs the command and output
            // for clients (`onUpdated`), and the cart then runs the new command.
            let command = carts[5];
            assert_eq!(command.command(game).as_deref(), Some(""));
            game.carts.update(5, |c| c.command.success_count = 3);
            assert!(command.set_command(game, "gamerule mob_griefing false"));
            assert_eq!(
                command.command(game).as_deref(),
                Some("gamerule mob_griefing false")
            );
            let cart = &game.carts[5];
            assert_eq!(cart.command.success_count, 0);
            assert_eq!(cart.synced_command, "gamerule mob_griefing false");
            assert_eq!(cart.synced_output, cart.command.last_output_string());
            let id = command.entity().id();
            assert!(
                game.ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
            );
            assert!(game.perform_cart_command(id));
            assert!(
                !game
                    .ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING),
                "the new command ran"
            );
            assert!(!rideable.set_command(game, "say hi"));
            assert_eq!(rideable.command(game), None);

            let stale = MinecartRef::new(EntityRef::new(
                WorldEpoch::new(rideable.entity().epoch().get() + 1),
                rideable.entity().id(),
            ));
            assert_eq!(stale.kind(game), None);
            assert!(!stale.set_flipped(game, false));
            assert_eq!(stale.max_speed(game), None);
        });
    }
}
