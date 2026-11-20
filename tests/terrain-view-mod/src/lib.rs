//! A server-only consumer using public SDK calls.
use mod_api::{
    Flow, Hooks, PlayerLoggedInEvent, PlayerLoginEvent, ServerHost, ServerStartedEvent,
    TerrainBand, TerrainPattern, TerrainView,
};

#[derive(Default)]
struct Fixture;

#[mod_api::horizon_mod]
impl Hooks for Fixture {
    fn server_started(&self, host: &mut dyn ServerHost, _: &mut ServerStartedEvent) {
        let chest = host.block_state_id("minecraft:chest").expect("chest state");
        let stone = host.block_state_id("minecraft:stone").expect("stone state");
        let rules = TerrainView::new(
            vec![TerrainBand::new(
                -64,
                320,
                vec![chest],
                vec![stone],
                TerrainPattern::Block(0),
            )],
            vec![stone],
            true,
            2,
        );
        host.server_define_terrain_view(1, &rules)
            .expect("define terrain view");
    }

    fn player_login(&self, host: &mut dyn ServerHost, event: &mut PlayerLoginEvent<'_>) -> Flow {
        host.player_set_terrain_view(event.player, "minecraft:overworld", Some(1))
            .expect("select terrain view");
        Flow::Continue
    }

    fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        let player = host.player(event.player).expect("connected player");
        if player.permission_level(host).unwrap_or(0) >= 2 {
            host.player_set_terrain_view(event.player, "minecraft:overworld", None)
                .expect("release terrain view");
        }
    }
}
