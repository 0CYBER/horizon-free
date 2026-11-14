//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `client_view` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait ClientViewHost {
    /// Side: both.
    /// Collision boxes in block-local coordinates with an empty collision context.
    #[access(WorldOnly)]
    fn block_collision(&self, pos: [i32; 3]) -> Vec<[f64; 6]>;
}
/// The client host's methods of the `client_view` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ClientViewClientHost {
    /// Side: client.
    /// Read effective HUD visibility independently of first-person hands.
    #[access(ClientSetup)]
    fn hud_mode(&self) -> HudMode;
    /// Side: client.
    /// Select HUD and hand visibility, including after vanilla F1 input handling.
    /// Other first-person hand restrictions and render cancellation still apply.
    #[access(ClientSetup)]
    fn set_hud_mode(&mut self, mode: HudMode);
    /// Side: client.
    /// The window's camera type (`Options.getCameraType`), including its latest pending request.
    #[access(ClientSetup)]
    fn camera_type(&self) -> CameraType;
    /// Side: client.
    /// Request the window's camera type without cycling, also from shared-host callbacks.
    /// The last request wins and [`Self::camera_type`] reports it immediately. Requests before
    /// or during `camera_pose` apply to that frame's camera, FOV and spectator post effects;
    /// later requests apply at the next frame's camera extraction. Explicit pose overrides
    /// remain unchanged when vanilla placement is refreshed. No cycle hook is emitted.
    #[access(WorldOnly)]
    fn set_camera_type(&self, camera_type: CameraType);
    /// Side: client.
    /// The applied client pause state, not whether a pause screen is open.
    #[access(ClientSetup)]
    fn is_paused(&self) -> bool;
    /// Side: client.
    /// The clock of the world this window shows, for client mods: its game
    /// time and time of day as the server last sent them
    /// (`ClientboundSetTimePacket`), advanced each tick between packets; none
    /// on the title screen.
    #[access(ClientSetup)]
    fn clock(&self) -> Option<Clock>;
    /// Side: client.
    /// The first block along the view within `distance` (at most 1024), past the loaded chunks over the distant terrain.
    #[access(ClientSetup)]
    fn raycast(&self, distance: f32) -> Option<Hit>;
    /// Side: client.
    /// The first block along the view within `RAYCAST_MAX`, and its distance.
    #[access(ClientSetup)]
    fn view_hit(&self) -> Option<(f32, Hit)>;
    /// Side: client.
    /// The most recently extracted world frame's camera, if one exists.
    /// During GUI rendering this is the current frame, including bob/spin and FOV.
    #[access(ClientSetup)]
    fn camera(&self) -> Option<Camera>;
    /// Side: client.
    /// World point `at` projected on this frame's screen (`GameRenderer.projectPointToScreen`):
    /// normalised device x and y, -1 to 1 across the screen (x to the right, y up), and depth z
    /// (reversed: nearer is larger). The view is the camera's rotation without bob or spin, as
    /// vanilla's. Unlike vanilla, None for a point behind the camera or a non-finite `at`; also None
    /// outside a world, before extraction, and between render frames. Supported during GUI,
    /// screen and world drawing after camera extraction. In GUI pixels of a [`DrawContext`] of size
    /// `[w, h]`: `((x + 1) / 2 * w, (1 - y) / 2 * h)`.
    #[access(ClientSetup)]
    fn project(&self, at: [f64; 3]) -> Option<[f32; 3]>;
    /// Side: client.
    /// The local player's item use as the client sees it: the hand, ticks used and ticks left
    /// (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
    /// None when the player uses no item or no client world is open. Sends nothing.
    #[access(ClientSetup)]
    fn local_item_use(&self) -> Option<ItemUse>;
    /// Side: client.
    /// Turns the local player's view to `yaw` and `pitch` (degrees), as `setYRot` and `setXRot`
    /// store them: yaw as given (not wrapped; the server wraps it), pitch `% 360` then clamped to
    /// -90..90. The previous-frame angles move by the same amount, as `Entity.turn` does for the
    /// mouse, so the view doesn't jump for a frame. Sends no packet: the game's own player sync of
    /// this client tick sends the look in its one move packet, only while the camera is the player
    /// (the rotation-only packet when riding); a call after that step goes out the next tick.
    /// `NotFinite`, changing nothing, for a NaN or infinite value; `NoWorld` outside a client world.
    #[access(WorldOnly)]
    fn set_local_rotation(&mut self, yaw: f32, pitch: f32) -> Result<(), LocalRotationError>;
    /// Side: client.
    /// The entity the view follows (`Minecraft.getCameraEntity`): the entity a server attached
    /// the camera to, else the local player. None outside a client world.
    #[access(ClientSetup)]
    fn camera_entity(&self) -> Option<EntityRef>;
    /// Side: client.
    /// Each column's height by `heightmap` in the world this window shows
    /// (`LevelReader.getHeight(type, x, z)` on a client level): one above the highest block that
    /// counts on that map, from the blocks the server has sent, none for a column whose chunk this
    /// client does not hold. `Err`, reading nothing, for no client world (the title screen) or more
    /// than 65,536 columns (`MAX_COLUMNS`).
    #[access(WorldOnly)]
    fn client_surface_heights(
        &self,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
    ) -> Result<Vec<Option<i32>>, String>;
    /// Side: client.
    /// Points the view at `entity` (`Minecraft.setCameraEntity`), as a server's camera packet
    /// does; the local player's own ref puts it back. True when the view now follows it; false,
    /// with nothing changed, for a stale ref, an entity this window is not shown, or one the
    /// camera cannot follow (only the local player and mobs). While the view is off the player,
    /// the player sends no moves (`LocalPlayer.isControlledCamera`). When the camera entity is
    /// removed, the view returns to the local player. A mod can also release the view by calling
    /// this with the player's ref; this client-only selection does not notify the server.
    #[access(WorldOnly)]
    fn set_camera_entity(&mut self, entity: EntityRef) -> bool;
    /// Side: client.
    /// The ordinary block-or-entity pick (Minecraft.hitResult), using the
    /// game's targeting ray, reach and occlusion. None without a client world
    /// or on a miss. Entity hits carry this client level's epoch. Unlike
    /// native_view_hit, this never traces distant terrain.
    #[access(ClientSetup)]
    fn crosshair_target(&self) -> Option<CrosshairTarget>;
    /// Side: client.
    /// Current targeting controls. Stale entity overrides are omitted. Reach comes from the
    /// player's vanilla interaction-range attributes, including server and equipment modifiers:
    /// this is the read for the player's block and entity reach.
    #[access(WorldOnly)]
    fn targeting(&self) -> ClientTargeting;
    /// Side: client.
    /// Set the window's targeting controls for subsequent picks. False, without changing any
    /// control, outside a client world or for a stale/self entity. Entity range, item attack
    /// range and (unless enabled here) occlusion are checked again when attacking. This does
    /// not change server reach checks or attack cooldowns. None clears an entity override.
    #[access(WorldOnly)]
    fn set_targeting(&self, targeting: ClientTargeting) -> bool;
}
