//! A new world's spawn and its first player's position: MinecraftServer.
//! setInitialSpawn walks a chunk spiral around NoiseBasedChunkGenerator.
//! getOrigin, then PlayerSpawnFinder.findSpawn picks a free column around
//! the world spawn within the respawn radius.
use super::{Game, dispenser::entity_random};
use horizon_client::{
    Result,
    assets::Properties,
    chunk::{HEIGHT, MIN_Y},
    collisions::{SceneCollisions, no_collision_or_liquid},
    world::World,
    world_spawn::{
        PlayerSpawnFinder, SpawnCandidate, SpawnLevel, level_respawn_pos, spawn_pos_in_chunk,
    },
};
use horizon_core::{
    geometry::{Aabb, Direction, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{
    block_collision::BlockCollisionRegistry, collision_context::CollisionContext,
    heightmap::HeightmapType, worldgen::terrain::TerrainBlockStates,
};

/// The loaded world as the spawn searches query it.
struct Columns<'a, 'r> {
    world: &'a World,
    properties: &'a Properties,
    binding: &'a BlockCollisionRegistry<'r>,
}
impl SpawnLevel for Columns<'_, '_> {
    fn min_y(&self) -> i32 {
        MIN_Y
    }
    fn height(&self, kind: HeightmapType, x: i32, z: i32) -> i32 {
        (MIN_Y..MIN_Y + HEIGHT)
            .rev()
            .find(|&y| kind.is_opaque(self.properties.heightmap_state(self.world.block(x, y, z))))
            .map_or(MIN_Y, |y| y + 1)
    }
    fn has_fluid(&self, x: i32, y: i32, z: i32) -> bool {
        use horizon_world::heightmap::HeightmapState;
        self.properties
            .heightmap_state(self.world.block(x, y, z))
            .has_fluid()
    }
    fn top_face_full(&self, x: i32, y: i32, z: i32) -> bool {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.binding,
            subject: None,
        };
        let state = self.world.block(x, y, z);
        let Ok(shape) = self.binding.collision_shape_with_context(
            &mut scene,
            state,
            BlockPos::new(x, y, z),
            &CollisionContext::empty(false),
        ) else {
            return false;
        };
        shape.face_shape(Direction::Up).is_ok_and(|face| {
            VoxelShape::block()
                .join_is_not_empty(&face, ShapeOp::XOR)
                .is_ok_and(|differs| !differs)
        })
    }
}

impl Game<'_> {
    fn spawn_columns(&self) -> Columns<'_, '_> {
        Columns {
            world: &self.world,
            properties: &self.pack.assets.properties,
            binding: self.collision,
        }
    }
    /// ChunkGenerator.getOrigin and getSpawnHeight of the overworld.
    fn spawn_origin(&self) -> Result<((i32, i32), i32)> {
        if let Some(flat) =
            horizon_client::save::world_gen_settings(&self.world_root).and_then(|s| {
                horizon_client::flatgen::FlatTerrain::from_settings(
                    &s,
                    self.pack.assets.registry,
                    self.res.biomes(),
                )
            })
        {
            return Ok(((0, 0), flat.spawn_height()));
        }
        if self.options.test_terrain {
            return Ok(((0, 0), 64));
        }
        let origin = match self.end.locator() {
            Some(terrain) => terrain.origin()?,
            None => horizon_core::position::ChunkPos::new(0, 0),
        };
        Ok(((origin.x, origin.z), 64))
    }
    /// MinecraftServer.setInitialSpawn: the first chunk of the spiral with a
    /// valid respawn column gives the world spawn, else the origin chunk's
    /// centre at the spawn height. None while a chunk to test is loading.
    pub(super) fn find_world_spawn(&mut self) -> Result<Option<[i32; 3]>> {
        let ((ox, oz), height) = match self.spawn_origin {
            Some(origin) => origin,
            None => *self.spawn_origin.insert(self.spawn_origin()?),
        };
        while let Some((dx, dz)) = self.spawn_spiral.current() {
            if self.spawn_spiral.tested() {
                let (cx, cz) = (ox + dx, oz + dz);
                if !self.streaming.is_lit(cx, cz) {
                    self.streaming.center = (cx, cz);
                    return Ok(None);
                }
                if let Some(pos) = spawn_pos_in_chunk(&self.spawn_columns(), cx, cz) {
                    return Ok(Some(pos));
                }
            }
            self.spawn_spiral.advance();
        }
        Ok(Some([ox * 16 + 8, height, oz * 16 + 8]))
    }
    /// PlayerSpawnFinder.findSpawn: the finder for a suggestion, its radius
    /// the respawn radius cut to the world border.
    fn spawn_finder(&self, suggestion: [i32; 3]) -> PlayerSpawnFinder {
        if self.level.game_mode == 2 {
            // Adventure mode: the suggestion itself.
            let mut finder = PlayerSpawnFinder::new(suggestion, 0, |_| 0);
            finder.next_candidate();
            return finder;
        }
        let mut radius = self.ui.rules.int("respawn_radius").max(0);
        let to_border = self
            .border
            .distance(f64::from(suggestion[0]), f64::from(suggestion[2]))
            .floor() as i32;
        if to_border < radius {
            radius = to_border;
        }
        if to_border <= 1 {
            radius = 1;
        }
        PlayerSpawnFinder::new(suggestion, radius, |n| {
            entity_random().next_int_bound(n).unwrap_or(0)
        })
    }
    /// PlayerSpawnFinder.findSpawn around the world spawn for a player placed
    /// at once (TeleportTransition.createDefault and missingRespawnBlock): the
    /// first free column within the respawn radius, the suggestion's own
    /// column when none is. A candidate whose chunk is not loaded counts as
    /// not free; the suggestion's chunk must be.
    pub(super) fn respawn_spawn(&self, suggestion: [i32; 3]) -> Vec3 {
        let mut finder = self.spawn_finder(suggestion);
        while let Some(candidate) = finder.next_candidate() {
            let SpawnCandidate::Column(x, z) = candidate else {
                break;
            };
            if !self.streaming.is_lit(x >> 4, z >> 4) {
                continue;
            }
            if let Some(pos) = level_respawn_pos(&self.spawn_columns(), x, z)
                && self.player_fits(pos)
            {
                return bottom_center(pos);
            }
        }
        self.fixup_spawn_height(suggestion)
    }
    /// PlayerSpawnFinder.findSpawn around the world spawn: the feet of a
    /// player who has no position yet. None while a candidate's chunk loads.
    pub(super) fn find_player_spawn(&mut self, suggestion: [i32; 3]) -> Result<Option<Vec3>> {
        let mut finder = match self.player_spawn {
            Some(finder) => finder,
            None => self.spawn_finder(suggestion),
        };
        let found = loop {
            let Some(candidate) = finder.next_candidate() else {
                break Some(self.fixup_spawn_height(suggestion));
            };
            let (x, z) = match candidate {
                SpawnCandidate::Column(x, z) => (x, z),
                SpawnCandidate::Suggestion => (suggestion[0], suggestion[2]),
            };
            if !self.streaming.is_lit(x >> 4, z >> 4) {
                self.streaming.center = (x >> 4, z >> 4);
                finder.retry();
                break None;
            }
            match candidate {
                SpawnCandidate::Column(x, z) => {
                    if let Some(pos) = level_respawn_pos(&self.spawn_columns(), x, z)
                        && self.player_fits(pos)
                    {
                        break Some(bottom_center(pos));
                    }
                }
                SpawnCandidate::Suggestion => break Some(self.fixup_spawn_height(suggestion)),
            }
        };
        self.player_spawn = found.is_none().then_some(finder);
        Ok(found)
    }
    /// PlayerSpawnFinder.noCollisionNoLiquid: a player's box standing at the
    /// block's bottom centre touches no block or liquid.
    fn player_fits(&self, pos: [i32; 3]) -> bool {
        let feet = bottom_center(pos);
        let bounds = Aabb::new(
            Vec3::new(feet.x - 0.3, feet.y, feet.z - 0.3),
            Vec3::new(feet.x + 0.3, feet.y + 1.8, feet.z + 0.3),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        no_collision_or_liquid(&mut scene, bounds)
    }
    /// PlayerSpawnFinder.fixupSpawnHeight: up out of anything solid, then
    /// down onto the ground.
    fn fixup_spawn_height(&self, pos: [i32; 3]) -> Vec3 {
        let [x, mut y, z] = pos;
        while !self.player_fits([x, y, z]) && y < MIN_Y + HEIGHT - 1 {
            y += 1;
        }
        y -= 1;
        while self.player_fits([x, y, z]) && y > MIN_Y {
            y -= 1;
        }
        bottom_center([x, y + 1, z])
    }
}

/// Vec3.atBottomCenterOf.
fn bottom_center(pos: [i32; 3]) -> Vec3 {
    Vec3::new(
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    )
}
