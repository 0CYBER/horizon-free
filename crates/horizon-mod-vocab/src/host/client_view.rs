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
    /// Whether F3's debug screen leaves `corner` free this frame for a box `render_gui_post`
    /// places: false for a corner it takes (`DebugScreenOverlay`), both top corners while it
    /// is open (its left and right columns) and a bottom corner while one of its charts is
    /// drawn there. Only F3: read [`Self::hud_mode`] for whether the HUD is drawn at all (F1).
    #[access(ClientSetup)]
    fn native_corner_shown(&self, corner: Corner) -> bool;
    /// Side: client.
    /// Select HUD and hand visibility, including after vanilla F1 input handling.
    /// Other first-person hand restrictions and render cancellation still apply.
    #[access(ClientSetup)]
    fn set_hud_mode(&mut self, mode: HudMode);
    /// Side: client.
    /// Show styled `line` in this client's action bar, replacing the previous overlay
    /// message and restarting its normal 60-tick lifetime. Available while playing,
    /// including from local key callbacks on a remote server; sends nothing to the server.
    /// An invalid component returns [`TextFormError`] without changing the overlay.
    #[access(WorldOnly)]
    fn show_action_bar(&mut self, line: Text) -> Result<(), TextFormError>;
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
    /// The surface of each requested `(x, z)` column, in input order (including repeats).
    /// Uses the same first free Y as `client_surface_heights`; block, biome and light are
    /// read from the client level's retained data at the surface, not from generation.
    /// None for a column whose chunk this client does not hold; never loads a chunk.
    /// `Err` without reading any column if there is no client world or more than 65,536
    /// columns (`MAX_COLUMNS`) were requested.
    #[access(WorldOnly)]
    fn client_surface_columns(
        &self,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
    ) -> Result<Vec<Option<SurfaceColumn>>, String>;
    /// Side: client.
    /// Chunk arrivals, changes and removals in the current client level (including
    /// singleplayer). Each touched key occurs once with its current residency.
    /// Pass None for a full snapshot; a cursor from another level or map, or outside
    /// the retained history, also returns a full snapshot. A stale or unavailable
    /// level handle returns None. Loads nothing.
    #[access(WorldOnly)]
    fn level_chunk_deltas(
        &self,
        level: LevelRef,
        since: Option<ChunkDeltaCursor>,
    ) -> Option<ChunkDeltas>;
    /// Side: client.
    /// For a client mod drawing a map past the loaded chunks: each column's surface as this window's far
    /// terrain (the LOD Distance setting) holds it, whether or not the chunk is loaded. It is the finest
    /// far tile sampled there, so a far column is the reduced surface of a cell up to many blocks wide, and
    /// it comes from wherever the far terrain does (this world's generator, the server's offer, the chunks
    /// kept from earlier visits). None for a column no tile covers yet, and for every
    /// column while far terrain is off or not started; the tiles follow the camera, so a column far from it
    /// can go back to None. Inside the loaded chunks read them instead (`client_surface_heights`): a tile can
    /// lag an edit. `Err`, reading nothing, for no client world (the title screen) or more than 65,536
    /// columns (`MAX_COLUMNS`).
    #[access(WorldOnly)]
    fn client_far_surface(
        &self,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<TerrainColumn>>, String>;
    /// Side: client.
    /// Read the CPU footprints of exact far-terrain tiles by `(tile_x, tile_z, level)`.
    /// A tile's world origin is `(tile_x, tile_z) * (16 << level)`; each response
    /// corresponds to one input. `None` means that exact tile is unavailable
    /// or far terrain is off, not that its cells are empty. A ready tile still
    /// covers the terrain the renderer keeps, which can lag live chunk edits.
    /// It does not generate, request, or load anything and never substitutes
    /// another level. Returns `Err` before reading if there is no client
    /// world or more than 256 tiles (65,536 cells) were requested.
    #[access(WorldOnly)]
    fn client_lod_columns(
        &self,
        tiles: &[(i32, i32, u8)],
    ) -> Result<Vec<Option<LodTileFootprint>>, String>;
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
