//! Upstream prose, retained with fork routing described in the modding book.
pub trait Notes {
    #[notes = "`native_simulate_player_move(uuid, input)` plays one tick of a server player's movement from a bot's keys (forward, strafe, jump, sneak, sprint, yaw, pitch, the last velocity) with the client's own movement code, as `LocalPlayer.aiStep` and `Player.travel` do, and returns the position, velocity and ground it reaches without moving the player; `native_collision_boxes(dimension, aabb)` lists the block collision boxes in a box, as `Level.getBlockCollisions(null, aabb)`. Bundles need \"hooks\": @API:player_bots@."]
    fn player_bots();
}
