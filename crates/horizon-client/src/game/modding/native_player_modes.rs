//! A player's permission level for native server mods, through a
//! `PlayerRef` (docs/modding/src/how-to/native-mods.md): the `Player` reads the handle's game
//! mode reads don't give, and what the player may do to blocks and other
//! players (`mayUseItemAt`, `canHarmPlayer`). They return None for a stale
//! handle or a player who isn't online here.
use super::Game;
use super::native_entity_refs::block_face_direction;
use super::native_server::PlayerRead;
use horizon_client::items::ItemStack;
use mod_api::{BlockFace, GameMode, PlayerRef};

impl Game<'_> {
    /// The player's permission level, 0 to 4 (`Player.permissions()`,
    /// `PlayerList.getProfilePermissions`): an op's level, the
    /// single-player owner's 4 with commands allowed. A displaced connected
    /// player keeps their permission level even while their body is unavailable.
    /// None for a stale or unavailable player.
    pub(super) fn player_permission_level(&self, player: PlayerRef) -> Option<u8> {
        let read = self
            .player_handle_read(player)
            .or_else(|| self.online_player_record(player).map(PlayerRead::Guest))?;
        Some(match read {
            PlayerRead::Guest(g) => g.permission,
            PlayerRead::Acting { .. } if self.lan_acting.is_some() => self.ui.registries.permission,
            PlayerRead::Acting { .. } if self.allow_commands() => 4,
            PlayerRead::Acting { .. } => 0,
        })
    }
    /// `Player.hasInfiniteMaterials`: the `instabuild` ability, which
    /// creative mode gives.
    pub(super) fn player_has_infinite_materials(&self, player: PlayerRef) -> Option<bool> {
        self.player_is_creative(player)
    }
    /// `Player.canUseGameMasterBlocks`: creative and a permission level of
    /// at least 2, as command blocks, structure blocks and jigsaws ask.
    pub(super) fn player_can_use_game_master_blocks(&self, player: PlayerRef) -> Option<bool> {
        let creative = self.player_is_creative(player)?;
        Some(creative && self.player_permission_level(player)? >= 2)
    }
    /// `ServerPlayerGameMode.getPreviousGameModeForPlayer`: the mode the
    /// player last left (`previousPlayerGameType`, saved with the player), or
    /// `Some(None)` when they never changed mode. None for a stale or
    /// unavailable player.
    pub(super) fn player_previous_game_mode(&self, player: PlayerRef) -> Option<Option<GameMode>> {
        Some(match self.player_handle_read(player)? {
            // The acting player's own: a guest's turn swaps theirs in.
            PlayerRead::Acting { .. } => self.previous_mode,
            PlayerRead::Guest(guest) => guest.previous_game_mode.and_then(GameMode::from_id),
        })
    }
    /// `Player.isSecondaryUseActive`: whether the player uses items and
    /// blocks as with sneak held, as vanilla reads it from the shift-key
    /// flag (the same read as `EntityRef::is_shift_key_down`). None for a
    /// stale or unavailable player.
    pub(super) fn player_is_secondary_use_active(&self, player: PlayerRef) -> Option<bool> {
        self.entity_is_shift_key_down(player.entity())
    }
    /// `Player.mayUseItemAt(pos, face, stack)`: the game's own check for an
    /// item used on a block (`Game::may_use_item_at`), with the player's
    /// `mayBuild` as the handle reads it. None for a stale or unavailable
    /// player, and for a guest in another level than the one in place, whose
    /// blocks it can't test.
    pub(super) fn player_may_use_item_at(
        &self,
        player: PlayerRef,
        pos: [i32; 3],
        face: BlockFace,
        stack: &ItemStack,
    ) -> Option<bool> {
        if let PlayerRead::Guest(guest) = self.player_handle_read(player)?
            && guest.dimension != self.dim.id()
        {
            return None;
        }
        let may_build = self.player_may_build(player)?;
        Some(self.may_use_item_at(may_build, pos, block_face_direction(face), stack))
    }
    /// `ServerPlayer.canHarmPlayer(other)`: the `pvp` rule and the two
    /// players' teams, as an attack, an arrow or a trident asks them
    /// (`player_may_harm`). None when either handle is stale or unavailable.
    pub(super) fn player_can_harm(&self, player: PlayerRef, other: PlayerRef) -> Option<bool> {
        self.player_handle_read(player)?;
        self.player_handle_read(other)?;
        let attacker = self.native_target_team(self.native_entity_target(player.entity())?)?;
        let victim = self.native_target_team(self.native_entity_target(other.entity())?)?;
        Some(self.player_may_harm(victim, attacker))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Game;
    use super::super::api_values::test_snapshot;
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::{chunk::Chunk, items::ItemStack};
    use horizon_core::nbt::Tag;
    use mod_api::__private::game::*;
    use mod_api::BlockFace;
    use std::sync::Arc;

    fn set_mode(game: &mut Game<'_>, uuid: u128, mode: i32) {
        game.lan.as_mut().unwrap().hub.guests.update_all(|g| {
            if g.uuid == uuid {
                g.game_mode = mode;
                g.abilities.update_for_mode(mode);
            }
        });
    }

    /// A stack that, in adventure mode, places on `block` only.
    fn placing_on(block: &str) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack.data_mut().set_other(
            "minecraft:can_place_on",
            Some(Tag::compound([("blocks", Tag::String(block.into()))])),
        );
        stack
    }

    /// `Player.mayUseItemAt`: builders always may, adventure and spectator
    /// players only where the stack's `can_place_on` matches the block
    /// clicked, which lies opposite the face from the placed position.
    #[test]
    #[ignore = "daily"]
    fn item_use_follows_the_mode_the_stack_and_the_clicked_block() {
        with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(5, 63, 5, stone).unwrap();
            set_mode(game, 13, 2);
            let (builder, adventurer) = (game.player(12).unwrap(), game.player(13).unwrap());
            let plain = test_snapshot(&ItemStack::new("minecraft:stone", 1));
            let on_stone = test_snapshot(&placing_on("minecraft:stone"));
            let on_dirt = test_snapshot(&placing_on("minecraft:dirt"));
            let up = |player: mod_api::PlayerRef, game: &Game<'_>, stack| {
                player.may_use_item_at(game, [5, 64, 5], BlockFace::Up, stack)
            };

            // Survival builds with anything.
            assert_eq!(up(builder, game, &plain), Some(true));
            // Adventure needs the component, naming the block clicked (the stone below).
            assert_eq!(up(adventurer, game, &plain), Some(false));
            assert_eq!(up(adventurer, game, &on_dirt), Some(false));
            assert_eq!(up(adventurer, game, &on_stone), Some(true));
            // The same placed position clicked on a side face lies against air.
            assert_eq!(
                adventurer.may_use_item_at(game, [5, 64, 5], BlockFace::North, &on_stone),
                Some(false)
            );
            // The acting player in their own turn answers alike.
            let acting = game
                .as_guest(13, |game| up(adventurer, game, &on_stone))
                .unwrap();
            assert_eq!(acting, Some(true));
            // A spectator may not build either, so the same rule applies.
            set_mode(game, 13, 3);
            assert_eq!(up(adventurer, game, &plain), Some(false));
            assert_eq!(up(adventurer, game, &on_stone), Some(true));
        });
    }

    /// `ServerPlayer.canHarmPlayer`: the `pvp` rule first, then the teams
    /// (allies hurt each other only with friendly fire on).
    #[test]
    #[ignore = "daily"]
    fn one_player_may_harm_another_by_the_pvp_rule_and_their_teams() {
        use horizon_client::gamerules::Value;
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert_eq!(a.can_harm(&*game, b), Some(true), "no teams, pvp on");

            let board = &mut game.score.board;
            board.add_team("red").allow_friendly_fire = false;
            board.add_to_team("guest12", "red");
            board.add_to_team("guest13", "red");
            assert_eq!(
                a.can_harm(&*game, b),
                Some(false),
                "allies, no friendly fire"
            );
            assert_eq!(b.can_harm(&*game, a), Some(false));

            game.score.board.add_team("red").allow_friendly_fire = true;
            assert_eq!(
                a.can_harm(&*game, b),
                Some(true),
                "allies with friendly fire"
            );

            game.ui.rules.set("pvp", Value::Bool(false));
            assert_eq!(
                a.can_harm(&*game, b),
                Some(false),
                "pvp off beats the teams"
            );

            // During the attacker's own turn the same answer comes from their state in place.
            let acting = game.as_guest(12, |game| a.can_harm(&*game, b)).unwrap();
            assert_eq!(acting, Some(false));
            game.ui.rules.set("pvp", Value::Bool(true));
            let acting = game.as_guest(12, |game| a.can_harm(&*game, b)).unwrap();
            assert_eq!(acting, Some(true));

            // A handle that names nobody here reads nothing.
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(a.fields().entity.epoch(), a.fields().entity.id() + 100),
                a.uuid(),
            );
            assert_eq!(stale.can_harm(&*game, b), None);
            assert_eq!(a.can_harm(&*game, stale), None);
        });
    }
}
