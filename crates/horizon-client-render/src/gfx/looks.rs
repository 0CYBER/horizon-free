//! The arenas' quad looks (`mesher::QuadLook`) bound for a shader API 0.2 pack's terrain while
//! sections carry them (`wanted`): one bind group a page, at `GROUP` of its terrain pipelines,
//! set with the page's vertices. A page without looks binds an empty buffer, which the
//! shaders read as no look (`LOOK_WGSL`).

use super::arena::Arena;
use super::flat::TRANSLUCENT_KEYS;
use crate::mesher::QuadLook;

/// The group of a pack's terrain pipelines that holds the looks.
pub const GROUP: u32 = 2;

/// Whether sections carry their quads' looks while a Horizon pack runs: for a pack that reads
/// them (`pack`, `Report::looks`), and for `HORIZON_TERRAIN_LOOKS=1`, the engine's own tests'
/// switch (`decide`).
pub fn wanted(adapter: &wgpu::Adapter, pack: bool) -> Result<bool, String> {
    static ASKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let switch =
        *ASKED.get_or_init(|| std::env::var("HORIZON_TERRAIN_LOOKS").as_deref() == Ok("1"));
    let flags = adapter.get_downlevel_capabilities().flags;
    let storage = flags.contains(wgpu::DownlevelFlags::VERTEX_STORAGE);
    if switch && !pack && !storage {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            log::warn!(
                "HORIZON_TERRAIN_LOOKS: this device's vertex stage binds no storage buffers, so terrain keeps no quad looks"
            )
        });
    }
    decide(pack, switch, storage)
}

/// `wanted` for a pack that reads the looks or not, the switch on or off, on a device whose
/// vertex stage binds storage buffers (`storage`) or not: where it doesn't, a pack that reads
/// them is refused, and the switch alone does nothing.
pub fn decide(pack: bool, switch: bool, storage: bool) -> Result<bool, String> {
    if pack && !storage {
        return Err(
            "the device's vertex stage reads no storage buffers, which the terrain's \
                    `tint`, `shade`, `ao`, `face` and `emission` need (`shaded_tint` doesn't)"
                .into(),
        );
    }
    Ok((pack || switch) && storage)
}

/// The looks as the material and shadow wrappers read them: `horizon_look(slot)`, a slot's
/// three words (`mesher::QuadLook`), zero past the buffer.
pub const LOOK_WGSL: &str = "
@group(2) @binding(0) var<storage, read> horizon_looks: array<u32>;
fn horizon_look(slot: u32) -> vec3<u32> {
    if slot * 3u + 2u >= arrayLength(&horizon_looks) {
        return vec3<u32>(0u);
    }
    return vec3<u32>(horizon_looks[slot * 3u], horizon_looks[slot * 3u + 1u],
        horizon_looks[slot * 3u + 2u]);
}
";

/// A mesh's quad `looks` laid out one a slot of its range of `slots`: a compact record's
/// (`packed`: records come in quad order, `mesher::packed::place`), or each of a wide quad's
/// four vertices'; the slots past them (padding, spilled vertices) have none. Nothing for a
/// mesh without looks, which `Arena::write_looks` clears.
pub fn per_slot(looks: &[QuadLook], packed: bool, slots: u64) -> Vec<QuadLook> {
    let mut out = Vec::new();
    if !looks.is_empty() {
        match packed {
            true => out.extend_from_slice(looks),
            false => out.extend(looks.iter().flat_map(|&l| [l; 4])),
        }
        out.resize(slots as usize, QuadLook::default());
    }
    out
}

/// The looks' WGSL for terrain pipelines with the group (`looks`) or without.
pub fn wgsl(looks: bool) -> &'static str {
    if looks { LOOK_WGSL } else { NO_LOOK_WGSL }
}

/// `LOOK_WGSL` for a pack that doesn't read them: every slot has none.
pub const NO_LOOK_WGSL: &str = "
fn horizon_look(slot: u32) -> vec3<u32> {
    return vec3<u32>(0u);
}
";

pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Terrain quad looks"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Each page's group, by flat page key (`flat::TRANSLUCENT_KEYS` and up: the translucent
/// arena's pages), made again when the page's buffer changes.
pub struct LookGroups {
    layout: wgpu::BindGroupLayout,
    empty: wgpu::Buffer,
    none: wgpu::BindGroup,
    groups: Vec<Option<(wgpu::Buffer, wgpu::BindGroup)>>,
}

impl LookGroups {
    pub fn new(device: &wgpu::Device, layout: wgpu::BindGroupLayout) -> Self {
        let empty = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("No terrain quad looks"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let none = Self::group(device, &layout, &empty);
        Self {
            layout,
            empty,
            none,
            groups: Vec::new(),
        }
    }

    fn group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Terrain quad looks"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    /// Before each frame's draws: every live page's buffer (a page that grew is another); a
    /// page gone drops its group.
    pub fn refresh(&mut self, device: &wgpu::Device, arena: &Arena, translucent: Option<&Arena>) {
        let pages = arena.page_buffers().map(|(p, _)| (p, arena.looks(p)));
        let split = translucent.into_iter().flat_map(|t| {
            t.page_buffers()
                .map(move |(p, _)| (p + TRANSLUCENT_KEYS, t.looks(p)))
        });
        let mut groups: Vec<Option<(wgpu::Buffer, wgpu::BindGroup)>> = Vec::new();
        for (key, buffer) in pages.chain(split) {
            let key = key as usize;
            if groups.len() <= key {
                groups.resize_with(key + 1, || None);
            }
            let buffer = buffer.unwrap_or(&self.empty);
            groups[key] = match self.groups.get_mut(key).and_then(Option::take) {
                Some((b, group)) if &b == buffer => Some((b, group)),
                _ => Some((buffer.clone(), Self::group(device, &self.layout, buffer))),
            };
        }
        self.groups = groups;
    }

    /// The group of the page with flat page key `key`.
    pub fn get(&self, key: u32) -> &wgpu::BindGroup {
        match self.groups.get(key as usize) {
            Some(Some((_, group))) => group,
            _ => &self.none,
        }
    }

    /// The group of `page` of the translucent arena where there is one (`translucent`), else of
    /// the section arena.
    pub fn page(&self, page: u32, translucent: bool) -> &wgpu::BindGroup {
        self.get(page + if translucent { TRANSLUCENT_KEYS } else { 0 })
    }
}

/// A section's quad looks of one part (solid, translucent), a slot of its range each
/// (`per_slot`): as its arena's page holds them (`None`: the page keeps none) and as its mesh
/// was handed over.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct PartLooks {
    pub held: Option<Vec<QuadLook>>,
    pub handed: Vec<QuadLook>,
}

#[cfg(any(test, feature = "test-support"))]
impl super::Renderer {
    /// `slots` stands in for the device's limit on the slots of a translucent page with looks
    /// (`Arena::set_looks`): the only arena whose pages may be past the real one.
    pub fn limit_translucent_look_pages(&mut self, slots: u64) {
        if let Some(arena) = &mut self.translucent_arena {
            arena.limit_looks(slots);
        }
    }
    /// Section `key`'s looks, solid then translucent, read back from its arenas, beside its
    /// mesh's as handed over (`keep_capture_copies` on before it was uploaded), and whether
    /// translucent quads have an arena of their own.
    pub fn section_looks(&mut self, key: super::SectionKey) -> Option<([PartLooks; 2], bool)> {
        if let Some(copies) = self.take_section_copies() {
            self.queue.submit([copies]);
        }
        let s = self.sections.find(key)?;
        let mesh = self.capture_copies.as_ref()?.held(key)?;
        let translucent = mesh.translucent.len() as u64;
        let split = self.translucent_arena.is_some();
        let parts = [
            (
                &self.arena,
                (s.page, s.start),
                s.solid_slots,
                &mesh.solid_looks,
                s.packed,
            ),
            (
                self.translucent_arena.as_ref().unwrap_or(&self.arena),
                s.translucent_at,
                translucent,
                &mesh.translucent_looks,
                false,
            ),
        ];
        let parts = parts.map(|(arena, (page, start), slots, looks, packed)| PartLooks {
            held: match slots {
                0 => Some(Vec::new()),
                _ => arena.looks(page).map(|buffer| {
                    let range = start * LOOK_BYTES..(start + slots) * LOOK_BYTES;
                    let (tx, rx) = std::sync::mpsc::channel();
                    wgpu::util::DownloadBuffer::read_buffer(
                        &self.device,
                        &self.queue,
                        &buffer.slice(range),
                        move |r| tx.send(r.unwrap().to_vec()).unwrap(),
                    );
                    self.device
                        .poll(wgpu::PollType::wait_indefinitely())
                        .unwrap();
                    bytemuck::pod_collect_to_vec(&rx.recv().unwrap())
                }),
            },
            handed: per_slot(looks, packed, slots),
        });
        Some((parts, split))
    }
}

#[cfg(any(test, feature = "test-support"))]
const LOOK_BYTES: u64 = std::mem::size_of::<QuadLook>() as u64;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::{TerrainVertex, packed};

    /// A pack that reads the looks gets them, or on a device whose vertex stage binds no
    /// storage buffers is refused with the reason; the engine's switch alone does nothing there.
    #[test]
    fn a_pack_that_reads_the_looks_needs_vertex_storage() {
        assert_eq!(decide(true, false, true), Ok(true));
        assert_eq!(decide(false, true, true), Ok(true));
        assert_eq!(decide(false, false, true), Ok(false));
        assert_eq!(decide(false, true, false), Ok(false));
        let refused = decide(true, true, false).unwrap_err();
        assert!(refused.starts_with("the device's vertex stage reads no storage buffers"));
        assert!(refused.contains("(`shaded_tint` doesn't)"));
    }

    /// A mesh's looks as the renderer uploads them (`per_slot`, `Arena::write_looks`) sit at
    /// its quads' slots, compact record by record or wide vertex by vertex, with none past
    /// them, also after the page grows; a range written again without looks reads none.
    #[test]
    fn uploaded_looks_sit_at_their_quads_slots() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        // Quads that pack into a record each, and one off the block grid that spills.
        let mut records: Vec<Option<[u32; 6]>> = (0..9u32)
            .map(|i| {
                Some([
                    i << 4,
                    (i * 7) << 4,
                    3 << 4,
                    0xffff,
                    0x00ff_ffff,
                    (i % 3) << 12,
                ])
            })
            .collect();
        let mut quads: Vec<[TerrainVertex; 4]> = records
            .iter()
            .map(|r| std::array::from_fn(|c| packed::packed_vertex(&r.unwrap(), c)))
            .collect();
        let mut spill = quads[2];
        spill[0].pos[1] = 8.5;
        quads.insert(4, spill);
        records.insert(4, None);
        let layout = packed::place(&records, &quads);
        assert!(!layout.is_empty());
        let looks: Vec<QuadLook> = (0..quads.len() as u32)
            .map(|q| QuadLook([0x0400_0000 | q, 0x8040_2010 + q, q % 16]))
            .collect();
        // Record r stands for quad r, which is what puts quad r's look at slot r.
        for (r, quad) in quads.iter().enumerate() {
            if layout[r][5] & packed::FULL == 0 {
                let v = packed::packed_vertex(&layout[r], 0);
                assert_eq!(bytemuck::bytes_of(&v), bytemuck::bytes_of(&quad[0]));
            }
        }
        let mut arena = Arena::new(&device, 24, 256, true);
        arena.set_looks(&device, true);
        let mut uploads = super::super::upload::Uploads::new(&device);
        let mut encoder = device.create_command_encoder(&Default::default());
        // A section without translucent quads: an empty range, in an arena with no page yet.
        let mut translucent = Arena::new(&device, 24, 256, true);
        translucent.set_looks(&device, true);
        translucent.write_looks(&mut encoder, &mut uploads, 0, 0, 0, &[]);
        let compact = (
            layout.len() as u64,
            per_slot(&looks, true, layout.len() as u64),
        );
        let wide = (
            4 * looks.len() as u64,
            per_slot(&looks, false, 4 * looks.len() as u64),
        );
        let mut at = Vec::new();
        for (slots, looks) in [&compact, &wide] {
            let (page, start) = arena.alloc(&device, &mut encoder, *slots).unwrap();
            arena.write_looks(&mut encoder, &mut uploads, page, start, *slots, looks);
            at.push((page, start));
        }
        // A range written with looks, freed and taken again by a mesh without them.
        let (page, start) = arena.alloc(&device, &mut encoder, wide.0).unwrap();
        arena.write_looks(&mut encoder, &mut uploads, page, start, wide.0, &wide.1);
        arena.release(page, start, wide.0);
        let again = arena.alloc(&device, &mut encoder, wide.0).unwrap();
        assert_eq!(again, (page, start));
        arena.write_looks(&mut encoder, &mut uploads, page, start, wide.0, &[]);
        // The page grows, its looks copied over.
        let grown = arena.alloc(&device, &mut encoder, 4096).unwrap();
        assert_eq!(grown.0, at[0].0, "the page grew in place");
        uploads.finish(&encoder);
        queue.submit([encoder.finish()]);
        let buffer = arena.looks(at[0].0).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        wgpu::util::DownloadBuffer::read_buffer(&device, &queue, &buffer.slice(..), move |r| {
            tx.send(r.unwrap().to_vec()).unwrap()
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = rx.recv().unwrap();
        let entry = |slot: u64| {
            let at = (slot * 12) as usize;
            QuadLook(bytemuck::pod_read_unaligned(&bytes[at..at + 12]))
        };
        let none = QuadLook::default();
        let (compact_at, wide_at) = (at[0].1, at[1].1);
        for (q, &look) in looks.iter().enumerate() {
            assert_eq!(entry(compact_at + q as u64), look, "record {q}");
            for i in 0..4 {
                assert_eq!(
                    entry(wide_at + 4 * q as u64 + i),
                    look,
                    "quad {q} corner {i}"
                );
            }
        }
        for slot in looks.len() as u64..compact.0 {
            assert_eq!(entry(compact_at + slot), none, "padding or spill {slot}");
        }
        for slot in 0..wide.0 {
            assert_eq!(entry(start + slot), none, "a range taken again {slot}");
        }
    }
}
