#![cfg(test)]
use crate::game::{execution_role::ExecutionRole, lan::world_spawn_sync_tests::host_for};

/// `DedicatedServer.isUnderSpawnProtection` is the dedicated server's. The integrated server
/// thread keeps the launch configuration (`options.dedicated`) too, so it cannot be what turns
/// the protection on.
#[test]
#[ignore = "daily"]
fn spawn_protection_protects_only_on_the_dedicated_role() {
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    game.options.dedicated = Some(Vec::new());
    let _rx = host_for(&mut game);
    game.set_spawn_protection(16);
    // An operator exists, so the guest (uuid 1) is not one and the radius applies to it.
    game.lan.as_mut().unwrap().hub.ops.insert(99, 4);
    for (role, player, operator) in [
        (ExecutionRole::DedicatedServer, Some(16), None),
        (ExecutionRole::IntegratedServer, None, None),
        (ExecutionRole::Client, None, None),
    ] {
        game.options.execution_role = role;
        let host = game.lan.as_ref().unwrap();
        assert_eq!(
            game.spawn_protection_for(host, 1),
            player,
            "{role:?}: a player"
        );
        assert_eq!(
            game.spawn_protection_for(host, 99),
            operator,
            "{role:?}: an operator"
        );
    }
}
