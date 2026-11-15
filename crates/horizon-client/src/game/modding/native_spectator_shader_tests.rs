//! `register_entity_spectator_shaders` (arismoko/horizon#889) end to end: a mod maps an entity type to a post
//! chain at start, spectating one puts it on the screen, and the third-person view or the entity
//! leaving takes it off; vanilla's mobs keep their own chains.
use super::Game;
use super::execution_role::ExecutionRole;
use super::native_client_session::tests::logged_client;
use super::native_entity_refs::tests::with_client_game;
use mod_api::{
    EntitySpectatorShader, Hooks, ModEntry, RegisterEntitySpectatorShadersEvent,
    RegistrationContext,
};

struct Vision;

#[mod_api::__subscriptions]
impl Hooks for Vision {
    fn register_entity_spectator_shaders(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterEntitySpectatorShadersEvent<'_>,
    ) {
        event.shaders.push(EntitySpectatorShader {
            entity: "minecraft:cow".into(),
            shader: "example:cow_vision".into(),
        });
        // Vanilla's creeper registered first: this one is skipped.
        event.shaders.push(EntitySpectatorShader {
            entity: "minecraft:creeper".into(),
            shader: "example:pink".into(),
        });
    }
}

/// A remote client in the overworld, with entity `id` of type `kind` added.
fn add_entity(game: &mut Game<'_>, id: i32, kind: &str) {
    use horizon_protocol::packets::play::{self, Clientbound as C};
    let kind = game
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id(kind)
        .unwrap();
    game.handle_packet(C::AddEntity(play::AddEntity {
        id,
        uuid: id as u128,
        kind,
        position: [10.0, 64.0, 12.0],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

#[test]
#[ignore = "daily"]
fn a_mods_entity_type_puts_its_chain_on_while_spectated() {
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    use horizon_protocol::packets::play::{self, Clientbound as C};

    let mods: mod_api::Mods = vec![ModEntry::new("vision", Box::new(Vision))].into();
    mod_platform::testing::with_mods(mods, || {
        super::mods::register_entity_spectator_shaders(ExecutionRole::Client);
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());

            add_entity(game, 31, "minecraft:cow");
            game.remote_set_camera(31);
            assert_eq!(game.spectated_post_effect, Some("example:cow_vision"));
            // The third-person view clears it; back in first person it applies again.
            game.check_entity_post_effect(false);
            assert_eq!(game.spectated_post_effect, None);
            game.check_entity_post_effect(true);
            assert_eq!(game.spectated_post_effect, Some("example:cow_vision"));
            // The entity leaving returns the view to the player, with no chain.
            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![31] }))
                .unwrap();
            assert_eq!(game.spectated_post_effect, None);

            // A vanilla mob keeps its own chain: the mod's registration for it was skipped.
            add_entity(game, 32, "minecraft:creeper");
            game.remote_set_camera(32);
            assert_eq!(game.spectated_post_effect, Some("minecraft:creeper"));
        });
    });
}
