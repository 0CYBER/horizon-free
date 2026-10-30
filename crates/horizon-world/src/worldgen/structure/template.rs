//! StructureTemplate: NBT loading, palettes, transforms and bounding boxes.
use super::{BoundingBox, Mirror, Rotation, StateTransforms};
use crate::{
    chunk::BlockState,
    nbt::{Compound, Tag},
    registry::{BlockRegistry, BlockStateId},
};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, PartialEq)]
pub struct BlockInfo {
    pub pos: [i32; 3],
    pub state: BlockStateId,
    pub nbt: Option<Compound>,
}
#[derive(Clone, Debug)]
pub struct JigsawInfo {
    pub info: BlockInfo,
    pub rollable: bool,
    pub name: String,
    pub pool: String,
    pub target: String,
    pub placement_priority: i32,
    pub selection_priority: i32,
}
#[derive(Debug)]
pub struct Palette {
    blocks: Vec<BlockInfo>,
    jigsaws: OnceLock<Vec<JigsawInfo>>,
}
impl Palette {
    pub fn blocks(&self) -> &[BlockInfo] {
        &self.blocks
    }
}
/// StructureTemplate.StructureEntityInfo: an entity saved with the template.
#[derive(Clone, Debug)]
pub struct EntityInfo {
    pub pos: [f64; 3],
    pub block_pos: [i32; 3],
    pub nbt: Compound,
}
#[derive(Debug)]
pub struct StructureTemplate {
    /// The id it was loaded under (`StructureResources::template`); empty
    /// for a template read from elsewhere.
    pub location: Arc<str>,
    pub size: [i32; 3],
    palettes: Vec<Palette>,
    pub entities: Vec<EntityInfo>,
}

fn int_list(tag: Option<&Tag>) -> [i32; 3] {
    let mut out = [0; 3];
    if let Some(Tag::List(values)) = tag {
        for (slot, value) in out.iter_mut().zip(values) {
            if let Tag::Int(v) = value {
                *slot = *v;
            }
        }
    }
    out
}
fn string(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}
fn get<'a>(compound: &'a Compound, key: &str) -> Option<&'a Tag> {
    compound.get(&crate::nbt::JavaString::from(key))
}
/// NbtUtils.readBlockState: unknown blocks read as air; missing or invalid
/// properties keep the block's defaults.
pub(super) fn read_state(registry: &BlockRegistry, tag: &Tag) -> BlockStateId {
    let air = registry.default_state("minecraft:air").expect("air");
    let Tag::Compound(compound) = tag else {
        return air;
    };
    // 26.3 writes `id`/`properties`; older templates use `Name`/`Properties`.
    let Some(name) = string(get(compound, "id")).or_else(|| string(get(compound, "Name"))) else {
        return air;
    };
    let Some(default) = registry.default_state(&name) else {
        return air;
    };
    let mut state = registry.state(default).unwrap().clone();
    if let Some(Tag::Compound(properties)) =
        get(compound, "properties").or_else(|| get(compound, "Properties"))
    {
        for (key, value) in properties {
            let (Tag::String(value), key) = (value, key.to_string_lossy()) else {
                continue;
            };
            let value = value.to_string_lossy();
            if let Some(slot) = state.properties.get_mut(&key) {
                let previous = std::mem::replace(slot, value);
                if registry.id(&state).is_none() {
                    *state.properties.get_mut(&key).unwrap() = previous;
                }
            }
        }
    }
    registry.id(&state).unwrap_or(default)
}
/// Parse `name[key=value,...]` (BlockStateParser without NBT); None if invalid.
pub(super) fn parse_state_string(registry: &BlockRegistry, text: &str) -> Option<BlockStateId> {
    let text = text.split('{').next().unwrap_or(text).trim();
    let (name, properties) = match text.split_once('[') {
        Some((name, rest)) => (name, rest.strip_suffix(']')?),
        None => (text, ""),
    };
    let name = if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    };
    let mut descriptor = BlockState {
        name,
        properties: Default::default(),
    };
    for pair in properties.split(',').filter(|p| !p.trim().is_empty()) {
        let (k, v) = pair.split_once('=')?;
        descriptor
            .properties
            .insert(k.trim().to_owned(), v.trim().to_owned());
    }
    registry.resolve(&descriptor).ok()
}

impl StructureTemplate {
    /// A new, never-loaded template: zero size and no palettes.
    pub fn empty() -> Self {
        Self {
            location: "".into(),
            size: [0; 3],
            palettes: Vec::new(),
            entities: Vec::new(),
        }
    }
    /// StructureTemplate.load. Blocks are ordered as vanilla orders them:
    /// full collision blocks, then other blocks, then block entities, each
    /// sorted by Y, X, Z.
    pub fn from_nbt(tag: &Tag, registry: &BlockRegistry, transforms: &StateTransforms) -> Self {
        let empty = Compound::new();
        let root = match tag {
            Tag::Compound(c) => c,
            _ => &empty,
        };
        let size = int_list(get(root, "size"));
        let blocks = match get(root, "blocks") {
            Some(Tag::List(list)) => list.as_slice(),
            _ => &[],
        };
        let mut palettes = Vec::new();
        let mut load = |palette: &[Tag]| {
            let states: Vec<BlockStateId> =
                palette.iter().map(|t| read_state(registry, t)).collect();
            let air = registry.default_state("minecraft:air").unwrap();
            let mut lists: [Vec<BlockInfo>; 3] = Default::default();
            for block in blocks {
                let Tag::Compound(block) = block else {
                    continue;
                };
                let index = match get(block, "state") {
                    Some(Tag::Int(i)) => *i,
                    _ => 0,
                };
                let state = usize::try_from(index)
                    .ok()
                    .and_then(|i| states.get(i))
                    .copied()
                    .unwrap_or(air);
                let nbt = match get(block, "nbt") {
                    Some(Tag::Compound(c)) => Some(c.clone()),
                    _ => None,
                };
                let class = if nbt.is_some() {
                    2
                } else if transforms.full_block(state) {
                    0
                } else {
                    1
                };
                lists[class].push(BlockInfo {
                    pos: int_list(get(block, "pos")),
                    state,
                    nbt,
                });
            }
            let mut result = Vec::with_capacity(blocks.len());
            for mut list in lists {
                list.sort_by_key(|b| (b.pos[1], b.pos[0], b.pos[2]));
                result.extend(list);
            }
            palettes.push(Palette {
                blocks: result,
                jigsaws: OnceLock::new(),
            });
        };
        match get(root, "palettes") {
            Some(Tag::List(lists)) => {
                for list in lists {
                    match list {
                        Tag::List(p) => load(p),
                        _ => load(&[]),
                    }
                }
            }
            _ => match get(root, "palette") {
                Some(Tag::List(p)) => load(p),
                _ => load(&[]),
            },
        }
        let mut entities = Vec::new();
        if let Some(Tag::List(list)) = get(root, "entities") {
            for e in list {
                let Tag::Compound(e) = e else { continue };
                let Some(Tag::Compound(nbt)) = get(e, "nbt") else {
                    continue;
                };
                let mut pos = [0.0; 3];
                if let Some(Tag::List(p)) = get(e, "pos") {
                    for (slot, v) in pos.iter_mut().zip(p) {
                        if let Tag::Double(bits) = v {
                            *slot = f64::from_bits(*bits);
                        }
                    }
                }
                entities.push(EntityInfo {
                    pos,
                    block_pos: int_list(get(e, "blockPos")),
                    nbt: nbt.clone(),
                });
            }
        }
        Self {
            location: "".into(),
            size,
            palettes,
            entities,
        }
    }
    /// StructureTemplate.transform(Vec3, ...): entity positions.
    pub fn transform_vec(
        pos: [f64; 3],
        mirror: Mirror,
        rotation: Rotation,
        pivot: [i32; 3],
    ) -> [f64; 3] {
        let [mut x, y, mut z] = pos;
        match mirror {
            Mirror::LeftRight => z = 1.0 - z,
            Mirror::FrontBack => x = 1.0 - x,
            Mirror::None => {}
        }
        let (px, pz) = (f64::from(pivot[0]), f64::from(pivot[2]));
        match rotation {
            Rotation::Counterclockwise90 => [px - pz + z, y, px + pz + 1.0 - x],
            Rotation::Clockwise90 => [px + pz + 1.0 - z, y, pz - px + x],
            Rotation::Clockwise180 => [px + px + 1.0 - x, y, pz + pz + 1.0 - z],
            Rotation::None => [x, y, z],
        }
    }
    pub fn palettes(&self) -> &[Palette] {
        &self.palettes
    }
    pub fn size(&self, rotation: Rotation) -> [i32; 3] {
        match rotation {
            Rotation::Clockwise90 | Rotation::Counterclockwise90 => {
                [self.size[2], self.size[1], self.size[0]]
            }
            _ => self.size,
        }
    }
    pub fn transform(
        pos: [i32; 3],
        mirror: Mirror,
        rotation: Rotation,
        pivot: [i32; 3],
    ) -> [i32; 3] {
        let [mut x, y, mut z] = pos;
        match mirror {
            Mirror::LeftRight => z = z.wrapping_neg(),
            Mirror::FrontBack => x = x.wrapping_neg(),
            Mirror::None => {}
        }
        let (px, pz) = (pivot[0], pivot[2]);
        match rotation {
            Rotation::Counterclockwise90 => [px - pz + z, y, px + pz - x],
            Rotation::Clockwise90 => [px + pz - z, y, pz - px + x],
            Rotation::Clockwise180 => [px + px - x, y, pz + pz - z],
            Rotation::None => [x, y, z],
        }
    }
    pub fn zero_position_with_transform(
        zero: [i32; 3],
        mirror: Mirror,
        rotation: Rotation,
        size_x: i32,
        size_z: i32,
    ) -> [i32; 3] {
        let (sx, sz) = (size_x - 1, size_z - 1);
        let mx = if mirror == Mirror::FrontBack { sx } else { 0 };
        let mz = if mirror == Mirror::LeftRight { sz } else { 0 };
        let d = match rotation {
            Rotation::Counterclockwise90 => [mz, 0, sx - mx],
            Rotation::Clockwise90 => [sz - mz, 0, mx],
            Rotation::Clockwise180 => [sx - mx, 0, sz - mz],
            Rotation::None => [mx, 0, mz],
        };
        [zero[0] + d[0], zero[1], zero[2] + d[2]]
    }
    pub fn bounding_box(
        &self,
        position: [i32; 3],
        rotation: Rotation,
        pivot: [i32; 3],
        mirror: Mirror,
    ) -> BoundingBox {
        let delta = [self.size[0] - 1, self.size[1] - 1, self.size[2] - 1];
        let a = Self::transform([0; 3], mirror, rotation, pivot);
        let b = Self::transform(delta, mirror, rotation, pivot);
        BoundingBox::from_corners(a, b).moved(position)
    }
    /// The palette selected by StructurePlaceSettings.getRandomPalette.
    pub fn palette_index(&self, random: &mut impl horizon_core::random::RandomSource) -> usize {
        random
            .next_int_bound(self.palettes.len().max(1) as i32)
            .unwrap_or(0) as usize
    }
    /// Jigsaw blocks of a palette, parsed from their block-entity data.
    pub fn jigsaws(
        &self,
        palette: usize,
        jigsaw_states: &dyn Fn(BlockStateId) -> Option<bool>,
    ) -> &[JigsawInfo] {
        let Some(palette) = self.palettes.get(palette) else {
            return &[];
        };
        palette.jigsaws.get_or_init(|| {
            palette
                .blocks
                .iter()
                .filter_map(|info| {
                    let horizontal = jigsaw_states(info.state)?;
                    let nbt = info.nbt.clone().unwrap_or_default();
                    let joint = string(get(&nbt, "joint"));
                    let text = |key: &str, default: &str| {
                        string(get(&nbt, key)).unwrap_or_else(|| default.to_owned())
                    };
                    let int = |key: &str| match get(&nbt, key) {
                        Some(Tag::Int(v)) => *v,
                        _ => 0,
                    };
                    Some(JigsawInfo {
                        rollable: match joint.as_deref() {
                            Some("rollable") => true,
                            Some("aligned") => false,
                            _ => !horizontal,
                        },
                        name: text("name", "minecraft:empty"),
                        pool: text("pool", "minecraft:empty"),
                        target: text("target", "minecraft:empty"),
                        placement_priority: int("placement_priority"),
                        selection_priority: int("selection_priority"),
                        info: info.clone(),
                    })
                })
                .collect()
        })
    }
}
