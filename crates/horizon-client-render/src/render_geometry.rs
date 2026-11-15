/// An entity, item and effect vertex (the entity pipelines), with a face
/// normal for `entity.wgsl`'s directional light.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
    /// RGBA8: tint multiplied by directional shade and ambient occlusion.
    pub color: u32,
    /// Smooth light, 16x fixed point: sky in bits 0..8, block in bits 8..16.
    pub light: u32,
    /// RGBA8_SNORM face normal; w = -127 for two-sided lighting, 0 to keep baked/unlit colour,
    /// `CULL_BACK` to keep it and also drop back-facing pixels.
    pub normal: [i8; 4],
    /// The `GLINT_TEXTURING` atlas UV for a foil item quad; zero elsewhere.
    pub glint_uv: [f32; 2],
}
/// Where a run of `Frame::entities` begins, and what it is (slice 7): the run lasts until the
/// next span. Recorded only while a shader pack draws entities, so Horizon's own frames carry
/// none; Horizon itself still draws the whole list at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct EntitySpan {
    /// The first vertex.
    pub start: u32,
    /// The render stage (5.4: `ENTITIES`, `BLOCK_ENTITIES`, `PARTICLES`, `DESTROY`), which picks
    /// the program.
    pub stage: u32,
    /// `entityId` for entities, `blockEntityId` for block entities (5.3), else 0.
    pub id: i32,
    /// An entity part that blends (a name tag, a shadow): `gbuffers_entities_translucent`.
    pub translucent: bool,
    /// Hidden from the camera (Horizon's entity culling, the player in first person): only the
    /// shadow pass draws it.
    pub hidden: bool,
    /// The local player (`shadowPlayer`).
    pub player: bool,
    /// A block entity whose block emits light (`shadowLightBlockEntities`).
    pub emissive: bool,
}
impl EntitySpan {
    /// Marks vertices `start..end` of `spans`' list (one entity) as hidden: the span covering
    /// `start` is split there, the spans inside are hidden, and the geometry after `end` goes
    /// on in the last span's kind. False when no span covers `start`.
    pub fn hide(spans: &mut Vec<EntitySpan>, start: usize, end: usize) -> bool {
        let Some(i) = spans.iter().rposition(|s| s.start as usize <= start) else {
            return false;
        };
        let after = spans[spans.len() - 1];
        if spans[i].start as usize != start {
            let split = EntitySpan {
                start: start as u32,
                ..spans[i]
            };
            spans.insert(i + 1, split);
        }
        for s in spans.iter_mut().skip(i) {
            if s.start as usize >= start {
                s.hidden = true;
            }
        }
        if end > start {
            spans.push(EntitySpan {
                start: end as u32,
                ..after
            });
        }
        true
    }
}
