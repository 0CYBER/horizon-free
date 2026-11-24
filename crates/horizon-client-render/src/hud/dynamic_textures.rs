//! Client mods' dynamic textures (`ClientRenderClientHost::create_dynamic_texture`, #11138),
//! vanilla's `DynamicTexture` registered in the `TextureManager` under an id. Here every image
//! lives in one page of the HUD atlas ([`PAGE`] texels) that the atlas packs only when a client
//! mod is loaded ([`super::HudAtlas::build_over_with_image`]). The page is two halves, and each
//! mod packs all its images into one of them ([`DynamicTextures::create`] says which), so two
//! mods never take room from each other. An image's id names its rect among the atlas's sprites,
//! so `DrawContext::sprite` (`native_sprite`), `gui_sprite` and an inline `TextObject::Atlas` of
//! `minecraft:gui` draw it as any other GUI sprite, and no resource reload is needed. An update
//! rewrites the image in place when it still fits there; a free drops the name and gives the
//! room back. None of it changes the atlas's `revision`, which says which atlas it is: what
//! draws these names looks them up as it draws (name tags and text displays keep no laid-out
//! line that holds an inline object). The last pixels of each image stay here, counted in the
//! mod's byte budget, to patch the atlas texture again after a resource reload or for a new
//! renderer.
use super::{HudAtlas, Rect, gui_sprite::atlas_key_of_gui_sprite};
use mod_api::{
    DYNAMIC_TEXTURE_MAX_SIDE, DYNAMIC_TEXTURE_MOD_BYTES, DYNAMIC_TEXTURE_MOD_IMAGES,
    DynamicTextureBudget, DynamicTextureError, Identifier,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A half of the page, in texels a side: 32 rows of 32 images of 32 x 32, each with the
/// transparent column and row it keeps ([`need`]). So a half holds 1024 images of one size up to
/// 32 x 32, or a whole byte budget (`DYNAMIC_TEXTURE_MOD_BYTES`) of images of one size whose sides
/// are powers of two from 32 to 1024 (`DYNAMIC_TEXTURE_MAX_SIDE`): 1024 of 32 x 32, 256 of
/// 64 x 64, ..., one of 1024 x 1024.
const HALF: u32 = 32 * 33;

/// The page's size in texels: two halves side by side, 8.5 MiB of atlas, paid only by a game with
/// a client mod loaded. Its 1056 rows are 32 more than the atlas's 1024-row slots; the atlas
/// rounds its height up to whole 1024-row bands, so whether they cost a band depends on the packs.
pub const PAGE: [u32; 2] = [2 * HALF, HALF];

/// The atlas sprite of the page while the atlas is packed; [`Page::take`] then takes it out of
/// the sprites, so no mod draws the page by name and sees another mod's images.
pub(super) const PAGE_SPRITE: &str = "horizon/dynamic-textures";

/// Where the page lies in one packed atlas, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    /// Its top left corner.
    pub origin: [u32; 2],
    /// Its width and height.
    pub size: [u32; 2],
    /// The atlas's width and height.
    pub atlas: [u32; 2],
}

impl Page {
    /// The packed page of an atlas `width` x `height`, taken out of its `sprites`; none when it
    /// packed none (no client mod was loaded).
    pub(super) fn take(
        sprites: &mut HashMap<String, Rect>,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let rect = sprites.remove(PAGE_SPRITE)?;
        let (w, h) = rect.texels((width, height));
        Some(Self {
            origin: [
                (rect.u0 * width as f32).round() as u32,
                (rect.v0 * height as f32).round() as u32,
            ],
            size: [w, h],
            atlas: [width, height],
        })
    }
    /// Each half's size: half the page's width, all its height.
    fn half(&self) -> [u32; 2] {
        [self.size[0] / 2, self.size[1]]
    }
    /// The atlas texel at `space`'s top left corner.
    fn corner(&self, space: Space) -> [u32; 2] {
        [
            self.origin[0] + space.side as u32 * self.half()[0] + space.at[0],
            self.origin[1] + space.at[1],
        ]
    }
    /// The sprite of a `size` image at `space`: its texels from the space's corner, its GUI size
    /// the image's.
    fn rect(&self, space: Space, size: [u32; 2]) -> Rect {
        let [x, y] = self.corner(space);
        let [width, height] = self.atlas.map(|side| side as f32);
        Rect {
            u0: x as f32 / width,
            v0: y as f32 / height,
            u1: (x + size[0]) as f32 / width,
            v1: (y + size[1]) as f32 / height,
            w: size[0],
            h: size[1],
        }
    }
    /// `image`'s pixels as a patch at its space, with the transparent column to its right and row
    /// below it that its space holds: the gap keeps a neighbour's texels, or those of an image
    /// freed there before, from bleeding in at the sprite's edges.
    fn patch(&self, image: &Image) -> crate::font::Patch {
        let [w, h] = image.size.map(|side| side as usize);
        let width = (w + 1).min(image.space.size[0] as usize);
        let height = (h + 1).min(image.space.size[1] as usize);
        let mut rgba = vec![0; width * height * 4];
        for (row, src) in image.pixels.chunks_exact(w * 4).enumerate() {
            rgba[row * width * 4..][..w * 4].copy_from_slice(src);
        }
        let [x, y] = self.corner(image.space);
        crate::font::Patch {
            x,
            y,
            width: width as u32,
            height: height as u32,
            rgba,
        }
    }
}

/// Where an image lies: its half (0 the left, 1 the right), the node of that half's [`Cuts`]
/// that holds it, and that node's corner and size in texels from the half's corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Space {
    side: usize,
    node: usize,
    at: [u32; 2],
    size: [u32; 2],
}

/// The texels an image of `size` takes: one more each way, for the transparent column to its
/// right and row below it, which stay inside its half.
fn need(size: [u32; 2]) -> [u32; 2] {
    size.map(|side| side + 1)
}

/// A half's packer: a tree of rectangles, each free, held by one image, or cut in two. An image
/// takes the smallest free rectangle it fits (the topmost, then the leftmost, of equal ones),
/// cut across at the image's height and then along at its width: images of one size fill a half
/// row by row, and an image shorter than a row starts rows of its own in the room beside the
/// tall one, rather than holding a column as tall as that row. A freed rectangle joins its free
/// neighbour back into the rectangle they were cut from, and so on up, so a half emptied is one
/// free rectangle again.
#[derive(Clone, Debug, Default)]
struct Cuts {
    nodes: Vec<Node>,
    /// Nodes that joined back into their parent, for the next cuts to reuse.
    spare: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    /// Its corner and size in texels from the half's corner.
    at: [u32; 2],
    size: [u32; 2],
    /// The node it was cut from; none for the half itself.
    parent: Option<usize>,
    state: State,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    Held,
    Cut([usize; 2]),
    Spare,
}

impl Cuts {
    /// A rectangle of `room` texels in a half of `half`: the node that now holds it, and its
    /// corner; none when no free rectangle is that large.
    fn alloc(&mut self, half: [u32; 2], room: [u32; 2]) -> Option<(usize, [u32; 2])> {
        if self.nodes.is_empty() {
            self.nodes.push(Node {
                at: [0, 0],
                size: half,
                parent: None,
                state: State::Free,
            });
        }
        let (mut held, _) = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.state == State::Free && node.size[0] >= room[0] && node.size[1] >= room[1]
            })
            .min_by_key(|(_, node)| {
                let area = u64::from(node.size[0]) * u64::from(node.size[1]);
                (area, node.at[1], node.at[0])
            })?;
        let Node {
            at: [x, y],
            size: [width, height],
            ..
        } = self.nodes[held];
        let [w, h] = room;
        if height > h {
            held = self.cut(
                held,
                [[x, y], [width, h]],
                [[x, y + h], [width, height - h]],
            );
        }
        if width > w {
            held = self.cut(held, [[x, y], [w, h]], [[x + w, y], [width - w, h]]);
        }
        self.nodes[held].state = State::Held;
        Some((held, [x, y]))
    }
    /// `node` cut into two free nodes, `first` and `second` (corner and size each): the first.
    fn cut(&mut self, node: usize, first: [[u32; 2]; 2], second: [[u32; 2]; 2]) -> usize {
        let a = self.add(node, first);
        let b = self.add(node, second);
        self.nodes[node].state = State::Cut([a, b]);
        a
    }
    /// A free node of `parent` at `at`, `size` texels.
    fn add(&mut self, parent: usize, [at, size]: [[u32; 2]; 2]) -> usize {
        let node = Node {
            at,
            size,
            parent: Some(parent),
            state: State::Free,
        };
        match self.spare.pop() {
            Some(spare) => {
                self.nodes[spare] = node;
                spare
            }
            None => {
                self.nodes.push(node);
                self.nodes.len() - 1
            }
        }
    }
    /// `node`'s room back, joined with its free neighbours into what they were cut from.
    fn free(&mut self, mut node: usize) {
        self.nodes[node].state = State::Free;
        while let Some(parent) = self.nodes[node].parent {
            let State::Cut(children) = self.nodes[parent].state else {
                break;
            };
            if children
                .iter()
                .any(|&child| self.nodes[child].state != State::Free)
            {
                break;
            }
            for child in children {
                self.nodes[child].state = State::Spare;
                self.spare.push(child);
            }
            self.nodes[parent].state = State::Free;
            node = parent;
        }
    }
}

/// Every mod's dynamic textures: where each lies in the page, and its last pixels.
#[derive(Default)]
pub struct DynamicTextures {
    /// Each image by its id (`mymod:emoji/smile`), the name it has among the atlas's sprites.
    images: BTreeMap<String, Image>,
    /// The halves' packers, the left's first.
    halves: [Cuts; 2],
    limits: Limits,
    /// The `revision` of the atlas whose sprites name the images: a call that changes the atlas or
    /// patches it first binds them to one it has not seen ([`DynamicTextures::rebind`]).
    bound: u64,
}

struct Image {
    /// The native mod that created it: its budget counts the image, and only it changes it.
    owner: Box<str>,
    space: Space,
    size: [u32; 2],
    /// The last pixels given, `size` RGBA: what the atlas gets again after a rebuild.
    pixels: Vec<u8>,
    /// Whether its id is among the atlas's sprites as this image: a sprite the packs brought
    /// under the same id after a resource reload keeps the name.
    named: bool,
    /// Whether the atlas texture lacks these pixels yet: set by a create, an update, a rebuilt
    /// atlas or a new renderer, cleared when [`DynamicTextures::patches`] hands them to a frame.
    dirty: bool,
}

/// Each mod's budgets and the longest side: the API's own, smaller in tests.
#[derive(Clone, Copy, Debug)]
struct Limits {
    images: u32,
    bytes: u64,
    side: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            images: DYNAMIC_TEXTURE_MOD_IMAGES,
            bytes: DYNAMIC_TEXTURE_MOD_BYTES,
            side: DYNAMIC_TEXTURE_MAX_SIDE,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Held {
    images: u32,
    bytes: u64,
}

impl DynamicTextures {
    /// Mod `owner` (none outside a mod's call) creates image `id` of `size` (width, height) from
    /// `rgba`, `width * height` pixels of four bytes, in its half of `atlas`'s page, and names it
    /// among the atlas's sprites. A mod's first image takes the half fewer mods hold images in
    /// (the left on a tie) and its later ones follow it there, so a second mod always gets the
    /// half the first does not use, and a mod shares a half only when both halves hold other
    /// mods' images as it takes one. Every refusal comes before the pixels are copied, and
    /// changes nothing.
    pub fn create(
        &mut self,
        atlas: &mut HudAtlas,
        owner: Option<&str>,
        id: &Identifier,
        size: [u32; 2],
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let page = atlas
            .dynamic_texture_page
            .ok_or(DynamicTextureError::Unavailable)?;
        let owner = owned(owner, id)?;
        let bytes = self.sized(size, rgba.len())?;
        self.rebind(atlas);
        let name = id.to_string();
        if clashes(&name, |key| {
            self.images.contains_key(key) || atlas.sprites.contains_key(key)
        }) {
            return Err(DynamicTextureError::NameTaken);
        }
        let held = self.held(owner);
        if held.images >= self.limits.images || held.bytes + bytes > self.limits.bytes {
            return Err(DynamicTextureError::OverBudget);
        }
        let side = self.side(owner);
        let space = self
            .alloc(page, side, size)
            .ok_or(DynamicTextureError::NoSlot)?;
        atlas.sprites.insert(name.clone(), page.rect(space, size));
        self.images.insert(
            name,
            Image {
                owner: owner.into(),
                space,
                size,
                pixels: rgba.to_vec(),
                named: true,
                dirty: true,
            },
        );
        Ok(())
    }
    /// Mod `owner` gives its image `id` new pixels of `size`: in the space it holds when they fit
    /// there, else in new room of its half, the old given back. Every refusal comes before any
    /// copy, and changes nothing.
    pub fn update(
        &mut self,
        atlas: &mut HudAtlas,
        owner: Option<&str>,
        id: &Identifier,
        size: [u32; 2],
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let page = atlas
            .dynamic_texture_page
            .ok_or(DynamicTextureError::Unavailable)?;
        let owner = owned(owner, id)?;
        self.rebind(atlas);
        let name = id.to_string();
        let held = self.held(owner);
        let (old, kept) = match self.images.get(&name) {
            None => return Err(DynamicTextureError::Unknown),
            Some(image) if image.owner.as_ref() != owner => {
                return Err(DynamicTextureError::NotOwned);
            }
            Some(image) => (image.space, image.pixels.len() as u64),
        };
        let bytes = self.sized(size, rgba.len())?;
        if held.bytes - kept + bytes > self.limits.bytes {
            return Err(DynamicTextureError::OverBudget);
        }
        let [w, h] = need(size);
        let space = if w <= old.size[0] && h <= old.size[1] {
            old
        } else {
            let space = self
                .alloc(page, old.side, size)
                .ok_or(DynamicTextureError::NoSlot)?;
            self.halves[old.side].free(old.node);
            space
        };
        let image = self.images.get_mut(&name).expect("the image checked above");
        if image.pixels.len() == rgba.len() {
            image.pixels.copy_from_slice(rgba);
        } else {
            image.pixels = rgba.to_vec();
        }
        if image.space != space || image.size != size {
            image.space = space;
            image.size = size;
            if image.named {
                atlas.sprites.insert(name, page.rect(space, size));
            }
        }
        image.dirty = true;
        Ok(())
    }
    /// Mod `owner` frees its image `id`: the name leaves the atlas's sprites, the room and the
    /// budget come back. A refusal changes nothing.
    pub fn free(
        &mut self,
        atlas: &mut HudAtlas,
        owner: Option<&str>,
        id: &Identifier,
    ) -> Result<(), DynamicTextureError> {
        atlas
            .dynamic_texture_page
            .ok_or(DynamicTextureError::Unavailable)?;
        let owner = owned(owner, id)?;
        self.rebind(atlas);
        let name = id.to_string();
        match self.images.get(&name) {
            None => return Err(DynamicTextureError::Unknown),
            Some(image) if image.owner.as_ref() != owner => {
                return Err(DynamicTextureError::NotOwned);
            }
            Some(_) => {}
        }
        let image = self.images.remove(&name).expect("the image checked above");
        self.halves[image.space.side].free(image.space.node);
        if image.named {
            atlas.sprites.remove(&name);
        }
        Ok(())
    }
    /// What mod `owner` may still create.
    pub fn budget(
        &self,
        atlas: &HudAtlas,
        owner: Option<&str>,
    ) -> Result<DynamicTextureBudget, DynamicTextureError> {
        atlas
            .dynamic_texture_page
            .ok_or(DynamicTextureError::Unavailable)?;
        let owner = owner.ok_or(DynamicTextureError::NotOwned)?;
        let held = self.held(owner);
        Ok(mod_api::__private::built::dynamic_texture_budget(
            self.limits.images.saturating_sub(held.images),
            self.limits.bytes.saturating_sub(held.bytes),
        ))
    }
    /// The images bound to `atlas`, a rebuilt one (a resource reload): every image keeps its place
    /// in the new atlas's page, its name goes back among the sprites and its kept pixels are
    /// patched again. A sprite the packs have under the image's id, in either form ([`clashes`]),
    /// keeps the name. An atlas with no page leaves the names out and the pixels kept, until one
    /// has a page again. A create, an update, a free and the frame's [`DynamicTextures::patches`]
    /// first bind the images to an atlas they have not seen (by its `revision`), so the images
    /// follow a rebuilt atlas without this call too; the game makes it as the atlas comes, so the
    /// names are there before anything draws.
    pub fn rebind(&mut self, atlas: &mut HudAtlas) {
        if self.bound == atlas.revision {
            return;
        }
        self.bound = atlas.revision;
        let page = atlas.dynamic_texture_page;
        // Each against the packs' sprites alone, before any image's name goes in.
        for (name, image) in &mut self.images {
            image.dirty = true;
            image.named = page.is_some() && !clashes(name, |key| atlas.sprites.contains_key(key));
            if page.is_some() && !image.named {
                log::warn!(
                    "GUI atlas: dynamic texture {name} is hidden by a resource pack's sprite of that id"
                );
            }
        }
        let Some(page) = page else {
            return;
        };
        for (name, image) in self.images.iter().filter(|(_, image)| image.named) {
            atlas
                .sprites
                .insert(name.clone(), page.rect(image.space, image.size));
        }
    }
    /// A new renderer, whose atlas texture holds only the packed pixels: every image is patched
    /// again.
    pub fn forget_uploads(&mut self) {
        for image in self.images.values_mut() {
            image.dirty = true;
        }
    }
    /// The images `atlas`'s texture lacks, as patches into its page, each once until it changes
    /// again; bound to `atlas` first ([`DynamicTextures::rebind`]).
    pub fn patches(&mut self, atlas: &mut HudAtlas, patches: &mut Vec<crate::font::Patch>) {
        self.rebind(atlas);
        let Some(page) = atlas.dynamic_texture_page else {
            return;
        };
        for image in self.images.values_mut().filter(|image| image.dirty) {
            image.dirty = false;
            patches.push(page.patch(image));
        }
    }
    /// The images and bytes mod `owner` holds.
    fn held(&self, owner: &str) -> Held {
        self.images
            .values()
            .filter(|image| image.owner.as_ref() == owner)
            .fold(Held::default(), |held, image| Held {
                images: held.images + 1,
                bytes: held.bytes + image.pixels.len() as u64,
            })
    }
    /// The half `owner`'s images lie in; for a mod with none, the half fewer mods hold images in,
    /// the left on a tie.
    fn side(&self, owner: &str) -> usize {
        let mut mods: [BTreeSet<&str>; 2] = Default::default();
        for image in self.images.values() {
            if image.owner.as_ref() == owner {
                return image.space.side;
            }
            mods[image.space.side].insert(&*image.owner);
        }
        usize::from(mods[1].len() < mods[0].len())
    }
    /// Room for an image of `size` in half `side` of `page`.
    fn alloc(&mut self, page: Page, side: usize, size: [u32; 2]) -> Option<Space> {
        let room = need(size);
        let (node, at) = self.halves[side].alloc(page.half(), room)?;
        Some(Space {
            side,
            node,
            at,
            size: room,
        })
    }
    /// The bytes of a `[width, height]` image of `len` bytes: `BadSize` for a side of 0 or
    /// pixels that are not `width * height * 4` bytes, `TooLarge` past the longest side. The
    /// sides are checked before the bytes are counted, so the count cannot overflow.
    fn sized(&self, [width, height]: [u32; 2], len: usize) -> Result<u64, DynamicTextureError> {
        if width == 0 || height == 0 {
            return Err(DynamicTextureError::BadSize);
        }
        if width > self.limits.side || height > self.limits.side {
            return Err(DynamicTextureError::TooLarge);
        }
        let bytes = u64::from(width) * u64::from(height) * 4;
        if bytes != len as u64 {
            return Err(DynamicTextureError::BadSize);
        }
        Ok(bytes)
    }
}

/// Whether `holds` holds `name` or the other form of its id. `DrawContext::sprite` and
/// `gui_sprite` find `ns:path` itself, and an inline `TextObject::Atlas` of `minecraft:gui` finds
/// `ns:gui/sprites/path` first and `ns:path` after it, so the two are one id's two forms
/// (`mymod:emoji/smile` and `mymod:gui/sprites/emoji/smile`). An image never takes a form that a
/// pack's sprite or another image holds, whichever came first, so one id draws one picture.
fn clashes(name: &str, holds: impl Fn(&str) -> bool) -> bool {
    let plain = name.split_once(':').and_then(|(namespace, path)| {
        let path = path.strip_prefix("gui/sprites/")?;
        Some(format!("{namespace}:{path}"))
    });
    holds(name)
        || holds(atlas_key_of_gui_sprite(name).as_str())
        || plain.is_some_and(|plain| holds(plain.as_str()))
}

/// `owner`, when `id` is in a namespace `owner` owns (its own or its manifest's other
/// namespaces), never vanilla's `minecraft`.
fn owned<'a>(owner: Option<&'a str>, id: &Identifier) -> Result<&'a str, DynamicTextureError> {
    owner
        .filter(|owner| {
            id.namespace() != "minecraft"
                && mod_api::__private::owns_namespace(owner, id.namespace())
        })
        .ok_or(DynamicTextureError::NotOwned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DynamicTextureError::*;

    /// A page of two 40 x 40 halves at (512, 600) of the 1024 x 1024 boot atlas.
    const ORIGIN: [u32; 2] = [512, 600];

    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }
    fn atlas(origin: [u32; 2]) -> HudAtlas {
        let mut atlas = HudAtlas::boot();
        atlas.dynamic_texture_page = Some(Page {
            origin,
            size: [80, 40],
            atlas: [atlas.width, atlas.height],
        });
        atlas
    }
    /// Three 16 x 16 images, four images and a 32-texel side a mod.
    fn textures() -> DynamicTextures {
        DynamicTextures {
            limits: Limits {
                images: 4,
                bytes: 3 * 16 * 16 * 4,
                side: 32,
            },
            ..Default::default()
        }
    }
    fn pixels([w, h]: [u32; 2], rgba: [u8; 4]) -> Vec<u8> {
        rgba.repeat((w * h) as usize)
    }
    /// A sprite's rect in texels of the 1024 x 1024 atlas, and its GUI size.
    fn texels(rect: Rect) -> [u32; 6] {
        let at = |uv: f32| (uv * 1024.0).round() as u32;
        [
            at(rect.u0),
            at(rect.v0),
            at(rect.u1),
            at(rect.v1),
            rect.w,
            rect.h,
        ]
    }
    fn drained(textures: &mut DynamicTextures, atlas: &mut HudAtlas) -> Vec<crate::font::Patch> {
        let mut patches = Vec::new();
        textures.patches(atlas, &mut patches);
        patches
    }

    #[test]
    fn the_cuts_fill_rows_reuse_freed_room_and_join_it_back() {
        let half = [64, 32];
        assert_eq!(
            Cuts::default().alloc(half, [65, 1]),
            None,
            "wider than the half"
        );
        let mut cuts = Cuts::default();
        // Three 17-texel squares fill a row; a fourth fits neither it nor below it.
        let first = cuts.alloc(half, [17, 17]).unwrap();
        let second = cuts.alloc(half, [17, 17]).unwrap();
        let third = cuts.alloc(half, [17, 17]).unwrap();
        assert_eq!([first.1, second.1, third.1], [[0, 0], [17, 0], [34, 0]]);
        assert_eq!(cuts.alloc(half, [17, 17]), None, "a full half");
        // A short image takes the smallest room it fits, the row's end, and starts rows there.
        let short = cuts.alloc(half, [8, 4]).unwrap();
        assert_eq!(short.1, [51, 0]);
        let below = cuts.alloc(half, [8, 4]).unwrap();
        assert_eq!(below.1, [51, 4]);
        // A freed square's room takes the next square.
        cuts.free(second.0);
        let again = cuts.alloc(half, [17, 17]).unwrap();
        assert_eq!(again.1, [17, 0]);
        // Freed rooms join back into what they were cut from: the whole half is free again.
        for (node, _) in [first, again, third, short, below] {
            cuts.free(node);
        }
        assert_eq!(cuts.alloc(half, [64, 32]).map(|(_, at)| at), Some([0, 0]));
    }

    /// Two mods never take room from each other: whatever the first holds in its half, the second
    /// fills its whole budget in the other and is refused only past it. These are the worst
    /// shapes for a page the mods share: one 1024 x 1024 image, and 1 x 1024 strips, which take
    /// twice their pixels in room.
    #[test]
    fn two_mods_fill_their_budgets_in_their_own_halves() {
        let page = Some(Page {
            origin: [0, 0],
            size: PAGE,
            atlas: [4096, 4096],
        });
        let (big, strip, emoji) = (
            pixels([1024, 1024], [1; 4]),
            pixels([1, 1024], [2; 4]),
            pixels([32, 32], [3; 4]),
        );
        // How many images of one size mod `owner` creates before a refusal, and the refusal.
        let fill = |textures: &mut DynamicTextures,
                    atlas: &mut HudAtlas,
                    owner: &str,
                    size: [u32; 2],
                    rgba: &[u8]| {
            let mut made = 0;
            loop {
                let name = id(&format!("{owner}:image/{made}"));
                match textures.create(atlas, Some(owner), &name, size, rgba) {
                    Ok(()) => made += 1,
                    Err(refusal) => return (made, refusal),
                }
            }
        };
        let mut atlas = HudAtlas::boot();
        atlas.dynamic_texture_page = page;
        let mut textures = DynamicTextures::default();
        let one = fill(&mut textures, &mut atlas, "a", [1024, 1024], &big);
        assert_eq!(
            one,
            (1, OverBudget),
            "one 1024 x 1024 image is a whole budget"
        );
        let set = fill(&mut textures, &mut atlas, "b", [32, 32], &emoji);
        assert_eq!(set, (1024, OverBudget), "beside it, a whole emoji set");
        let mut atlas = HudAtlas::boot();
        atlas.dynamic_texture_page = page;
        let mut textures = DynamicTextures::default();
        let strips = fill(&mut textures, &mut atlas, "a", [1, 1024], &strip);
        assert_eq!(strips, (528, NoSlot), "strips fill their own half first");
        let set = fill(&mut textures, &mut atlas, "b", [32, 32], &emoji);
        assert_eq!(
            set,
            (1024, OverBudget),
            "the other half still holds a whole set"
        );
        let third = fill(&mut textures, &mut atlas, "c", [32, 32], &emoji);
        assert_eq!(third, (0, NoSlot), "a third mod shares the left half");
    }

    #[test]
    fn an_image_resolves_as_a_gui_sprite_and_an_inline_atlas_object_until_freed() {
        let mut atlas = atlas(ORIGIN);
        let mut textures = textures();
        let smile = id("a:emoji/smile");
        let revision = atlas.revision;
        let red = [255, 0, 0, 255];
        textures
            .create(
                &mut atlas,
                Some("a"),
                &smile,
                [16, 16],
                &pixels([16, 16], red),
            )
            .unwrap();
        assert_eq!(atlas.revision, revision, "still the same atlas");
        // `DrawContext::sprite` and `gui_sprite` read the name; the inline object its GUI atlas.
        let expected = [512, 600, 528, 616, 16, 16];
        assert_eq!(texels(atlas.sprites["a:emoji/smile"]), expected);
        let inline = atlas
            .inline_sprite("minecraft:gui", "a:emoji/smile")
            .unwrap();
        assert_eq!(texels(inline), expected);
        // The next frame uploads it once, with a transparent gap right and below.
        let patches = drained(&mut textures, &mut atlas);
        assert_eq!(patches.len(), 1);
        let patch = &patches[0];
        assert_eq!(
            [patch.x, patch.y, patch.width, patch.height],
            [512, 600, 17, 17]
        );
        let texel = |x: usize, y: usize| &patch.rgba[(y * 17 + x) * 4..][..4];
        assert_eq!(texel(15, 15), red);
        assert_eq!(texel(16, 0), [0; 4]);
        assert_eq!(texel(0, 16), [0; 4]);
        assert!(
            drained(&mut textures, &mut atlas).is_empty(),
            "uploaded once"
        );
        textures.free(&mut atlas, Some("a"), &smile).unwrap();
        assert_eq!(atlas.revision, revision);
        assert!(!atlas.sprites.contains_key("a:emoji/smile"));
        // The boot atlas has no missing sprite to fall back to.
        assert!(
            atlas
                .inline_sprite("minecraft:gui", "a:emoji/smile")
                .is_none()
        );
    }

    /// `a:x` and `a:gui/sprites/x` are one id's two forms (an inline atlas object finds the
    /// second first): an image never takes one while an image or a pack's sprite holds the
    /// other, in either order, and a rebuilt atlas names every image again.
    #[test]
    fn an_id_and_its_gui_sprites_form_draw_one_picture() {
        let mut atlas = atlas(ORIGIN);
        let mut textures = textures();
        let square = pixels([4, 4], [7; 4]);
        let create = |textures: &mut DynamicTextures, atlas: &mut HudAtlas, name| {
            textures.create(atlas, Some("a"), &id(name), [4, 4], &square)
        };
        create(&mut textures, &mut atlas, "a:x").unwrap();
        let other = create(&mut textures, &mut atlas, "a:gui/sprites/x");
        assert_eq!(other, Err(NameTaken));
        create(&mut textures, &mut atlas, "a:gui/sprites/y").unwrap();
        assert_eq!(create(&mut textures, &mut atlas, "a:y"), Err(NameTaken));
        // A resource reload names both images again, each its own id's picture.
        let mut rebuilt = self::atlas([100, 700]);
        textures.rebind(&mut rebuilt);
        let inline =
            |atlas: &HudAtlas, sprite| atlas.inline_sprite("minecraft:gui", sprite).map(texels);
        let x = texels(rebuilt.sprites["a:x"]);
        assert_eq!(inline(&rebuilt, "a:x"), Some(x));
        let y = texels(rebuilt.sprites["a:gui/sprites/y"]);
        assert_eq!(inline(&rebuilt, "a:y"), Some(y));
        // A pack's sprite under the other form hides an image while that atlas is up, no longer.
        let mut packed = self::atlas(ORIGIN);
        let white = packed.white;
        packed.sprites.insert("a:gui/sprites/x".into(), white);
        packed.sprites.insert("a:y".into(), white);
        textures.rebind(&mut packed);
        assert!(!packed.sprites.contains_key("a:x"));
        assert!(!packed.sprites.contains_key("a:gui/sprites/y"));
        let mut unpacked = self::atlas(ORIGIN);
        textures.rebind(&mut unpacked);
        assert!(unpacked.sprites.contains_key("a:x"));
        assert!(unpacked.sprites.contains_key("a:gui/sprites/y"));
    }

    #[test]
    fn every_refusal_is_named_and_changes_nothing() {
        let mut atlas = atlas(ORIGIN);
        atlas.dynamic_texture_page = None;
        let mut textures = textures();
        let square = pixels([16, 16], [1, 2, 3, 4]);
        assert_eq!(
            textures.create(&mut atlas, Some("a"), &id("a:one"), [16, 16], &square),
            Err(Unavailable)
        );
        assert_eq!(textures.budget(&atlas, Some("a")), Err(Unavailable));
        let mut atlas = self::atlas(ORIGIN);
        for name in ["a:one", "a:two", "a:three"] {
            textures
                .create(&mut atlas, Some("a"), &id(name), [16, 16], &square)
                .unwrap();
        }
        let tiny = pixels([1, 1], [9; 4]);
        for name in ["b:p0", "b:p1", "b:p2", "b:p3"] {
            textures
                .create(&mut atlas, Some("b"), &id(name), [1, 1], &tiny)
                .unwrap();
        }
        let budget = |textures: &DynamicTextures, atlas: &HudAtlas, owner| {
            let budget = textures.budget(atlas, Some(owner)).unwrap();
            [u64::from(budget.images), budget.bytes]
        };
        assert_eq!(budget(&textures, &atlas, "a"), [1, 0]);
        assert_eq!(budget(&textures, &atlas, "b"), [0, 3 * 1024 - 4 * 4]);
        assert_eq!(textures.budget(&atlas, None), Err(NotOwned));
        atlas.sprites.insert("a:taken".into(), atlas.white);
        atlas
            .sprites
            .insert("c:gui/sprites/panel".into(), atlas.white);
        let state = |textures: &DynamicTextures, atlas: &HudAtlas| {
            let mut names: Vec<_> = atlas.sprites.keys().cloned().collect();
            names.sort();
            (
                names,
                budget(textures, atlas, "a"),
                budget(textures, atlas, "b"),
            )
        };
        let before = state(&textures, &atlas);
        let (wide, long, big, bar) = (
            pixels([32, 14], [5; 4]),
            pixels([33, 1], [0; 4]),
            pixels([32, 32], [0; 4]),
            pixels([30, 8], [6; 4]),
        );
        type Call<'a> = (
            Option<&'a str>,
            &'a str,
            [u32; 2],
            &'a [u8],
            DynamicTextureError,
        );
        // `c` shares `a`'s half, the left: both halves held another mod's images when it came.
        let refusals: [Call<'_>; 12] = [
            (None, "a:x", [1, 1], &tiny, NotOwned),
            (Some("b"), "a:x", [1, 1], &tiny, NotOwned),
            (Some("minecraft"), "minecraft:x", [1, 1], &tiny, NotOwned),
            (Some("c"), "c:x", [0, 4], &[], BadSize),
            (Some("c"), "c:x", [4, 4], &square[..63], BadSize),
            (Some("c"), "c:x", [33, 1], &long, TooLarge),
            (Some("a"), "a:one", [16, 16], &square, NameTaken),
            (Some("a"), "a:taken", [1, 1], &tiny, NameTaken),
            (Some("c"), "c:panel", [1, 1], &tiny, NameTaken),
            (Some("a"), "a:four", [16, 16], &square, OverBudget),
            (Some("b"), "b:p4", [1, 1], &tiny, OverBudget),
            (Some("c"), "c:wide", [32, 14], &wide, NoSlot),
        ];
        for (owner, name, size, rgba, refusal) in refusals {
            let got = textures.create(&mut atlas, owner, &id(name), size, rgba);
            assert_eq!(got, Err(refusal), "create {name} {size:?}");
            assert_eq!(
                state(&textures, &atlas),
                before,
                "create {name} changed nothing"
            );
        }
        let one = id("a:one");
        let updates: [Call<'_>; 5] = [
            (Some("b"), "a:one", [1, 1], &tiny, NotOwned),
            (Some("c"), "c:none", [1, 1], &tiny, Unknown),
            (Some("a"), "a:one", [16, 16], &square[..16], BadSize),
            (Some("a"), "a:one", [32, 32], &big, OverBudget),
            (Some("a"), "a:one", [30, 8], &bar, NoSlot),
        ];
        for (owner, name, size, rgba, refusal) in updates {
            let got = textures.update(&mut atlas, owner, &id(name), size, rgba);
            assert_eq!(got, Err(refusal), "update {name} {size:?}");
            assert_eq!(
                state(&textures, &atlas),
                before,
                "update {name} changed nothing"
            );
        }
        assert_eq!(textures.free(&mut atlas, Some("b"), &one), Err(NotOwned));
        assert_eq!(
            textures.free(&mut atlas, Some("c"), &id("c:none")),
            Err(Unknown)
        );
        assert_eq!(state(&textures, &atlas), before);
        // A freed image gives its budget and its room back.
        let rect = atlas.sprites["a:two"];
        textures.free(&mut atlas, Some("a"), &id("a:two")).unwrap();
        assert_eq!(budget(&textures, &atlas, "a"), [2, 1024]);
        textures
            .create(&mut atlas, Some("a"), &id("a:four"), [16, 16], &square)
            .unwrap();
        assert_eq!(texels(atlas.sprites["a:four"]), texels(rect));
    }

    #[test]
    fn an_update_replaces_the_pixels_in_place_or_moves_a_larger_image() {
        let mut atlas = atlas(ORIGIN);
        let mut textures = textures();
        let (img, red, green) = (id("a:img"), [255, 0, 0, 255], [0, 255, 0, 255]);
        let create = |textures: &mut DynamicTextures, atlas: &mut HudAtlas, name| {
            let pixels = pixels([8, 8], red);
            textures.create(atlas, Some("a"), &id(name), [8, 8], &pixels)
        };
        create(&mut textures, &mut atlas, "a:img").unwrap();
        create(&mut textures, &mut atlas, "a:next").unwrap();
        drained(&mut textures, &mut atlas);
        // The same size: new pixels in the same place.
        let revision = atlas.revision;
        textures
            .update(&mut atlas, Some("a"), &img, [8, 8], &pixels([8, 8], green))
            .unwrap();
        assert_eq!(texels(atlas.sprites["a:img"]), [512, 600, 520, 608, 8, 8]);
        let patches = drained(&mut textures, &mut atlas);
        assert_eq!([patches[0].x, patches[0].y], [512, 600]);
        assert_eq!(patches[0].rgba[..4], green);
        // Smaller: still its place, the sprite its new size, and still the same atlas.
        textures
            .update(&mut atlas, Some("a"), &img, [6, 4], &pixels([6, 4], green))
            .unwrap();
        assert_eq!(texels(atlas.sprites["a:img"]), [512, 600, 518, 604, 6, 4]);
        assert_eq!(atlas.revision, revision);
        // Larger than its place: it moves past `a:next`, and its old room comes back.
        textures
            .update(
                &mut atlas,
                Some("a"),
                &img,
                [16, 8],
                &pixels([16, 8], green),
            )
            .unwrap();
        assert_eq!(texels(atlas.sprites["a:img"]), [530, 600, 546, 608, 16, 8]);
        assert_eq!(atlas.revision, revision);
        let patches = drained(&mut textures, &mut atlas);
        let at = [
            patches[0].x,
            patches[0].y,
            patches[0].width,
            patches[0].height,
        ];
        assert_eq!(at, [530, 600, 17, 9]);
        create(&mut textures, &mut atlas, "a:back").unwrap();
        assert_eq!(texels(atlas.sprites["a:back"])[..2], [512, 600]);
        assert_eq!(texels(atlas.sprites["a:next"]), [521, 600, 529, 608, 8, 8]);
    }

    #[test]
    fn a_rebuilt_atlas_gets_every_kept_image_again() {
        let mut atlas = atlas(ORIGIN);
        let mut textures = textures();
        let one = id("a:one");
        let blue = [0, 0, 255, 255];
        textures
            .create(&mut atlas, Some("a"), &one, [4, 2], &pixels([4, 2], blue))
            .unwrap();
        drained(&mut textures, &mut atlas);
        // A resource reload: the page lies elsewhere in the new atlas.
        let mut rebuilt = self::atlas([100, 700]);
        textures.rebind(&mut rebuilt);
        assert_eq!(texels(rebuilt.sprites["a:one"]), [100, 700, 104, 702, 4, 2]);
        let patches = drained(&mut textures, &mut rebuilt);
        assert_eq!(patches.len(), 1);
        assert_eq!([patches[0].x, patches[0].y], [100, 700]);
        assert_eq!(patches[0].rgba[..4], blue);
        // A new renderer's texture has only the packed pixels.
        textures.forget_uploads();
        assert_eq!(drained(&mut textures, &mut rebuilt).len(), 1);
        // The frame's patches bind an atlas the images have not seen, with no `rebind` call.
        let mut later = self::atlas([200, 800]);
        let patches = drained(&mut textures, &mut later);
        assert_eq!(texels(later.sprites["a:one"]), [200, 800, 204, 802, 4, 2]);
        assert_eq!(
            [patches.len() as u32, patches[0].x, patches[0].y],
            [1, 200, 800]
        );
        // An atlas with no page names nothing and refuses every call, keeping the image.
        let mut bare = self::atlas(ORIGIN);
        bare.dynamic_texture_page = None;
        textures.rebind(&mut bare);
        assert!(!bare.sprites.contains_key("a:one"));
        assert_eq!(textures.free(&mut bare, Some("a"), &one), Err(Unavailable));
        assert!(drained(&mut textures, &mut bare).is_empty());
        // A pack's sprite of the same name keeps it, through the image's free too.
        let mut shadowed = self::atlas(ORIGIN);
        let white = shadowed.white;
        shadowed.sprites.insert("a:one".into(), white);
        textures.rebind(&mut shadowed);
        assert_eq!(texels(shadowed.sprites["a:one"]), texels(white));
        textures.free(&mut shadowed, Some("a"), &one).unwrap();
        assert_eq!(texels(shadowed.sprites["a:one"]), texels(white));
    }
}
