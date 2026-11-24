//! OptiFine-format Better Grass rules resolved to block states and atlas layers.
use crate::{connected::TileLayer, resource_source::ResourceRoot, sprites::Sprites};
use horizon_assets::model::Direction;
use horizon_core::fast_hash::FxHashMap;
use horizon_optifine::better_grass::{Mode, Rules, Surface};
use horizon_world::registry::{BlockRegistry, BlockStateId};

const PNG_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct Side {
    pub tile: TileLayer,
    pub tint: i32,
    pub overlay: Option<TileLayer>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ground {
    surface: Surface,
    snowy: bool,
}

impl Ground {
    /// Replace a stock-sized side (including its coextensive overlay), not a resource pack's
    /// inset or partial element that happens to declare the same cull direction.
    pub(crate) fn is_side(self, positions: [[f32; 3]; 4], facing: Direction) -> bool {
        let (fixed, across, plane) = match facing {
            Direction::North => (2, 0, 0.0),
            Direction::South => (2, 0, 1.0),
            Direction::West => (0, 2, 0.0),
            Direction::East => (0, 2, 1.0),
            _ => return false,
        };
        let height = if self.surface == Surface::DirtPath {
            15.0 / 16.0
        } else {
            1.0
        };
        let mut corners = 0;
        for p in positions {
            let index = match (p[fixed] == plane, p[across], p[1]) {
                (true, 0.0, 0.0) => 0,
                (true, 1.0, 0.0) => 1,
                (true, 0.0, y) if y == height => 2,
                (true, 1.0, y) if y == height => 3,
                _ => return false,
            };
            corners |= 1 << index;
        }
        corners == 0b1111
    }
}

pub(crate) struct BetterGrass {
    pack_config: bool,
    states: FxHashMap<u32, Ground>,
    tiles: [[Option<Side>; 2]; Surface::ALL.len()],
    path_side: Option<Side>,
}

fn tile(sprites: &mut Sprites, name: &str, fallback: &str) -> crate::Result<TileLayer> {
    let layer = if name == fallback {
        sprites.insert(fallback)?
    } else {
        match sprites.try_insert_limited(name, PNG_LIMIT) {
            Ok(layer) => layer,
            Err(error) => {
                log::warn!("Horizon: Better Grass sprite {name} is unusable: {error}");
                sprites.insert(fallback)?
            }
        }
    };
    Ok(TileLayer::Layer {
        layer: u16::try_from(layer)?,
        translucent: sprites.translucent[layer as usize],
        opaque: sprites.opaque[layer as usize],
    })
}

impl BetterGrass {
    pub(crate) fn load(
        root: &ResourceRoot,
        registry: &BlockRegistry,
        sprites: &mut Sprites,
    ) -> crate::Result<Self> {
        let pack = Rules::load(root.source());
        let pack_config = pack.is_some();
        let defaults = Rules::default();
        let rules = pack.as_ref().unwrap_or(&defaults);
        let mut states = FxHashMap::default();
        for id in 0..registry.len() as u32 {
            let state = registry
                .state(BlockStateId(id))
                .expect("registered block state");
            if let Some(surface) = Surface::ALL
                .into_iter()
                .find(|kind| kind.block_id() == state.name)
            {
                states.insert(
                    id,
                    Ground {
                        surface,
                        snowy: state.properties.get("snowy").is_some_and(|s| s == "true"),
                    },
                );
            }
        }
        let mut tiles = [[None; 2]; Surface::ALL.len()];
        for surface in Surface::ALL {
            for snowy in [false, true] {
                if !rules.enabled(surface, snowy) {
                    continue;
                }
                let top = tile(
                    sprites,
                    rules.texture(surface, snowy),
                    defaults.texture(surface, snowy),
                )?;
                let grass_tint = surface == Surface::Grass && !snowy;
                tiles[surface as usize][usize::from(snowy)] =
                    Some(if grass_tint && rules.grass_multilayer() {
                        Side {
                            tile: tile(sprites, rules.grass_side(), defaults.grass_side())?,
                            tint: -1,
                            overlay: Some(top),
                        }
                    } else {
                        Side {
                            tile: top,
                            tint: if grass_tint { 0 } else { -1 },
                            overlay: None,
                        }
                    });
            }
        }
        let path_side = (rules.enabled(Surface::DirtPath, false)
            && rules.dirt_path_side() != defaults.dirt_path_side())
        .then(|| tile(sprites, rules.dirt_path_side(), defaults.dirt_path_side()))
        .transpose()?
        .map(|tile| Side {
            tile,
            tint: -1,
            overlay: None,
        });
        Ok(Self {
            pack_config,
            states,
            tiles,
            path_side,
        })
    }

    pub(crate) fn resolve(&self, mode: Mode) -> Mode {
        match mode {
            Mode::Auto if self.pack_config => Mode::Fancy,
            Mode::Auto => Mode::Off,
            mode => mode,
        }
    }

    pub(crate) fn ground(&self, id: u32) -> Option<Ground> {
        self.states.get(&id).copied()
    }

    /// The caller supplies the state one step out and down, not just the side neighbour.
    pub(crate) fn side(&self, ground: Ground, mode: Mode, below: u32) -> Option<Side> {
        let top = self.tiles[ground.surface as usize][usize::from(ground.snowy)]?;
        match mode {
            Mode::Fast => Some(top),
            // The same block one step out and down, snowy or not: snow doesn't make it another block.
            Mode::Fancy
                if self
                    .ground(below)
                    .is_some_and(|below| below.surface == ground.surface) =>
            {
                Some(top)
            }
            Mode::Fancy if ground.surface == Surface::DirtPath => self.path_side,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fancy_side_matches_a_snowy_block_of_the_same_kind_below() {
        let grass = |snowy| Ground {
            surface: Surface::Grass,
            snowy,
        };
        let top = Side {
            tile: TileLayer::Layer {
                layer: 7,
                translucent: false,
                opaque: true,
            },
            tint: 0,
            overlay: None,
        };
        let mut tiles = [[None; 2]; Surface::ALL.len()];
        tiles[Surface::Grass as usize][0] = Some(top);
        let mut states = FxHashMap::default();
        states.insert(1, grass(false));
        states.insert(2, grass(true));
        states.insert(
            3,
            Ground {
                surface: Surface::Mycelium,
                snowy: false,
            },
        );
        let rules = BetterGrass {
            pack_config: true,
            states,
            tiles,
            path_side: None,
        };
        let layer = |below| rules.side(grass(false), Mode::Fancy, below).map(|s| s.tile);
        assert_eq!(layer(2), Some(top.tile));
        assert_eq!(layer(1), Some(top.tile));
        assert_eq!(layer(3), None);
        assert_eq!(layer(0), None);
    }
}
