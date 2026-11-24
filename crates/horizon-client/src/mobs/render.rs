//! Mob rendering (`LivingEntityRenderer.submit` and the render layers):
//! variant/baby model and texture selection, setupRotations, per-species
//! scale, and extra layers (sheep wool, spider eyes, drowned/stray outer
//! layers, villager overlays, collars...).
use super::{
    Mob, MobData, ModelNormals, Part, Species,
    anim::{self, Poses},
    living::wrap,
};
use crate::mesher::Vertex;
use horizon_core::geometry::trig::Mth;
use std::borrow::Cow;
use std::f32::consts::PI;

// The invisible body a viewer still sees (`Entity.isInvisibleTo`), beside this file.
#[path = "invisible_body.rs"]
mod invisible_body;
pub use invisible_body::render_seen_invisible;

/// A texture lookup into the shared atlas (name without namespace -> uv rect).
pub type Atlas<'a> = &'a dyn Fn(&str) -> Option<[f32; 4]>;

/// The overlay code of an enchanted armour layer's glint, past the ones
/// `gfx/entity_batch.rs` gives materials (13 is the boat water mask, which
/// draws depth only: armour with 13 vanished; 35 is text pulled toward the
/// camera). It marks the `ARMOR_ENTITY_GLINT_TEXTURING` sample: entity.wgsl
/// transforms the model UV and reads `enchanted_glint_armor`.
pub const ARMOR_GLINT: u32 = 36;

/// `RenderTypes.breezeWind`: the texture of the breeze's wind layer scrolls
/// by 0.02 of the texture per tick.
pub const BREEZE_WIND_SCROLL: f32 = 0.02;
/// `WindChargeRenderer.xOffset`: the wind charge's texture scrolls by 0.03 of
/// the texture per tick.
pub const WIND_CHARGE_SCROLL: f32 = 0.03;
/// The offset of a scrolling texture: `RenderTypes.breezeWind` takes the
/// texture's age times the speed, modulo one.
pub fn scroll_offset(age: f32, speed: f32) -> f32 {
    (age * speed) % 1.0
}
/// The vertex's U with the scroll: the texture matrix of
/// `RenderTypes.breezeWind` moves the texture under the model, so the U
/// moves by the offset and wraps around the texture.
fn scroll_u(u: f32, scroll: f32) -> f32 {
    if scroll == 0.0 { u } else { (u + scroll) % 1.0 }
}
/// Full sky and block light in a vertex's `light`.
pub const FULL_BRIGHT: u32 = 240 | 240 << 8;
/// How brightly the eyes render type's layers (spiders', endermen's and
/// phantoms' eyes, the breeze's, the creaking's, the copper golem's) shine in
/// HDR output, as `mesher::hdr_tier`.
const EYES_TIER: u8 = crate::mesher::BRIGHT;

/// Tags quads as drawn by a render type vanilla gives an `OitPipelineSet`
/// (`gfx::OIT_ENTITY`): improved transparency draws them in its stages.
fn mark_oit(quads: &mut [Vertex]) {
    for vertex in quads {
        vertex.layer |= crate::gfx::OIT_ENTITY;
    }
}
/// Whether the species' model is built with `RenderTypes::entityTranslucent`
/// (`Model.renderType`, which `LivingEntityRenderer.getRenderType` uses for the
/// body): the allay, the piglins, the breeze, the vex, the player model and the
/// sulfur cube.
fn translucent_body(m: &Mob) -> bool {
    matches!(
        m.species(),
        Species::Allay
            | Species::Piglin
            | Species::PiglinBrute
            | Species::ZombifiedPiglin
            | Species::Breeze
            | Species::Vex
            | Species::SulfurCube
    ) || avatar(m)
}
/// Whether a layer's render type has an `OitPipelineSet`: `RenderTypes.eyes` and
/// `entityTranslucentEmissive` (the full bright layers), `breezeWind`, and
/// `entityTranslucent` (the slime's outer cube, the sulfur cube's inner cube
/// and the horse's markings).
fn translucent_layer(m: &Mob, layer: &Layer) -> bool {
    layer.full_bright
        || layer.model == "breeze#wind"
        || layer.model == "slime#outer"
        || matches!(m.species(), Species::SulfurCube)
        || layer.texture.starts_with("entity/horse/horse_markings")
}

/// Whether a layer is the eyes render type's (`RenderTypes.eyes`: full bright, no
/// normal, at `EYES_TIER`) on a mob whose other draws are plain: spiders',
/// endermen's, phantoms', the creaking's and the copper golem's. The GPU path
/// poses them as [`super::skin::Skin`] `FLAT` draws when improved transparency
/// is off ([`super::skin::SkinModels::tagged`]).
fn eyes_layer(m: &Mob, layer: &Layer) -> bool {
    layer.full_bright
        && layer.scroll == 0.0
        && matches!(
            m.species(),
            Species::Spider
                | Species::CaveSpider
                | Species::Enderman
                | Species::Phantom
                | Species::Creaking
                | Species::CopperGolem
        )
}

/// One draw of a model with a texture.
pub struct Layer {
    pub model: Cow<'static, str>,
    pub texture: Cow<'static, str>,
    pub color: u32,
    pub full_bright: bool,
    /// `RenderTypes.breezeWind`: the texture scrolls with the entity's age.
    pub scroll: f32,
    /// The slot of what the mob wears that this layer draws (its armour, its body armour), which
    /// a mod's `render_worn_equipment` can hide or tint; None for the mob's own layers.
    pub worn: Option<mod_api::EquipmentRenderSlot>,
}

thread_local! {
    /// What the mods' `render_worn_equipment` left of the worn and held items of the entity
    /// being drawn on this thread ([`EquipmentScope`]); everything shown outside one.
    static SHOWN_EQUIPMENT: std::cell::Cell<mod_api::EquipmentRender> =
        const { std::cell::Cell::new(mod_api::EquipmentRender::new()) };
}

/// Draws one entity's worn and held items as a mod's `render_worn_equipment` left them, until
/// it is dropped: its armour and body layers here, and the game's armour, wings, held and head
/// items, read it ([`shown_equipment`]). Draws outside one (a screen's preview of the player)
/// show everything.
#[must_use]
pub struct EquipmentScope(mod_api::EquipmentRender);
impl EquipmentScope {
    /// Shows the entity's equipment as `shown` says until the scope is dropped.
    pub fn enter(shown: mod_api::EquipmentRender) -> Self {
        Self(SHOWN_EQUIPMENT.replace(shown))
    }
}
impl Drop for EquipmentScope {
    fn drop(&mut self) {
        SHOWN_EQUIPMENT.set(self.0);
    }
}
/// How the entity being drawn shows its equipment ([`EquipmentScope`]).
pub fn shown_equipment() -> mod_api::EquipmentRender {
    SHOWN_EQUIPMENT.get()
}
/// A vertex colour (RGBA8, red in the low byte) times a mod's tint `rgb` (`0xRRGGBB`), each
/// channel times the tint's over 255; the alpha stays.
pub fn tinted(color: u32, rgb: u32) -> u32 {
    if rgb & 0xFF_FFFF == 0xFF_FFFF {
        return color;
    }
    let channel = |c: u32, t: u32| (c & 0xFF) * (t & 0xFF) / 255;
    color & 0xFF00_0000
        | channel(color >> 16, rgb) << 16
        | channel(color >> 8, rgb >> 8) << 8
        | channel(color, rgb >> 16)
}
/// [`tinted`] on every vertex of a slot's draws.
pub fn tint_vertices(vertices: &mut [Vertex], rgb: u32) {
    if rgb & 0xFF_FFFF != 0xFF_FFFF {
        for v in vertices {
            v.color = tinted(v.color, rgb);
        }
    }
}
/// One draw of `slot` in `color` with `overlay`, its base or its trim (`part`), as `shown` lets
/// it be drawn: None when it is hidden, else its colour tinted and its overlay without the
/// armour glint when the glint is hidden.
pub fn shown_draw(
    shown: &mod_api::EquipmentRender,
    slot: mod_api::EquipmentRenderSlot,
    part: mod_api::EquipmentRenderPart,
    color: u32,
    overlay: u32,
) -> Option<(u32, u32)> {
    if shown.hides(slot, part) {
        return None;
    }
    let overlay =
        if overlay == ARMOR_GLINT && shown.hides(slot, mod_api::EquipmentRenderPart::Glint) {
            0
        } else {
            overlay
        };
    Some((tinted(color, shown.tint_color(slot)), overlay))
}
/// DyeColor.getTextureDiffuseColor (RGB).
pub const DYE: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
    0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];
pub const DYE_NAMES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
/// ABGR vertex colour from an RGB value.
pub fn abgr(rgb: u32) -> u32 {
    let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
    0xFF00_0000 | b << 16 | g << 8 | r
}

/// `EquipmentLayerRenderer.getColorForLayer`: a dyeable layer takes the
/// stack's dye colour (`minecraft:dyed_color`), the colour it has when
/// undyed otherwise; a layer that is not dyeable is not tinted.
pub fn armor_tint(undyed: Option<u32>, stack: Option<&crate::items::ItemStack>) -> u32 {
    let Some(undyed) = undyed else {
        return 0xFFFF_FFFF;
    };
    let dyed = stack
        .and_then(|s| s.extra.as_ref())
        .and_then(|e| e.dyed_color)
        .map(|d| d as u32)
        .unwrap_or(undyed);
    abgr(dyed)
}
pub use crate::pack_palette::{diffuse as dye, sheep_wool as sheep_wool_rgb};
/// `ColorLerper.getLerpedColor(ColorLerper.Type.SHEEP, ageInTicks)`: the 16
/// wool colours in DyeColor order, one per 25 ticks, lerped in between. The
/// colour a `jeb_` sheep's wool takes.
pub fn jeb_wool_rgb(age: f32) -> u32 {
    // ageInTicks is the tick count plus the partial tick: never negative.
    let i = age.floor();
    let ticks = i as u64;
    let j = ticks / 25;
    let f = ((ticks % 25) as f32 + (age - i)) / 25.0;
    let lo = sheep_wool_rgb((j % 16) as u8);
    let hi = sheep_wool_rgb(((j + 1) % 16) as u8);
    let mix = |shift: u32| {
        let (lo, hi) = ((lo >> shift & 255) as f32, (hi >> shift & 255) as f32);
        ((lo + (hi - lo) * f) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}
pub const VILLAGER_TYPES: [&str; 7] = [
    "plains", "desert", "jungle", "savanna", "snow", "swamp", "taiga",
];
fn baby_or<'a>(
    m: &Mob,
    adult: impl Into<Cow<'a, str>>,
    baby: impl Into<Cow<'a, str>>,
) -> Cow<'a, str> {
    let baby = baby.into();
    if m.is_baby() && m.kind.layer(&baby).is_some() {
        baby
    } else {
        adult.into()
    }
}
#[path = "villager_client.rs"]
pub mod villager_client;
use villager_client::interned;

/// `AvatarRenderer` entities: a mannequin draws exactly like a player —
/// the player model, its resolved skin and the same scale.
pub fn avatar(m: &Mob) -> bool {
    matches!(&*m.kind.id, "minecraft:player" | "minecraft:mannequin")
}
/// `AvatarRenderer.shouldRenderLayers`: a spectating player or mannequin
/// draws no layers at all (armour, held items, head item, cape, wings,
/// parrots, ears, stuck arrows), only its translucent head.
pub fn layers_shown(m: &Mob) -> bool {
    !(avatar(m) && m.extra.spectator)
}
/// The model layer key and texture for the mob's body.
pub fn body<'a>(m: &'a Mob) -> (Cow<'static, str>, Cow<'a, str>) {
    use Species as S;
    let baby = m.is_baby();
    let b = |s: &'static str| -> Cow<'static, str> {
        if baby {
            interned(format_args!("{s}_baby"))
        } else {
            Cow::Borrowed(s)
        }
    };
    let b_ref = |s: &'a str| -> Cow<'a, str> {
        if baby {
            interned(format_args!("{s}_baby"))
        } else {
            Cow::Borrowed(s)
        }
    };
    if avatar(m) {
        // AvatarRenderer: PlayerModel's slim layer for a slim skin.
        return (
            if m.slim { "player_slim#main" } else { "main" }.into(),
            m.skin.as_deref().unwrap_or(&m.kind.texture).into(),
        );
    }
    match m.species() {
        S::Llama => (
            baby_or(m, "llama#main", "llama_baby#main"),
            super::llama::texture(m).into(),
        ),
        S::TraderLlama => (
            baby_or(m, "trader_llama#main", "trader_llama_baby#main"),
            super::llama::texture(m).into(),
        ),
        s if super::equine::is_equine(s) => {
            let (adult, child) = match s {
                S::Horse => ("horse#main", "horse_baby#main"),
                S::Donkey => ("donkey#main", "donkey_baby#main"),
                S::Mule => ("mule#main", "mule_baby#main"),
                S::SkeletonHorse => ("skeleton_horse#main", "skeleton_horse_baby#main"),
                S::Camel | S::CamelHusk => ("camel#main", "camel_baby#main"),
                _ => ("zombie_horse#main", "zombie_horse_baby#main"),
            };
            (baby_or(m, adult, child), super::equine::texture(m).into())
        }
        S::Wolf => (
            baby_or(m, "wolf#main", "wolf_baby#main"),
            super::tame::texture(m).into(),
        ),
        S::Panda => (
            baby_or(m, "panda#main", "panda_baby#main"),
            super::panda::texture(m).into(),
        ),
        S::HappyGhast => (
            baby_or(m, "happy_ghast#main", "happy_ghast_baby#main"),
            if baby {
                "entity/ghast/happy_ghast_baby".into()
            } else {
                "entity/ghast/happy_ghast".into()
            },
        ),
        S::Nautilus | S::ZombieNautilus => (
            super::nautilus::model(m).into(),
            super::nautilus::texture(m).into(),
        ),
        S::SulfurCube => {
            let (model, texture) = super::sulfur_cube::body(m);
            (model.into(), texture.into())
        }
        S::Sniffer => (
            baby_or(m, "sniffer#main", "sniffer_baby#main"),
            if baby {
                "entity/sniffer/snifflet".into()
            } else {
                "entity/sniffer/sniffer".into()
            },
        ),
        S::Shulker => ("shulker#main".into(), super::shulker::texture(m).into()),
        S::Vex => ("vex#main".into(), super::vex::texture(m).into()),
        S::Wither => ("wither#main".into(), super::wither::texture(m).into()),
        S::Ghast => (
            "ghast#main".into(),
            if m.extra.flag {
                "entity/ghast/ghast_shooting".into()
            } else {
                "entity/ghast/ghast".into()
            },
        ),
        S::Cat => (
            baby_or(m, "cat#main", "cat_baby#main"),
            super::tame::cat_texture(m).into(),
        ),
        S::Frog => (
            "frog#main".into(),
            match m.extra.variant {
                1 => "entity/frog/frog_warm",
                2 => "entity/frog/frog_cold",
                _ => "entity/frog/frog_temperate",
            }
            .into(),
        ),
        // RabbitRenderer.getTextureLocation: the Toast name tag, else the
        // variant (the killer bunny's id is 99).
        S::Rabbit => {
            let tex = if m.custom_name.as_deref() == Some("Toast") {
                if baby {
                    "entity/rabbit/rabbit_toast_baby"
                } else {
                    "entity/rabbit/rabbit_toast"
                }
            } else {
                match (m.extra.variant, baby) {
                    (1, true) => "entity/rabbit/rabbit_white_baby",
                    (1, false) => "entity/rabbit/rabbit_white",
                    (2, true) => "entity/rabbit/rabbit_black_baby",
                    (2, false) => "entity/rabbit/rabbit_black",
                    (3, true) => "entity/rabbit/rabbit_white_splotched_baby",
                    (3, false) => "entity/rabbit/rabbit_white_splotched",
                    (4, true) => "entity/rabbit/rabbit_gold_baby",
                    (4, false) => "entity/rabbit/rabbit_gold",
                    (5, true) => "entity/rabbit/rabbit_salt_baby",
                    (5, false) => "entity/rabbit/rabbit_salt",
                    (99, true) => "entity/rabbit/rabbit_caerbannog_baby",
                    (99, false) => "entity/rabbit/rabbit_caerbannog",
                    (_, true) => "entity/rabbit/rabbit_brown_baby",
                    (_, false) => "entity/rabbit/rabbit_brown",
                }
            };
            (baby_or(m, "rabbit#main", "rabbit_baby#main"), tex.into())
        }
        // ParrotRenderer.getVariantTexture.
        S::Parrot => {
            const PARROTS: [&str; 5] = [
                "entity/parrot/parrot_red_blue",
                "entity/parrot/parrot_blue",
                "entity/parrot/parrot_green",
                "entity/parrot/parrot_yellow_blue",
                "entity/parrot/parrot_grey",
            ];
            (
                m.kind.model_name().into(),
                PARROTS[usize::from(m.extra.variant).min(4)].into(),
            )
        }
        // AxolotlRenderer.getTextureLocation.
        S::Axolotl => {
            const ADULT: [&str; 5] = [
                "entity/axolotl/axolotl_lucy",
                "entity/axolotl/axolotl_wild",
                "entity/axolotl/axolotl_gold",
                "entity/axolotl/axolotl_cyan",
                "entity/axolotl/axolotl_blue",
            ];
            const BABY: [&str; 5] = [
                "entity/axolotl/axolotl_lucy_baby",
                "entity/axolotl/axolotl_wild_baby",
                "entity/axolotl/axolotl_gold_baby",
                "entity/axolotl/axolotl_cyan_baby",
                "entity/axolotl/axolotl_blue_baby",
            ];
            let i = usize::from(m.extra.variant).min(4);
            (
                baby_or(m, "axolotl#main", "axolotl_baby#main"),
                if baby { BABY[i] } else { ADULT[i] }.into(),
            )
        }
        // SalmonRenderer.submit: the model of the salmon's size.
        S::Salmon => {
            let model = match m.extra.variant {
                0 => "salmon_small#main",
                2 => "salmon_large#main",
                _ => "salmon#main",
            };
            (model.into(), "entity/fish/salmon".into())
        }
        // The baby models and textures of AgeableMobRenderer's other
        // subclasses.
        S::Goat => (
            baby_or(m, "goat#main", "goat_baby#main"),
            b_ref(&m.kind.texture),
        ),
        S::PolarBear => (
            baby_or(m, "polar_bear#main", "polar_bear_baby#main"),
            b_ref(&m.kind.texture),
        ),
        S::Ocelot => (
            baby_or(m, "ocelot#main", "ocelot_baby#main"),
            b_ref(&m.kind.texture),
        ),
        S::Turtle => (
            baby_or(m, "turtle#main", "turtle_baby#main"),
            b_ref(&m.kind.texture),
        ),
        S::Armadillo => (
            baby_or(m, "armadillo#main", "armadillo_baby#main"),
            b_ref(&m.kind.texture),
        ),
        // FoxRenderer.getTextureLocation: snow variant, sleeping, baby.
        S::Fox => (
            baby_or(m, "fox#main", "fox_baby#main"),
            match (
                m.extra.variant != 0,
                super::fox::flag(m, super::fox::SLEEPING),
                baby,
            ) {
                (false, false, false) => "entity/fox/fox",
                (false, false, true) => "entity/fox/fox_baby",
                (false, true, false) => "entity/fox/fox_sleep",
                (false, true, true) => "entity/fox/fox_sleep_baby",
                (true, false, false) => "entity/fox/fox_snow",
                (true, false, true) => "entity/fox/fox_snow_baby",
                (true, true, false) => "entity/fox/fox_snow_sleep",
                (true, true, true) => "entity/fox/fox_snow_sleep_baby",
            }
            .into(),
        ),
        S::Pig => {
            let model = if baby {
                "pig_baby#main".into()
            } else if m.extra.variant == 2 {
                "cold_pig#main".into()
            } else {
                "pig#main".into()
            };
            let tex = match (m.extra.variant, baby) {
                (1, true) => "entity/pig/pig_warm_baby",
                (1, false) => "entity/pig/pig_warm",
                (2, true) => "entity/pig/pig_cold_baby",
                (2, false) => "entity/pig/pig_cold",
                (_, true) => "entity/pig/pig_temperate_baby",
                (_, false) => "entity/pig/pig_temperate",
            };
            (model, tex.into())
        }
        S::Cow => {
            let model = match (m.extra.variant, baby) {
                (1, false) => "warm_cow#main",
                (1, true) => "warm_cow_baby#main",
                (2, false) => "cold_cow#main",
                (2, true) => "cold_cow_baby#main",
                (_, true) => "cow_baby#main",
                _ => "cow#main",
            };
            let tex = match (m.extra.variant, baby) {
                (1, true) => "entity/cow/cow_warm_baby",
                (1, false) => "entity/cow/cow_warm",
                (2, true) => "entity/cow/cow_cold_baby",
                (2, false) => "entity/cow/cow_cold",
                (_, true) => "entity/cow/cow_temperate_baby",
                (_, false) => "entity/cow/cow_temperate",
            };
            (model.into(), tex.into())
        }
        S::Mooshroom => (
            baby_or(m, "mooshroom#main", "mooshroom_baby#main"),
            b(if m.extra.variant == 1 {
                "entity/cow/mooshroom_brown"
            } else {
                "entity/cow/mooshroom_red"
            }),
        ),
        S::Chicken => {
            let model = if baby {
                "chicken_baby#main"
            } else if m.extra.variant == 2 {
                "cold_chicken#main"
            } else {
                "chicken#main"
            };
            let tex = match (m.extra.variant, baby) {
                (1, true) => "entity/chicken/chicken_warm_baby",
                (1, false) => "entity/chicken/chicken_warm",
                (2, true) => "entity/chicken/chicken_cold_baby",
                (2, false) => "entity/chicken/chicken_cold",
                (_, true) => "entity/chicken/chicken_temperate_baby",
                (_, false) => "entity/chicken/chicken_temperate",
            };
            (model.into(), tex.into())
        }
        S::Sheep => (
            baby_or(m, "sheep#main", "sheep_baby#main"),
            b("entity/sheep/sheep"),
        ),
        S::Zombie => (
            baby_or(m, "zombie#main", "zombie_baby#main"),
            b("entity/zombie/zombie"),
        ),
        S::Husk => (
            baby_or(m, "husk#main", "husk_baby#main"),
            b("entity/zombie/husk"),
        ),
        S::Drowned => (
            baby_or(m, "drowned#main", "drowned_baby#main"),
            b("entity/zombie/drowned"),
        ),
        S::ZombieVillager => (
            baby_or(m, "zombie_villager#main", "zombie_villager_baby#main"),
            b("entity/zombie_villager/zombie_villager"),
        ),
        S::Villager => (
            baby_or(m, "villager#main", "villager_baby#main"),
            b("entity/villager/villager"),
        ),
        S::Piglin => (
            baby_or(m, "piglin#main", "piglin_baby#main"),
            b("entity/piglin/piglin"),
        ),
        S::ZombifiedPiglin => (
            baby_or(m, "zombified_piglin#main", "zombified_piglin_baby#main"),
            b("entity/piglin/zombified_piglin"),
        ),
        S::Hoglin => (
            baby_or(m, "hoglin#main", "hoglin_baby#main"),
            b("entity/hoglin/hoglin"),
        ),
        S::Zoglin => (
            baby_or(m, "zoglin#main", "zoglin_baby#main"),
            b("entity/hoglin/zoglin"),
        ),
        S::Strider => (
            baby_or(m, "strider#main", "strider_baby#main"),
            b(if m.extra.flag {
                "entity/strider/strider_cold"
            } else {
                "entity/strider/strider"
            }),
        ),
        // BeeRenderer.getTextureLocation: angry, carrying nectar, or both.
        S::Bee => {
            let angry = super::bee::get(m).is_some_and(|b| b.anger_end > 0);
            let nectar = super::bee::has_nectar(m);
            let texture = match (angry, nectar) {
                (true, true) => "entity/bee/bee_angry_nectar",
                (true, false) => "entity/bee/bee_angry",
                (false, true) => "entity/bee/bee_nectar",
                (false, false) => "entity/bee/bee",
            };
            let layer = baby_or(m, "bee#main", "bee_baby#main");
            let texture = if layer.starts_with("bee_baby") {
                b(texture)
            } else {
                texture.into()
            };
            (layer, texture)
        }
        S::Squid => (
            baby_or(m, "squid#main", "squid_baby#main"),
            b("entity/squid/squid"),
        ),
        S::GlowSquid => (
            baby_or(m, "glow_squid#main", "glow_squid_baby#main"),
            b("entity/squid/glow_squid"),
        ),
        S::Dolphin => (
            baby_or(m, "dolphin#main", "dolphin_baby#main"),
            b("entity/dolphin/dolphin"),
        ),
        S::Pufferfish => {
            let model = match m.extra.size {
                0 => "pufferfish_small#main",
                1 => "pufferfish_medium#main",
                _ => "pufferfish_big#main",
            };
            (model.into(), "entity/fish/pufferfish".into())
        }
        S::TropicalFish => {
            if m.extra.variant < 6 {
                (
                    "tropical_fish_small#main".into(),
                    "entity/fish/tropical_a".into(),
                )
            } else {
                (
                    "tropical_fish_large#main".into(),
                    "entity/fish/tropical_b".into(),
                )
            }
        }
        // CopperGolemOxidationLevels: the texture of the weathering.
        S::CopperGolem => (
            m.kind.model_name().into(),
            super::copper_golem::texture(m).into(),
        ),
        _ => (m.kind.model_name().into(), m.kind.texture.as_str().into()),
    }
}
/// LivingEntityRenderer.getModelTint (tropical fish base colour).
pub fn body_color(m: &Mob, alpha: f32) -> u32 {
    match m.species() {
        Species::Wolf => super::tame::tint(m, alpha),
        Species::TropicalFish => abgr(dye(m.extra.variant2 as usize)),
        // LivingEntityRenderer.submit: a spectator is invisible
        // (Player.updateInvisibilityStatus) but not to another spectator or
        // itself, so its body is forced translucent (654311423, alpha 38).
        _ if avatar(m) && m.extra.spectator => 0x26FF_FFFF,
        _ => 0xFFFF_FFFF,
    }
}
/// Extra render layers drawn over the body with the same pose. `age` is the
/// render state's `ageInTicks` (`Mob.age` plus the partial tick); `worn` the
/// game's [`WornArmor`] for the mob ([`humanoid_armor`]).
pub fn layers(m: &Mob, age: f32, worn: &WornArmor) -> Vec<Layer> {
    use Species as S;
    let mut v = Vec::new();
    if avatar(m) {
        return v;
    }
    let white = 0xFFFF_FFFF;
    fn push(
        v: &mut Vec<Layer>,
        model: impl Into<Cow<'static, str>>,
        texture: impl Into<Cow<'static, str>>,
        color: u32,
        full_bright: bool,
    ) {
        v.push(Layer {
            model: model.into(),
            texture: texture.into(),
            color,
            full_bright,
            scroll: 0.0,
            worn: None,
        })
    }
    /// `push` for a layer of what the mob wears in its body slot.
    fn push_body(
        v: &mut Vec<Layer>,
        model: impl Into<Cow<'static, str>>,
        texture: impl Into<Cow<'static, str>>,
        color: u32,
    ) {
        push(v, model, texture, color, false);
        if let Some(layer) = v.last_mut() {
            layer.worn = Some(mod_api::EquipmentRenderSlot::Body);
        }
    }
    match m.species() {
        // WolfArmorLayer, then WolfCollarLayer.
        S::Wolf if m.extra.tame => {
            let (model, texture) = if m.is_baby() {
                ("wolf_baby#main", "entity/wolf/wolf_collar_baby")
            } else {
                ("wolf#main", "entity/wolf/wolf_collar")
            };
            // The armor, then the cracks of its damage (`Crackiness.WOLF_ARMOR`).
            if !m.is_baby() && m.extra.body_id() == Some("minecraft:wolf_armor") {
                let left = (super::WOLF_ARMOR_DURABILITY - m.extra.body_damage()) as f32
                    / super::WOLF_ARMOR_DURABILITY as f32;
                push_body(
                    &mut v,
                    "wolf_armor#main",
                    "entity/equipment/wolf_body/armadillo_scute",
                    white,
                );
                let crack = [(0.32, "high"), (0.69, "medium"), (0.95, "low")]
                    .iter()
                    .find(|(t, _)| left < *t);
                if let Some((_, level)) = crack {
                    push_body(
                        &mut v,
                        "wolf_armor#main",
                        interned(format_args!("entity/wolf/wolf_armor_crackiness_{level}")),
                        white,
                    );
                }
            }
            push(
                &mut v,
                model,
                texture,
                abgr(dye(m.extra.color as usize)),
                false,
            );
        }
        // SimpleEquipmentLayer: the harness over the body.
        S::HappyGhast => {
            let harness = m
                .extra
                .body_id()
                .and_then(super::happy_ghast::harness_texture);
            if !m.is_baby()
                && let Some(texture) = &harness
            {
                push_body(&mut v, "happy_ghast_harness#main", texture.clone(), white);
            }
            // RopesLayer: the ropes over a harnessed body while the ghast
            // holds a quad lead (`isLeashHolder`), the baby's model for a baby.
            if harness.is_some() && super::happy_ghast::is_leash_holder(m) {
                let model = if m.is_baby() {
                    "happy_ghast_baby_ropes#main"
                } else {
                    "happy_ghast_ropes#main"
                };
                push(
                    &mut v,
                    model,
                    "entity/ghast/happy_ghast_ropes",
                    white,
                    false,
                );
            }
        }
        // BreezeWindLayer and BreezeEyesLayer. The wind layer's texture
        // scrolls (`RenderTypes.breezeWind`): the offset is taken inside the
        // sprite, so the atlas does not have to wrap.
        S::Breeze => {
            v.push(Layer {
                model: "breeze#wind".into(),
                texture: "entity/breeze/breeze_wind".into(),
                color: white,
                full_bright: false,
                scroll: scroll_offset(age, BREEZE_WIND_SCROLL),
                worn: None,
            });
            push(
                &mut v,
                "breeze#eyes",
                "entity/breeze/breeze_eyes",
                white,
                true,
            );
        }
        // LlamaDecorLayer: the carpet, or the trader's blanket.
        S::Llama | S::TraderLlama => {
            if let Some((model, texture)) = super::llama::decor(m) {
                // The carpet is worn in the body slot; the trader's own blanket is not.
                if super::llama::carpet(m).is_some() && !m.is_baby() {
                    push_body(&mut v, model, texture, white);
                } else {
                    push(&mut v, model, texture, white, false);
                }
            }
        }
        // HorseMarkingLayer, the saddle and the horse armour.
        s if super::equine::is_equine(s) => {
            let name = super::equine::model_name(s);
            if let Some(tex) = super::equine::markings(m) {
                let body = if m.is_baby() {
                    interned(format_args!("{name}_baby#main"))
                } else {
                    interned(format_args!("{name}#main"))
                };
                push(&mut v, body, tex, white, false);
            }
            if m.extra.saddled() && !m.is_baby() {
                push(
                    &mut v,
                    interned(format_args!("{name}#saddle")),
                    super::equine::saddle_texture(s),
                    white,
                    false,
                );
            }
            if let Some(armor) = m.extra.body_id()
                && let Some(material) = armor
                    .strip_prefix("minecraft:")
                    .and_then(super::equine::horse_armor_asset)
            {
                let tex = interned(format_args!("entity/equipment/horse_body/{material}"));
                let color = if material == "leather" {
                    abgr(0xA06540)
                } else {
                    white
                };
                // UndeadHorseRenderer bakes its own armour layer.
                let layer = if matches!(s, S::ZombieHorse | S::SkeletonHorse) {
                    "undead_horse_armor#main"
                } else {
                    "horse_armor#main"
                };
                push_body(&mut v, layer, tex, color);
            }
        }
        // SimpleEquipmentLayer: pig and strider saddles.
        S::Pig if m.extra.saddled() && !m.is_baby() => push(
            &mut v,
            "pig#saddle",
            "entity/equipment/pig_saddle/saddle",
            white,
            false,
        ),
        S::Strider if m.extra.saddled() && !m.is_baby() => push(
            &mut v,
            "strider#saddle",
            "entity/equipment/strider_saddle/saddle",
            white,
            false,
        ),
        // CatCollarLayer.
        S::Cat if m.extra.tame => {
            let (model, texture) = if m.is_baby() {
                ("cat_baby#collar", "entity/cat/cat_collar_baby")
            } else {
                ("cat#collar", "entity/cat/cat_collar")
            };
            push(
                &mut v,
                model,
                texture,
                abgr(dye(m.extra.color as usize)),
                false,
            );
        }
        // IronGolemCrackinessLayer.
        S::IronGolem => {
            if let Some(texture) = super::golem::crackiness(m) {
                push(&mut v, "iron_golem#main", texture, white, false);
            }
        }
        // NautilusRenderer: the armour and saddle SimpleEquipmentLayers.
        S::Nautilus | S::ZombieNautilus if !m.is_baby() => {
            if let Some(texture) = m.extra.body_id().and_then(super::nautilus::armor_texture) {
                push_body(&mut v, "nautilus_armor#main", texture, white);
            }
            if m.extra.saddled() {
                push(
                    &mut v,
                    "nautilus#saddle",
                    "entity/equipment/nautilus_saddle/saddle",
                    white,
                    false,
                );
            }
        }
        // SulfurCubeInnerLayer.
        S::SulfurCube => {
            if let Some((model, texture)) = super::sulfur_cube::inner(m) {
                push(&mut v, model, texture, white, false);
            }
        }
        // SlimeOuterLayer: the translucent outer cube.
        S::Slime => push(&mut v, "slime#outer", "entity/slime/slime", white, false),
        // SheepWoolUndercoatLayer has no isSheared guard: a sheared dyed sheep
        // keeps it, while only SheepWoolLayer stops.
        S::Sheep => {
            // MobRenderer.checkMagicName(entity, "jeb_"): an exact custom name.
            let jeb = m.custom_name.as_deref() == Some("jeb_");
            let color = abgr(if jeb {
                jeb_wool_rgb(age)
            } else {
                sheep_wool_rgb(m.extra.color)
            });
            if !m.is_baby() && (jeb || m.extra.color != 0) {
                push(
                    &mut v,
                    "sheep#wool_undercoat",
                    "entity/sheep/sheep_wool_undercoat",
                    color,
                    false,
                );
            }
            // SheepWoolLayer: none once sheared.
            if !m.extra.sheared {
                let (model, texture) = if m.is_baby() {
                    ("sheep_baby#wool", "entity/sheep/sheep_wool_baby")
                } else {
                    ("sheep#wool", "entity/sheep/sheep_wool")
                };
                push(&mut v, model, texture, color, false);
            }
        }
        S::Creaking if super::creaking::eyes_glowing(m) => push(
            &mut v,
            "creaking#eyes",
            "entity/creaking/creaking_eyes",
            white,
            true,
        ),
        // The copper golem's eyes glow: CopperGolemRenderer's
        // LivingEntityEmissiveLayer (eyes render type) draws on a model baked
        // from ModelLayers.COPPER_GOLEM, the body's, not COPPER_GOLEM_EYES.
        S::CopperGolem => {
            let eyes = super::copper_golem::eye_texture(m);
            push(&mut v, m.kind.model_name(), eyes, white, true);
        }
        S::Spider | S::CaveSpider => push(
            &mut v,
            m.kind.model_name(),
            "entity/spider/spider_eyes",
            white,
            true,
        ),
        S::Enderman => push(
            &mut v,
            "enderman#main",
            "entity/enderman/enderman_eyes",
            white,
            true,
        ),
        S::Phantom => push(
            &mut v,
            "phantom#main",
            "entity/phantom/phantom_eyes",
            white,
            true,
        ),
        S::Drowned => {
            if m.is_baby() {
                push(
                    &mut v,
                    "drowned_baby#outer",
                    "entity/zombie/drowned_outer_layer_baby",
                    white,
                    false,
                );
            } else {
                push(
                    &mut v,
                    "drowned#outer",
                    "entity/zombie/drowned_outer_layer",
                    white,
                    false,
                );
            }
        }
        S::Stray => push(
            &mut v,
            "stray#outer",
            "entity/skeleton/stray_overlay",
            white,
            false,
        ),
        S::Bogged if !m.extra.sheared => push(
            &mut v,
            "bogged#outer",
            "entity/skeleton/bogged_overlay",
            white,
            false,
        ),
        S::TropicalFish => {
            // TropicalFishPatternLayer: the pattern tinted with the pattern colour.
            let (model, letter, n) = if m.extra.variant < 6 {
                ("tropical_fish_small#pattern", "a", m.extra.variant + 1)
            } else {
                ("tropical_fish_large#pattern", "b", m.extra.variant - 5)
            };
            push(
                &mut v,
                model,
                interned(format_args!("entity/fish/tropical_{letter}_pattern_{n}")),
                abgr(dye(m.extra.variant3 as usize)),
                false,
            );
        }
        S::Villager | S::ZombieVillager => {
            const VILLAGER_TYPE_ADULT: [&str; 7] = [
                "entity/villager/type/plains",
                "entity/villager/type/desert",
                "entity/villager/type/jungle",
                "entity/villager/type/savanna",
                "entity/villager/type/snow",
                "entity/villager/type/swamp",
                "entity/villager/type/taiga",
            ];
            const VILLAGER_TYPE_BABY: [&str; 7] = [
                "entity/villager/baby/plains",
                "entity/villager/baby/desert",
                "entity/villager/baby/jungle",
                "entity/villager/baby/savanna",
                "entity/villager/baby/snow",
                "entity/villager/baby/swamp",
                "entity/villager/baby/taiga",
            ];
            const ZOMBIE_TYPE_ADULT: [&str; 7] = [
                "entity/zombie_villager/type/plains",
                "entity/zombie_villager/type/desert",
                "entity/zombie_villager/type/jungle",
                "entity/zombie_villager/type/savanna",
                "entity/zombie_villager/type/snow",
                "entity/zombie_villager/type/swamp",
                "entity/zombie_villager/type/taiga",
            ];
            const ZOMBIE_TYPE_BABY: [&str; 7] = [
                "entity/zombie_villager/baby/plains",
                "entity/zombie_villager/baby/desert",
                "entity/zombie_villager/baby/jungle",
                "entity/zombie_villager/baby/savanna",
                "entity/zombie_villager/baby/snow",
                "entity/zombie_villager/baby/swamp",
                "entity/zombie_villager/baby/taiga",
            ];
            const VILLAGER_LEVELS: [&str; 5] = [
                "entity/villager/profession_level/stone",
                "entity/villager/profession_level/iron",
                "entity/villager/profession_level/gold",
                "entity/villager/profession_level/emerald",
                "entity/villager/profession_level/diamond",
            ];
            const ZOMBIE_LEVELS: [&str; 5] = [
                "entity/zombie_villager/profession_level/stone",
                "entity/zombie_villager/profession_level/iron",
                "entity/zombie_villager/profession_level/gold",
                "entity/zombie_villager/profession_level/emerald",
                "entity/zombie_villager/profession_level/diamond",
            ];
            let zombie = m.species() == S::ZombieVillager;
            let baby = m.is_baby();
            let model = match (zombie, baby) {
                (false, false) => "villager#main",
                (false, true) => "villager_baby#main",
                (true, false) => "zombie_villager#main",
                (true, true) => "zombie_villager_baby#main",
            };
            let ti = m.extra.variant as usize % VILLAGER_TYPE_ADULT.len();
            // VillagerProfessionLayer: a baby's type texture is in `baby/`.
            let type_tex = match (zombie, baby) {
                (false, false) => VILLAGER_TYPE_ADULT[ti],
                (false, true) => VILLAGER_TYPE_BABY[ti],
                (true, false) => ZOMBIE_TYPE_ADULT[ti],
                (true, true) => ZOMBIE_TYPE_BABY[ti],
            };
            push(&mut v, model, type_tex, white, false);
            let nitwit = m.extra.profession.index() == Some(11);
            if !baby && let Some(texture) = villager_client::texture(&m.extra.profession, zombie) {
                push(&mut v, model, texture, white, false);
                if !nitwit {
                    // `LEVEL_LOCATIONS` clamps the level to 1..=5.
                    let li = (m.extra.villager_level.max(1) as usize - 1).min(4);
                    let level_tex = if zombie {
                        ZOMBIE_LEVELS[li]
                    } else {
                        VILLAGER_LEVELS[li]
                    };
                    push(&mut v, model, level_tex, white, false);
                }
            }
        }
        _ => {}
    }
    humanoid_armor(m, worn, &mut v);
    v
}
/// A mob's worn armour, resolved by the game from each stack's `equippable`
/// asset and the equipment table: per slot (`equipment::ARMOR`'s order: head,
/// chest, legs, feet) the asset's layers in vanilla's layer type for that mob,
/// each (texture, the colour it has when undyed) — the table's own lists, lent
/// for the frame. A stack without an asset wears no layer at all.
pub type WornArmor<'a> = [Option<&'a [(String, Option<u32>)]>; 4];
/// `HumanoidArmorLayer`: the armour models of the mob's main model with the
/// equipment layers `worn` resolved (`EquipmentLayerRenderer.renderLayers`):
/// a dyeable layer takes the stack's dye colour, the colour it has when undyed
/// otherwise, and a layer keeps its place in the asset's own order (leather's
/// overlay is its second layer).
fn humanoid_armor(m: &Mob, worn: &WornArmor, v: &mut Vec<Layer>) {
    // A player's or mannequin's armour is drawn by the game's armour layers.
    if super::equine::is_equine(m.species()) || avatar(m) {
        return;
    }
    if worn.iter().all(Option::is_none) {
        return;
    }
    let (model, _) = body(m);
    let Some(base) = model.strip_suffix("#main") else {
        return;
    };
    use mod_api::EquipmentRenderSlot as Worn;
    let slots = [
        ("helmet", Worn::Head),
        ("chestplate", Worn::Chest),
        ("leggings", Worn::Legs),
        ("boots", Worn::Feet),
    ];
    for (i, (slot, (layer, part))) in horizon_items::equipment::ARMOR
        .into_iter()
        .zip(slots)
        .enumerate()
    {
        let Some(layers) = worn[i] else {
            continue;
        };
        let stack = m.extra.slot_stack(slot);
        let model = interned(format_args!("{base}#{layer}"));
        for (texture, undyed) in layers {
            v.push(Layer {
                model: model.clone(),
                texture: interned(format_args!("{texture}")),
                color: armor_tint(*undyed, stack),
                full_bright: false,
                scroll: 0.0,
                worn: Some(part),
            });
        }
    }
}
impl super::MobType {
    /// The main model layer key without the namespace.
    pub fn model_key(&self) -> String {
        self.model_name().to_owned()
    }
    /// `model_key` without the copy.
    pub fn model_name(&self) -> &'static str {
        super::species::def(self.species).model
    }
}
/// The mob's root transform (LivingEntityRenderer.submit + setupRotations).
pub fn base_transform(m: &Mob, alpha: f32, camera: glam::DVec3) -> glam::Mat4 {
    let a = f64::from(alpha);
    let p = glam::DVec3::new(
        m.previous.x + (m.position.x - m.previous.x) * a,
        m.previous.y + (m.position.y - m.previous.y) * a,
        m.previous.z + (m.position.z - m.previous.z) * a,
    ) - camera
        + enderman_render_offset(m, alpha)
        + super::shulker::render_offset(m, alpha);
    base_transform_at(m, alpha, glam::Mat4::from_translation(p.as_vec3()))
}
/// `EndermanRenderer.getRenderOffset`: a creepy (angry) enderman is moved by
/// a gaussian 0.02 blocks sideways in x and z every frame. Vanilla draws from
/// the renderer's own random; this one is seeded by the mob and the frame's
/// age, so the body, its carried block and its eyes agree within a frame.
fn enderman_render_offset(m: &Mob, alpha: f32) -> glam::DVec3 {
    use horizon_core::random::{LegacyRandom, RandomSource};
    if m.species() != Species::Enderman || !m.extra.flag {
        return glam::DVec3::ZERO;
    }
    let frame = (m.age as f32 + alpha).to_bits();
    let mut random = LegacyRandom::new((i64::from(m.id) << 32) ^ i64::from(frame));
    // `0.02 * state.scale`.
    let d = 0.02 * f64::from(m.model_scale());
    let x = random.next_gaussian() * d;
    let z = random.next_gaussian() * d;
    glam::DVec3::new(x, 0.0, z)
}
/// `base_transform` from a pose of the entity's origin (`EntityRenderDispatcher.submit`
/// under a pose stack, as `SpawnerRenderer` draws the mob inside a spawner).
pub fn base_transform_at(m: &Mob, alpha: f32, origin: glam::Mat4) -> glam::Mat4 {
    let mut body = anim::body_rot(m, alpha);
    if shaking(m) {
        // LivingEntityRenderer.setupRotations: a rapid wobble.
        let ticks = (m.age as f32 + alpha).floor();
        body += (f64::from(ticks * 3.25).cos() * std::f64::consts::PI * 0.4) as f32;
    }
    // A sleeping villager, or another player lying in their bed.
    let sleep = (m.species() == Species::Villager || avatar(m))
        .then(|| super::villager::sleep_pose(m))
        .flatten();
    // `LivingEntityRenderer.submit` moves a sleeper toward the pillow before
    // it scales the model, so the bed offset is not scaled and the pose after
    // it is. Keep the two apart.
    let (sleep_offset, rotation) = match sleep {
        Some((angle, step)) => {
            // The eye height of the standing pose (LivingEntityRenderer
            // extracts `getEyeHeight(Pose.STANDING)` for a bed).
            let eye = if avatar(m) {
                // `getEyeHeight(Pose.STANDING)` on an avatar is the standing
                // box's eye height, `minecraft:scale` included.
                m.kind.eye_height as f32 * m.model_scale()
            } else {
                m.awake_dims().2 as f32
            };
            let head = eye - 0.1;
            (
                glam::Mat4::from_translation(glam::Vec3::new(
                    -step[0] * head,
                    0.0,
                    -step[1] * head,
                )),
                // setupRotations for Pose.SLEEPING.
                glam::Mat4::from_rotation_y(angle.to_radians())
                    * glam::Mat4::from_rotation_z(90.0_f32.to_radians())
                    * glam::Mat4::from_rotation_y(270.0_f32.to_radians()),
            )
        }
        None => (
            glam::Mat4::IDENTITY,
            super::aquatic::rotations(m, alpha, body)
                .unwrap_or_else(|| glam::Mat4::from_rotation_y((180.0 - body).to_radians())),
        ),
    };
    // `LivingEntityRenderer.submit`: `poseStack.scale(state.scale, ...)` for
    // every living entity, after the sleeping offset and before
    // setupRotations, so a pose translation scales with the model but the bed
    // it lies in does not.
    let mut base = origin
        * sleep_offset
        * glam::Mat4::from_scale(glam::Vec3::splat(m.model_scale()))
        * pufferfish_bob(m, alpha)
        * rotation;
    if m.death_time > 0 && !super::creaking::tearing_down(m) {
        let f = ((m.death_time as f32 + alpha - 1.0) / 20.0 * 1.6)
            .sqrt()
            .min(1.0);
        base *= glam::Mat4::from_rotation_z(f * flip_degrees(m).to_radians());
    } else if m.extra.auto_spin_attack {
        // Riptide: lying along the look direction, spinning 75 degrees a tick.
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        base *= glam::Mat4::from_rotation_x((-90.0 - pitch).to_radians())
            * glam::Mat4::from_rotation_y(((m.age as f32 + alpha) * -75.0).to_radians());
    } else if anim::upside_down(m) {
        // isEntityUpsideDown
        base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, m.height() + 0.1, 0.0))
            * glam::Mat4::from_rotation_z(PI);
    }
    // IronGolemRenderer.setupRotations, after LivingEntityRenderer's: the body
    // tilts side to side while it walks.
    if m.species() == Species::IronGolem {
        let s = anim::state(m, alpha);
        if s.speed >= 0.01 {
            let wp = s.pos + 6.0;
            let triangle_wave = ((wp % 13.0 - 6.5).abs() - 3.25) / 3.25;
            base *= glam::Mat4::from_rotation_z((6.5 * triangle_wave).to_radians());
        }
    }
    // AvatarRenderer.setupRotations, after LivingEntityRenderer's.
    if m.extra.fall_flying {
        let ticks = m.extra.fall_fly_ticks as f32 + alpha;
        let scale = (ticks * ticks / 100.0).min(1.0);
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        if !m.extra.auto_spin_attack {
            base *= glam::Mat4::from_rotation_x((scale * (-90.0 - pitch)).to_radians());
        }
        let yaw = m.previous_head_yaw + wrap(m.head_yaw - m.previous_head_yaw) * alpha;
        let (look_x, look_z) = (-(yaw.to_radians()).msin(), (yaw.to_radians()).mcos());
        let v = m.extra.previous_velocity.lerp(m.velocity, f64::from(alpha));
        let view_scale = (pitch.to_radians()).mcos();
        let (look_x, look_z) = (look_x * view_scale, look_z * view_scale);
        let view_length = f64::from(look_x * look_x + look_z * look_z);
        let speed = v.x * v.x + v.z * v.z;
        if speed > f64::from(1.0e-5f32) && view_length > f64::from(1.0e-5f32) {
            let dot =
                (v.x * f64::from(look_x) + v.z * f64::from(look_z)) / (speed * view_length).sqrt();
            let sign = (v.x * f64::from(look_z) - v.z * f64::from(look_x)).signum();
            base *= glam::Mat4::from_rotation_y(
                (sign * horizon_core::jmath::acos(dot.abs().min(1.0))) as f32,
            );
        }
    } else if m.species() == Species::Drowned {
        // DrownedRenderer.setupRotations: tip toward the look pitch about
        // the middle of the box while swimming.
        let amount = m.extra.swim.amount(alpha);
        if amount > 0.0 {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            let middle = glam::Vec3::new(0.0, m.height() / 2.0, 0.0);
            base *= glam::Mat4::from_translation(middle)
                * glam::Mat4::from_rotation_x((amount * (-10.0 - pitch)).to_radians())
                * glam::Mat4::from_translation(-middle);
        }
    } else if m.extra.swim.amount(alpha) > 0.0 {
        // AvatarRenderer.setupRotations: tip forward with the swim amount,
        // following the pitch in water, then lie along the swimming box.
        let amount = m.extra.swim.amount(alpha);
        let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
        let target = if m.extra.swim.in_water {
            -90.0 - pitch
        } else {
            -90.0
        };
        base *= glam::Mat4::from_rotation_x((amount * target).to_radians());
        if m.extra.swim.visually {
            base *= glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.0, 0.3));
        }
    }
    base *= species_rotations(m, alpha);
    base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
    base *= species_scale(m, alpha);
    base * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0))
}
/// `PufferfishRenderer.setupRotations`: `translate(0, cos(ageInTicks * 0.05) * 0.08, 0)`
/// before the usual rotations, so a pufferfish bobs up and down.
fn pufferfish_bob(m: &Mob, alpha: f32) -> glam::Mat4 {
    if m.species() != Species::Pufferfish {
        return glam::Mat4::IDENTITY;
    }
    glam::Mat4::from_translation(glam::Vec3::new(
        0.0,
        pufferfish_bob_y(m.age as f32 + alpha),
        0.0,
    ))
}
fn pufferfish_bob_y(age_in_ticks: f32) -> f32 {
    (age_in_ticks * 0.05).mcos() * 0.08
}
/// `LivingEntityRenderer.isShaking`: a fully frozen mob (the skeleton's own
/// `isShaking` is its stray conversion only), converting zombies, piglins
/// and hoglins, a zombie villager being cured and a cold strider.
fn shaking(m: &Mob) -> bool {
    if m.is_fully_frozen() && m.species() != Species::Skeleton {
        return true;
    }
    match m.species() {
        Species::Strider => super::strider::suffocating(m),
        Species::ZombieVillager => super::zombie_villager::is_converting(m),
        Species::Zombie | Species::Husk | Species::Skeleton => m.extra.counter2 > 0,
        Species::Piglin | Species::PiglinBrute | Species::Hoglin => {
            !m.extra.flag2 && m.extra.counter2 > 0
        }
        _ => false,
    }
}
fn flip_degrees(m: &Mob) -> f32 {
    match m.species() {
        Species::Spider | Species::CaveSpider | Species::Silverfish | Species::Endermite => 180.0,
        _ => 90.0,
    }
}
fn species_rotations(m: &Mob, alpha: f32) -> glam::Mat4 {
    match m.species() {
        Species::Cat => super::tame::cat_rotations(m, alpha),
        Species::Panda => super::panda::rotations(m, alpha),
        Species::Shulker => super::shulker::rotations(m),
        // FoxRenderer.setupRotations: the body follows a pounce's pitch.
        Species::Fox if super::fox::flag(m, super::fox::POUNCING | super::fox::FACEPLANTED) => {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            glam::Mat4::from_rotation_x((-pitch).to_radians())
        }
        // PhantomRenderer.setupRotations: the body follows its pitch.
        Species::Phantom => {
            let pitch = m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha;
            glam::Mat4::from_rotation_x(pitch.to_radians())
        }
        _ => glam::Mat4::IDENTITY,
    }
}
/// Creeper.getSwelling: the fuse's progress, past 1 in its last two ticks.
fn creeper_swelling(m: &Mob, alpha: f32) -> f32 {
    (m.previous_swell as f32 + alpha * (m.swell - m.previous_swell) as f32)
        / (m.max_swell - 2) as f32
}
/// LivingEntityRenderer.getOverlayCoords for the body: the hurt red, or a
/// swelling creeper's white flash. Layers take the red only (progress 0).
fn body_overlay(m: &Mob, alpha: f32) -> u32 {
    let white = if m.species() == Species::Creeper {
        crate::overlay::creeper_progress(creeper_swelling(m, alpha))
    } else {
        0.0
    };
    crate::overlay::living(white, red_overlay(m))
}
/// LivingEntityRenderState.hasRedOverlay: hurt or dying (not a creaking
/// tearing down).
pub fn red_overlay(m: &Mob) -> bool {
    m.hurt_time > 0 || (m.death_time > 0 && !super::creaking::tearing_down(m))
}
fn species_scale(m: &Mob, alpha: f32) -> glam::Mat4 {
    if avatar(m) {
        // AvatarRenderer.scale, shared by the inventory and world renderer.
        // The entity's own `scale` is `LivingEntityRenderer.submit`'s, applied
        // by `base_transform_at` before setupRotations.
        return glam::Mat4::from_scale(glam::Vec3::splat(0.9375));
    }
    match m.species() {
        Species::Wither => {
            glam::Mat4::from_scale(glam::Vec3::splat(super::wither::scale(m, alpha)))
        }
        Species::Creeper => {
            // CreeperRenderer.scale: swelling with a wobble.
            let g = creeper_swelling(m, alpha);
            let wobble = 1.0 + (g * 100.0).msin() * g * 0.01;
            let s = g.clamp(0.0, 1.0);
            let s4 = s * s * s * s;
            let xz = (1.0 + s4 * 0.4) * wobble;
            let y = (1.0 + s4 * 0.1) / wobble;
            glam::Mat4::from_scale(glam::Vec3::new(xz, y, xz))
        }
        Species::Slime | Species::MagmaCube => super::cube::scale(m, alpha),
        Species::SulfurCube => super::sulfur_cube::scale(m, alpha),
        Species::Phantom => {
            // PhantomRenderer.scale: 15% bigger per size, and the model moved
            // down to the box (it is modelled 1.3125 blocks above the feet).
            let s = 1.0 + 0.15 * m.extra.size as f32;
            glam::Mat4::from_scale(glam::Vec3::splat(s))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 1.3125, 0.1875))
        }
        _ => glam::Mat4::IDENTITY,
    }
}
/// Draw a part tree with a pose map, texture rect, colour and light.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    part: &Part,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    draw_scrolled(part, base, poses, uv, 0.0, color, light, overlay, out);
}
/// `draw` for a texture that scrolls: the vertex's U is moved by the offset
/// and wraps inside the texture, as the texture matrix of
/// `RenderTypes.breezeWind` does.
#[allow(clippy::too_many_arguments)]
pub fn draw_scrolled(
    part: &Part,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    draw_normals(
        part,
        base,
        ModelNormals::new(base),
        poses,
        uv,
        scroll,
        color,
        light,
        overlay,
        out,
    );
}
#[allow(clippy::too_many_arguments)]
fn draw_normals(
    part: &Part,
    base: glam::Mat4,
    normals: ModelNormals,
    poses: &Poses,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    let p = poses.pose(part);
    let m = posed(base, p);
    if poses.is_hidden(part) || p[6] == 0.0 && p[7] == 0.0 && p[8] == 0.0 {
        return;
    }
    let normals = normals.part(p);
    // `skips` is a name lookup: ask once per part, not once per polygon. A part
    // hidden this way still draws its children, so only the polygons are gated.
    if !poses.skips(part) {
        for poly in &part.polygons {
            let normal = poly.vertex_normal(normals);
            for v in poly.vertices {
                let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
                // The armour glint scrolls the layer's model UV in the shader;
                // the item path already passed its transformed UV.
                let glint_uv = if overlay == ARMOR_GLINT {
                    [v[3], v[4]]
                } else {
                    [0.0, 0.0]
                };
                out.push(Vertex {
                    normal,
                    pos: pos.to_array(),
                    uv: [
                        uv[0] + (uv[2] - uv[0]) * scroll_u(v[3], scroll),
                        uv[1] + (uv[3] - uv[1]) * v[4],
                    ],
                    layer: overlay,
                    color,
                    light,
                    glint_uv,
                });
            }
        }
    }
    for c in &part.children {
        draw_normals(c, m, normals, poses, uv, scroll, color, light, overlay, out);
    }
}
/// A part's matrix under `base` for its pose (`ModelPart.translateAndRotate`
/// and its scale), as `draw` and the GPU path (`super::skin`) both use it.
pub(super) fn posed(base: glam::Mat4, p: [f32; 9]) -> glam::Mat4 {
    base * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]))
}

/// Keeps the original multiplication order when the local rotation was reused.
pub(super) fn posed_rotated(base: glam::Mat4, p: [f32; 9], rotation: glam::Mat4) -> glam::Mat4 {
    base * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * rotation
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]))
}
/// `draw_scrolled` through the GPU path when the mob takes it (`skin`), on
/// the CPU otherwise.
#[allow(clippy::too_many_arguments)]
pub fn put(
    skin: &mut Option<&mut super::skin::Skin>,
    part: &std::sync::Arc<Part>,
    base: glam::Mat4,
    poses: &Poses,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    out: &mut Vec<Vertex>,
) {
    let paint = super::skin::Paint {
        uv,
        scroll,
        color,
        light,
        overlay,
    };
    if let Some(skin) = skin
        && skin.draw(part, base, poses, paint)
    {
        return;
    }
    draw_scrolled(part, base, poses, uv, scroll, color, light, overlay, out);
}
/// Render a mob with its variant model, texture and layers.
pub fn render(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    render_with(
        m,
        alpha,
        camera,
        atlas,
        light,
        data,
        &Default::default(),
        &[],
        out,
    );
}
/// `render` plus extra layers posed like the body (`HumanoidArmorLayer`:
/// the armour model's parts copy the body's): (model, texture rect, colour,
/// overlay: 0 or `ARMOR_GLINT`, as `EquipmentLayerRenderer` never tints them).
/// `worn` is the mob's [`WornArmor`].
#[allow(clippy::too_many_arguments)]
pub fn render_with(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    worn: &WornArmor,
    extra: &[(&Part, [f32; 4], u32, u32, bool)],
    out: &mut Vec<Vertex>,
) {
    render_with_hidden(
        m,
        alpha,
        camera,
        atlas,
        light,
        data,
        worn,
        extra,
        Default::default(),
        None,
        out,
    );
}
/// [`render_with`], with the parts a mod's `render_player_pre` hid left out of the body (after Skin
/// Customization) and the layers on its own model; the armour in `extra` and the other layers keep
/// every part. `parts`, given while the mods' post hooks want them, gets the humanoid parts as the
/// body was drawn ([`PosedParts`](mod_api::PosedParts)); it stays None for a mob that isn't a
/// humanoid or isn't drawn.
#[allow(clippy::too_many_arguments)]
pub fn render_with_hidden(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    worn: &WornArmor,
    extra: &[(&Part, [f32; 4], u32, u32, bool)],
    hidden: mod_api::HiddenParts,
    mut parts: Option<&mut Option<mod_api::PosedParts>>,
    out: &mut Vec<Vertex>,
) {
    let paint = Some(body_color(m, alpha));
    // The invisibility effect hides the model (a drinking wandering trader);
    // an illusioner shows its four mirror images instead.
    if m.is_invisible() {
        if m.species() == Species::Illusioner {
            let age = m.age as f32 + alpha;
            for (i, off) in super::illusioner::offsets(m, alpha).iter().enumerate() {
                let w = super::illusioner::wobble(i, age);
                let shifted =
                    camera - glam::DVec3::new(off[0] + w[0], off[1] + w[1], off[2] + w[2]);
                let base = base_transform(m, alpha, shifted);
                render_visible(
                    m,
                    alpha,
                    base,
                    atlas,
                    light,
                    data,
                    worn,
                    extra,
                    paint,
                    hidden,
                    parts.as_deref_mut(),
                    out,
                    None,
                );
            }
        }
        return;
    }
    render_visible(
        m,
        alpha,
        base_transform(m, alpha, camera),
        atlas,
        light,
        data,
        worn,
        extra,
        paint,
        hidden,
        parts,
        out,
        None,
    );
}
/// `render` with the model's ordinary draws posed on the GPU when the mob
/// allows it (`super::skin`): the same picture, no vertices on the CPU.
/// `hidden` and `parts` as for [`render_with_hidden`].
#[allow(clippy::too_many_arguments)]
pub fn render_skinned(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    worn: &WornArmor,
    hidden: mod_api::HiddenParts,
    parts: Option<&mut Option<mod_api::PosedParts>>,
    out: &mut Vec<Vertex>,
    skin: &mut super::skin::Skin,
) {
    if m.is_invisible() {
        render_with_hidden(
            m,
            alpha,
            camera,
            atlas,
            light,
            data,
            worn,
            &[],
            hidden,
            parts,
            out,
        );
        return;
    }
    render_visible(
        m,
        alpha,
        base_transform(m, alpha, camera),
        atlas,
        light,
        data,
        worn,
        &[],
        Some(body_color(m, alpha)),
        hidden,
        parts,
        out,
        Some(skin),
    );
}
/// `CapeLayer.submit` with `PlayerCapeModel`: the player model's body posed
/// as the player's, its cape turned by `ModelPart.rotateBy` with the cape's
/// flap and leans. `base` is the player's root transform (moved out over a
/// chestplate); `uv` the cape texture.
#[allow(clippy::too_many_arguments)]
pub fn cape(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    uv: [f32; 4],
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    let Some(model) = m.kind.layer("player#cape") else {
        return;
    };
    let s = anim::state(m, alpha);
    let mut poses = Poses::pooled();
    poses.reset(model);
    anim::setup(m, &s, &mut poses, &data.animations);
    let a = f64::from(alpha);
    let at = [
        m.previous.x + (m.position.x - m.previous.x) * a,
        m.previous.y + (m.position.y - m.previous.y) * a,
        m.previous.z + (m.position.z - m.previous.z) * a,
    ];
    let body = anim::body_rot(m, alpha);
    // LivingEntityRenderState.fallFlyingScale.
    let ticks = m.extra.fall_fly_ticks as f32 + alpha;
    let gliding = (ticks * ticks / 100.0).clamp(0.0, 1.0);
    let [flap, lean, lean2] = m.extra.avatar.cape(alpha, at, body, gliding);
    let pose = poses.get("cape");
    let by = glam::Quat::from_rotation_y(-PI)
        * glam::Quat::from_rotation_x((6.0 + lean / 2.0 + flap).to_radians())
        * glam::Quat::from_rotation_z((lean2 / 2.0).to_radians())
        * glam::Quat::from_rotation_y((180.0 - lean2 / 2.0).to_radians());
    let now = glam::Quat::from_euler(glam::EulerRot::ZYX, pose[5], pose[4], pose[3]);
    let (z, y, x) = (now * by).to_euler(glam::EulerRot::ZYX);
    (pose[3], pose[4], pose[5]) = (x, y, z);
    // RenderTypes.entitySolid with OverlayTexture.NO_OVERLAY.
    draw(model, base, &poses, uv, 0xFFFF_FFFF, light, 0, out);
}
/// `Deadmau5EarsLayer.submit` with `PlayerEarsModel`: the player model's
/// head carrying two 6x6x1 ears (the model's other cubes are cleared),
/// drawn with the player's skin as `RenderTypes.entitySolid`. The caller has
/// checked `showExtraEars`; an invisible avatar draws no ears.
#[allow(clippy::too_many_arguments)]
pub fn ears(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    uv: [f32; 4],
    light: u32,
    data: &MobData,
    out: &mut Vec<Vertex>,
) {
    if m.is_invisible() {
        return;
    }
    let Some(model) = m.kind.layer("player#ears") else {
        return;
    };
    let s = anim::state(m, alpha);
    let mut poses = Poses::pooled();
    poses.reset(model);
    anim::setup(m, &s, &mut poses, &data.animations);
    // LivingEntityRenderer.getOverlayCoords(state, 0.0F).
    draw(
        model,
        base,
        &poses,
        uv,
        0xFFFF_FFFF,
        light,
        body_overlay(m, alpha),
        out,
    );
}
/// `PlayerModel.bodyParts`: what `getRandomBodyPart` picks from.
const BODY_PARTS: [&str; 6] = [
    "head",
    "body",
    "left_arm",
    "right_arm",
    "left_leg",
    "right_leg",
];

/// The cubes of a part as `ModelPart.cubes` has them: min and max corners in
/// model units. The part keeps six faces per cube, in cube order.
fn cube_bounds(part: &Part) -> Vec<[f32; 6]> {
    let size = if part.polygons.len().is_multiple_of(6) {
        6
    } else {
        part.polygons.len().max(1)
    };
    part.polygons
        .chunks(size)
        .map(|faces| {
            let mut b = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
            for v in faces.iter().flat_map(|f| f.vertices) {
                for k in 0..3 {
                    b[k] = b[k].min(v[k]);
                    b[3 + k] = b[3 + k].max(v[k]);
                }
            }
            b
        })
        .collect()
}

/// `StuckInBodyLayer.snapToFace`.
fn snap_to_face(value: f32) -> f32 {
    if value > 0.5 { 1.0 } else { 0.5 }
}

/// The most arrows or stingers drawn stuck in one body. Vanilla draws the
/// whole count, which only grows by one per hit and drains over time, so a
/// player rarely carries more than a few dozen; past that more copies look
/// alike, while a count a server or mod sets (up to 2^31) would cost a frame
/// that many models.
pub const MAX_STUCK_DRAWN: i32 = 128;

/// One copy `StuckInBodyLayer.submit` places: an index into `BODY_PARTS`, a
/// cube of that part and the point in it (`snapToFace`d on the surface).
#[derive(Clone, Copy, Debug, PartialEq)]
struct StuckPlacement {
    part: usize,
    cube: usize,
    mid: [f32; 3],
}

/// `StuckInBodyLayer.submit`'s random draws for the first `count` copies
/// (at most `MAX_STUCK_DRAWN`), from a random seeded by the entity id `id`:
/// a body part, one of its `cubes(part)` cubes, a point in it. A part with no
/// cube ends the copies.
fn stuck_placements(
    id: i32,
    count: i32,
    on_surface: bool,
    cubes: impl Fn(usize) -> usize,
) -> impl Iterator<Item = StuckPlacement> {
    use horizon_core::random::{LegacyRandom, RandomSource};
    let mut random = LegacyRandom::new(i64::from(id));
    std::iter::from_fn(move || {
        let part = random.next_int_bound(BODY_PARTS.len() as i32).unwrap_or(0) as usize;
        let cube = random.next_int_bound(cubes(part) as i32).ok()? as usize;
        let mut mid = [
            random.next_float(),
            random.next_float(),
            random.next_float(),
        ];
        if on_surface {
            let plane = random.next_int_bound(3).unwrap_or(0) as usize;
            if plane < 3 {
                mid[plane] = snap_to_face(mid[plane]);
            }
        }
        Some(StuckPlacement { part, cube, mid })
    })
    .take(count.clamp(0, MAX_STUCK_DRAWN) as usize)
}

/// `StuckInBodyLayer.submit` (with `submitStuckItem`) for a player: `count`
/// copies of `item` (`ArrowLayer`'s arrow model, `BeeStingerLayer`'s stinger)
/// stuck in random cubes of the posed body, placed from a random source
/// seeded by the entity id `id`. `on_surface` is `PlacementStyle.ON_SURFACE`.
#[allow(clippy::too_many_arguments)]
pub fn stuck_in_body(
    m: &Mob,
    id: i32,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    count: i32,
    on_surface: bool,
    item: &Part,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    if count <= 0 {
        return;
    }
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha);
    let (model_key, _) = body(m);
    let model = m
        .kind
        .layer(&model_key)
        .cloned()
        .unwrap_or_else(|| m.kind.model.clone());
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    // A part the model lacks has no cubes, which ends the copies as before.
    let cubes = BODY_PARTS.map(|name| find_part(&model, name).map(cube_bounds).unwrap_or_default());
    for stuck in stuck_placements(id, count, on_surface, |part| cubes[part].len()) {
        let name = BODY_PARTS[stuck.part];
        let cube = cubes[stuck.part][stuck.cube];
        let Some(mut t) = part_matrix(&model, base, &poses, name, 0.0) else {
            return;
        };
        let mid = stuck.mid;
        let lerp = |k: usize| cube[k] + mid[k] * (cube[3 + k] - cube[k]);
        t *= glam::Mat4::from_translation(glam::Vec3::new(lerp(0), lerp(1), lerp(2)) / 16.0);
        let (dx, dy, dz) = (
            -(mid[0] * 2.0 - 1.0),
            -(mid[1] * 2.0 - 1.0),
            -(mid[2] * 2.0 - 1.0),
        );
        let dxz = f64::from(dx * dx + dz * dz).sqrt() as f32;
        let degrees = |y: f32, x: f32| {
            (horizon_core::jmath::atan2(f64::from(y), f64::from(x))
                * f64::from(horizon_core::geometry::trig::RAD_TO_DEG)) as f32
        };
        let y_rot = degrees(dx, dz);
        let x_rot = degrees(dy, dxz);
        t *= glam::Mat4::from_rotation_y((y_rot - 90.0).to_radians())
            * glam::Mat4::from_rotation_z(x_rot.to_radians());
        super::render_model(item, t, &|p| p.pose, uv, light, 0, out);
    }
}
/// `ParrotOnShoulderLayer.submitOnShoulder`: a parrot of `variant` on the
/// avatar's left or right shoulder, in `ParrotModel.Pose.ON_SHOULDER` with
/// the avatar's walk animation, age and head turn. `base` is the avatar's
/// root transform; `parrot` the parrot's entity type (its model).
#[allow(clippy::too_many_arguments)]
pub fn shoulder_parrot(
    avatar: &Mob,
    parrot: &super::MobType,
    variant: i32,
    left: bool,
    alpha: f32,
    base: glam::Mat4,
    atlas: Atlas,
    light: u32,
    out: &mut Vec<Vertex>,
) {
    let Some(uv) = atlas(&shoulder_parrot_texture(variant)) else {
        return;
    };
    let model = parrot.layer(&parrot.model_key()).unwrap_or(&parrot.model);
    let s = anim::State {
        baby: false,
        ..anim::state(avatar, alpha)
    };
    let mut poses = Poses::new(model);
    // ParrotRenderState.flapAngle stays 0.
    super::body_anim::parrot(
        &mut poses,
        &s,
        super::body_anim::ParrotPose::OnShoulder,
        0.0,
    );
    let y = if avatar.extra.crouching { -1.3 } else { -1.5 };
    let x = if left { 0.4 } else { -0.4 };
    let base = base * glam::Mat4::from_translation(glam::Vec3::new(x, y, 0.0));
    // ParrotModel's cutout render type with OverlayTexture.NO_OVERLAY.
    draw(model, base, &poses, uv, 0xFFFF_FFFF, light, 0, out);
}
/// `ParrotRenderer.getVariantTexture` for `Parrot.Variant.byId` (an id out
/// of range is clamped).
fn shoulder_parrot_texture(variant: i32) -> String {
    const NAMES: [&str; 5] = ["red_blue", "blue", "green", "yellow_blue", "grey"];
    let name = usize::try_from(variant)
        .ok()
        .and_then(|i| NAMES.get(i))
        .unwrap_or(if variant < 0 { &NAMES[0] } else { &NAMES[4] });
    format!("entity/parrot/parrot_{name}")
}
/// `render_with` without the body, for an avatar the viewer can't see:
/// `LivingEntityRenderer.submit` gets no render type for an invisible body
/// (`isBodyVisible`, `isInvisibleTo`), but its layers still draw
/// (`HumanoidArmorLayer`: the armour in `extra`). `parts` as for [`render_with_hidden`], posed as
/// the layers draw over the unseen body.
#[allow(clippy::too_many_arguments)]
pub fn render_layers(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    worn: &WornArmor,
    extra: &[(&Part, [f32; 4], u32, u32, bool)],
    parts: Option<&mut Option<mod_api::PosedParts>>,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform(m, alpha, camera);
    render_visible(
        m,
        alpha,
        base,
        atlas,
        light,
        data,
        worn,
        extra,
        None,
        Default::default(),
        parts,
        out,
        None,
    );
}
/// A mob drawn under `origin` instead of at its position (the spinning mob
/// of `SpawnerRenderer.submitEntityInSpawner`).
pub fn render_posed(
    m: &Mob,
    alpha: f32,
    origin: glam::Mat4,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    worn: &WornArmor,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform_at(m, alpha, origin);
    render_visible(
        m,
        alpha,
        base,
        atlas,
        light,
        data,
        worn,
        &[],
        Some(body_color(m, alpha)),
        Default::default(),
        None,
        out,
        None,
    );
}
/// The custom model's key of a body model: `main` is the wide player model, `kind.model`.
fn cem_key(model_key: &str) -> &str {
    if model_key == "main" {
        "player#main"
    } else {
        model_key
    }
}
/// Every texture [`render_visible`] may look up for `m` now (keep the two in step: a texture
/// drawn but not listed here never gets a random pick): the body's and each layer's, a
/// custom model's own texture beside them (`cem_texture` falls back to the vanilla one when
/// the atlas lacks it, and only the layer a `.jem` was made for takes it, `paints_with_jem`),
/// and the warden's glowing layers (whether they glow this frame or not). Random textures
/// choose a variant for each.
pub fn drawn_textures(m: &Mob, data: &MobData, worn: &WornArmor) -> Vec<String> {
    fn push(names: &mut Vec<String>, custom: Option<&super::cem::CemLayer>, vanilla: String) {
        if let Some(texture) = custom.and_then(|c| c.texture_for(&vanilla)) {
            names.push(texture.to_owned());
        }
        names.push(vanilla);
    }
    let cem = data
        .cem
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (model_key, texture) = body(m);
    let cem_key = cem_key(&model_key);
    let mut names = Vec::new();
    let custom_body = (!cem.is_empty()).then(|| cem.layer(cem_key)).flatten();
    push(&mut names, custom_body, texture.into_owned());
    let mut all_layers = layers(m, m.age as f32, worn);
    if m.species() == Species::Warden {
        for (model, texture, _) in super::warden::emissive_layers(m, 0.0) {
            all_layers.push(Layer {
                model: model.into(),
                texture: texture.into(),
                color: 0,
                full_bright: true,
                scroll: 0.0,
                worn: None,
            });
        }
    }
    let body_keys = [&*model_key, cem_key];
    for (i, layer) in all_layers.iter().enumerate() {
        let custom = (!cem.is_empty())
            .then(|| cem.layer(&layer.model))
            .flatten()
            .filter(|_| paints_with_jem(&all_layers, i, body_keys));
        push(&mut names, custom, layer.texture.to_string());
    }
    names
}
/// The texture a model drawn as `custom` takes: the `.jem`'s when the atlas has it, else
/// `vanilla` (a texture the atlas lacks would hide the model).
fn cem_texture<'a>(
    custom: Option<&'a super::cem::CemLayer>,
    vanilla: &'a str,
    atlas: Atlas<'_>,
) -> &'a str {
    custom
        .and_then(|c| c.texture_for(vanilla))
        .filter(|t| atlas(t).is_some())
        .unwrap_or(vanilla)
}
/// Whether `layers[i]` is the layer its model's `.jem` was made for, so the `.jem`'s texture
/// paints it: the first drawn with that model (`sheep#wool`, a wolf's armour). Later layers
/// on the same model are overlays (the armour's cracks), like those on the body's model
/// (`body`: eyes, collars, a villager's type and profession), and keep their own textures
/// over the new geometry.
fn paints_with_jem(layers: &[Layer], i: usize, body: [&str; 2]) -> bool {
    let key = &*layers[i].model;
    !body.contains(&key) && layers[..i].iter().all(|l| l.model != key)
}
/// The model a mob's body draws with — the packs' custom one when it has one — posed the way
/// the body draws it (the vanilla animation, then the pack's own), with the `attachments`
/// anchors its bones declare. Everything that anchors to a mob — an item in its hand, a
/// block on its head — walks the same tree the player saw.
fn posed_body(
    m: &Mob,
    alpha: f32,
    data: &MobData,
) -> (
    std::sync::Arc<Part>,
    anim::PooledPoses,
    Vec<(String, String, [f32; 3])>,
) {
    let s = anim::state(m, alpha);
    let (model_key, _) = body(m);
    let cem = data
        .cem
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let custom = (!cem.is_empty())
        .then(|| cem.layer(cem_key(&model_key)))
        .flatten();
    let model = match custom {
        Some(c) => c.model.clone(),
        None => m
            .kind
            .layer(&model_key)
            .cloned()
            .unwrap_or_else(|| m.kind.model.clone()),
    };
    let mut poses = Poses::pooled();
    poses.reset(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    if let Some(c) = custom
        && let Some(animation) = &c.animation
    {
        animation.run(m, &s, &mut poses);
    }
    let anchors = custom.map_or(Vec::new(), |c| c.anchors.clone());
    (model, poses, anchors)
}
/// The humanoid parts of a body drawn with `poses`, for the mods' post hooks and layers: read from the poses the
/// draw already made, so nothing is posed twice; fixed-size, nothing allocated.
fn posed_parts(poses: &Poses) -> Option<mod_api::PosedParts> {
    mod_api::PosedParts::new(|part| {
        let name = part.model_name();
        Some((poses.pose_of(name)?, !poses.is_hidden_named(name)))
    })
}
/// The part of a player's cape layer (`PlayerCapeModel`'s `cape`): hidden by a mod's
/// `render_player_pre`, the player draws no cape this frame (`AvatarRenderState.showCape`).
pub const CAPE_PART: &str = "cape";
/// Whether `hidden` leaves out `m`'s cape: [`CAPE_PART`], on a model that has a cape layer.
pub fn hides_cape(m: &Mob, hidden: &mod_api::HiddenParts) -> bool {
    m.kind.layer("player#cape").is_some() && hidden.contains_named(CAPE_PART)
}
// Validate names against this frame's model before mutating its body poses; the cape is a
// part of the cape layer's model (`hides_cape`), which the body's poses don't hold.
// Return the borrowed name so a repeated refusal never allocates an error string.
fn hide_parts<'a>(
    poses: &mut Poses,
    hidden: &'a mod_api::HiddenParts,
    has_cape: bool,
) -> Result<(), &'a str> {
    for name in hidden.names() {
        if poses.pose_of(name).is_none() && !(has_cape && name == CAPE_PART) {
            return Err(name);
        }
    }
    for name in hidden.names() {
        poses.visible(name, false);
    }
    Ok(())
}

#[derive(Default)]
struct HiddenPartReports {
    names: std::collections::HashMap<String, std::collections::HashSet<String>>,
    count: usize,
}

impl HiddenPartReports {
    // Mods can change their eight hidden names between frames. Never evict an entry (which
    // would report it again); after this process-wide budget, suppress new diagnostics too.
    const LIMIT: usize = 4096;

    fn first(&mut self, kind: &str, name: &str) -> bool {
        if self.count >= Self::LIMIT
            || self
                .names
                .get(kind)
                .is_some_and(|names| names.contains(name))
        {
            return false;
        }
        self.names
            .entry(kind.to_owned())
            .or_default()
            .insert(name.to_owned());
        self.count += 1;
        true
    }
}

fn report_unknown_hidden_part(kind: &str, name: &str) {
    // Shared by every renderer, including mirror views and different render threads.
    static REPORTS: std::sync::Mutex<Option<HiddenPartReports>> = std::sync::Mutex::new(None);
    let mut reports = REPORTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if reports
        .get_or_insert_with(HiddenPartReports::default)
        .first(kind, name)
    {
        eprintln!("mob {kind}: unknown hidden part {name}");
    }
}

#[allow(clippy::too_many_arguments)]
fn render_visible(
    m: &Mob,
    alpha: f32,
    base: glam::Mat4,
    atlas: Atlas,
    light: u32,
    data: &MobData,
    worn: &WornArmor,
    extra: &[(&Part, [f32; 4], u32, u32, bool)],
    // `submit`'s `tintedColor` to draw the body in, or `None` for its layers only.
    paint: Option<u32>,
    hidden: mod_api::HiddenParts,
    parts: Option<&mut Option<mod_api::PosedParts>>,
    out: &mut Vec<Vertex>,
    skin: Option<&mut super::skin::Skin>,
) {
    // The armour is a layer: none for a spectator.
    let extra = if layers_shown(m) { extra } else { &[] };
    let overlay = u32::from(red_overlay(m));
    let s = anim::state(m, alpha);
    let (model_key, texture) = body(m);
    // The packs' custom entity models replace a layer's parts and maybe its texture. The read
    // guard, not a clone of the Arc: one uncontended lock per mob, nothing else without a pack.
    let cem = data
        .cem
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cem_key = cem_key(&model_key);
    let custom_body = (!cem.is_empty()).then(|| cem.layer(cem_key)).flatten();
    let body_animation = custom_body.and_then(|c| c.animation.as_ref());
    let model = match custom_body {
        Some(custom) => custom.model.clone(),
        // 3D skin layers: the player's own outer layer, one slab per painted
        // texel (`mobs::skin_layers`), in place of the flat inflated box.
        None => m
            .skin_3d
            .clone()
            .or_else(|| m.kind.layer(&model_key).cloned())
            .unwrap_or_else(|| m.kind.model.clone()),
    };
    let texture = cem_texture(custom_body, &texture, atlas);
    // OptiFine random textures and ETF blinking: the sprite a texture draws with instead, when
    // the atlas has it: the variant the mob wears, then its blink frame while it blinks (or
    // sleeps). The texture keeps its name for everything else.
    let random = data
        .random_textures
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let blinks = data
        .blinks
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sprite = |texture: &str| {
        let variant = (!random.is_empty())
            .then(|| super::random_textures::texture(m, &random, texture))
            .flatten()
            .filter(|v| atlas(v).is_some());
        let frame = (!blinks.is_empty())
            .then(|| blinks.get(variant.unwrap_or(texture)))
            .flatten()
            .and_then(|b| {
                if super::villager::sleeping(m) {
                    Some(b.shut.as_str())
                } else {
                    b.frame(s.age as u32, m.id.wrapping_mul(0x9E37_79B1))
                }
            })
            .filter(|f| atlas(f).is_some());
        let name = frame.or(variant)?;
        Some((name, atlas(name)?))
    };
    // OptiFine emissive overlays: the drawn sprite's `_e`, at full brightness with the model's
    // shading, as OptiFine draws them.
    let emissive = data
        .emissive
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let emissive_uv = |drawn: &str| {
        if emissive.is_empty() {
            return None;
        }
        atlas(emissive.overlay(drawn)?)
    };
    let mut poses = Poses::pooled();
    poses.reset(&model);
    anim::setup(m, &s, &mut poses, &data.animations);
    if let Some(animation) = body_animation {
        animation.run(m, &s, &mut poses);
    }
    let mut all_layers = layers(m, s.age, worn);
    // WardenRenderer: the translucent emissive layers fade with the animations.
    if m.species() == Species::Warden {
        for (model, texture, a) in super::warden::emissive_layers(m, alpha) {
            if a > 0.0 {
                all_layers.push(Layer {
                    model: model.into(),
                    texture: texture.into(),
                    color: ((a.clamp(0.0, 1.0) * 255.0) as u32) << 24 | 0x00FF_FFFF,
                    full_bright: true,
                    scroll: 0.0,
                    worn: None,
                });
            }
        }
    }
    // The GPU path takes the whole mob or none of it, so its draws keep their
    // order: only plain draws of models it can pose (no custom models,
    // translucent body or energy swirl, which the CPU path changes after
    // drawing), and with improved transparency off the tagged layers too.
    let mut skin = skin;
    let posable = match skin.as_deref_mut() {
        Some(skin) => {
            custom_body.is_none()
                && extra.is_empty()
                && m.species() != Species::SulfurCube
                && (!translucent_body(m)
                    || (skin.models.tagged && paint.is_none_or(|t| t >> 24 == 0xFF)))
                && energy_swirl(m, s.age).is_none()
                && skin.supports(&model)
                && all_layers.iter().all(|layer| {
                    // With improved transparency off, a translucent layer's tag
                    // is the only difference from a plain draw.
                    (skin.models.tagged && (eyes_layer(m, layer) || !layer.full_bright)
                        || !layer.full_bright && !translucent_layer(m, layer))
                        && (cem.is_empty() || cem.layer(&layer.model).is_none())
                        && m.kind
                            .layer(&layer.model)
                            .is_none_or(|part| skin.supports(part))
                })
        }
        None => false,
    };
    if !posable {
        skin = None;
    }
    // The parts a mod's `render_player_pre` or `render_living_pre` hid this frame: the body and the
    // layers over its own model (`getParentModel`) draw without them, the armour and the other
    // layers below with every part.
    let mut hidden_poses = Option::<anim::PooledPoses>::None;
    let body_poses: &Poses = if hidden.is_empty() {
        &poses
    } else {
        let hp = hidden_poses.get_or_insert_with(Poses::pooled);
        hp.copy_from(&poses);
        let has_cape = m.kind.layer("player#cape").is_some();
        if let Err(name) = hide_parts(hp, &hidden, has_cape) {
            report_unknown_hidden_part(&m.kind.id, name);
            return;
        }
        &*hp
    };
    // The mods' post hooks' snapshot: the body's parts as drawn, for `HumanoidMobRenderer`'s families
    // (`anim::extracts_humanoid`) only.
    if let Some(parts) = parts
        && anim::extracts_humanoid(m)
    {
        *parts = posed_parts(body_poses);
    }
    let draw_body = |out: &mut Vec<Vertex>, skin: &mut Option<&mut super::skin::Skin>| {
        // A player or mannequin whose skin has no atlas sprite (an evicted or
        // never stored download) draws with its default skin, not invisibly.
        let drawn = sprite(texture).or_else(|| Some((texture, atlas(texture)?)));
        let uv = drawn.map(|(_, uv)| uv).or_else(|| {
            if avatar(m) {
                atlas(&m.kind.texture)
            } else {
                None
            }
        });
        if let (Some(tint), Some(uv)) = (paint, uv) {
            let start = out.len();
            let first_job = skin.as_deref().map(|skin| skin.frame.jobs.len());
            put(
                skin,
                &model,
                base,
                body_poses,
                uv,
                0.0,
                tint,
                light,
                body_overlay(m, alpha),
                out,
            );
            if let Some(glow) = drawn.and_then(|(name, _)| emissive_uv(name)) {
                put(
                    skin,
                    &model,
                    base,
                    body_poses,
                    glow,
                    0.0,
                    tint,
                    FULL_BRIGHT,
                    0,
                    out,
                );
            }
            // A body below full alpha (`forceTransparent`'s 654311423) blends too.
            if translucent_body(m) || tint >> 24 != 0xFF {
                mark_oit(&mut out[start..]);
                if let (Some(skin), Some(first)) = (skin.as_deref_mut(), first_job) {
                    for job in &mut skin.frame.jobs[first..] {
                        job.overlay |= crate::gfx::OIT_ENTITY;
                    }
                }
            }
        }
    };
    // The translucent shell writes depth: draw the opaque core before it.
    let shell_last = m.species() == Species::SulfurCube;
    if !shell_last {
        draw_body(out, &mut skin);
    }
    // The worn armour's layers (and their emissive overlays, full bright).
    for (part, uv, color, layer_overlay, glows) in extra {
        let light = if *glows { FULL_BRIGHT } else { light };
        draw(part, base, &poses, *uv, *color, light, *layer_overlay, out);
    }
    // One Poses refilled for every layer (armour draws up to seven): it keeps
    // its maps' capacity. Only borrow from the pool when this mob actually has layers or swirl.
    let mut layer_poses = Option::<anim::PooledPoses>::None;
    let body_keys = [&*model_key, cem_key];
    let shown = shown_equipment();
    for (i, layer) in all_layers.iter().enumerate() {
        let custom = (!cem.is_empty()).then(|| cem.layer(&layer.model)).flatten();
        // `layer.texture` stays the vanilla name (the villager hats below look for its
        // `/type/` folder).
        let texture = cem_texture(
            custom.filter(|_| paints_with_jem(&all_layers, i, body_keys)),
            &layer.texture,
            atlas,
        );
        let part = match custom {
            Some(custom) => custom.model.clone(),
            None => match m.kind.layer(&layer.model) {
                Some(part) => part.clone(),
                None => continue,
            },
        };
        let Some((drawn, uv)) = sprite(texture).or_else(|| Some((texture, atlas(texture)?))) else {
            continue;
        };
        // VillagerProfessionLayer: the type layer under a profession's full hat draws on
        // `noHatModel`, a model of its own, the others on the parent model.
        let type_hat_shown = if matches!(m.species(), Species::Villager | Species::ZombieVillager)
            && let Some((folder, _variant)) = layer
                .texture
                .split_once("/type/")
                .or_else(|| layer.texture.split_once("/baby/"))
        {
            villager_client::type_hat_shown(
                &m.extra.profession,
                folder == "entity/zombie_villager",
                m.extra.variant,
            )
        } else {
            true
        };
        let reuse_body =
            custom.is_none() && custom_body.is_none() && std::sync::Arc::ptr_eq(&part, &model);
        let active_poses: &Poses = if reuse_body && type_hat_shown {
            // A layer over the body's own model with its hat visible (spider eyes,
            // villager profession/level, collars): reuse body_poses directly with zero copy.
            body_poses
        } else {
            let lp = layer_poses.get_or_insert_with(Poses::pooled);
            if reuse_body {
                lp.copy_from(&poses);
            } else {
                lp.reset(&part);
                anim::setup(m, &s, lp, &data.animations);
                if let Some(animation) = custom.and_then(|c| c.animation.as_ref()) {
                    animation.run(m, &s, lp);
                }
            }
            if !type_hat_shown {
                lp.visible("head", false);
            }
            &*lp
        };
        let light = if layer.full_bright {
            FULL_BRIGHT
        } else {
            light
        };
        // EquipmentLayerRenderer draws armour with OverlayTexture.NO_OVERLAY: never
        // the hurt tint, the armour glint (`ARMOR_GLINT`) when enchanted.
        let overlay = match armor_slot(&layer.model) {
            Some(slot) if super::gear::enchanted(m, slot) => ARMOR_GLINT,
            Some(_) => 0,
            None => overlay,
        };
        // What it wears, as a mod's `render_worn_equipment` lets it be drawn.
        let (color, overlay) = match layer.worn {
            Some(slot) => {
                let base_part = mod_api::EquipmentRenderPart::Base;
                match shown_draw(&shown, slot, base_part, layer.color, overlay) {
                    Some(color_and_overlay) => color_and_overlay,
                    None => continue,
                }
            }
            None => (layer.color, overlay),
        };
        let start = out.len();
        let first_job = skin.as_deref().map(|skin| skin.frame.jobs.len());
        put(
            &mut skin,
            &part,
            base,
            active_poses,
            uv,
            layer.scroll,
            color,
            light,
            overlay,
            out,
        );
        if let Some(glow) = emissive_uv(drawn) {
            put(
                &mut skin,
                &part,
                base,
                active_poses,
                glow,
                layer.scroll,
                color,
                FULL_BRIGHT,
                0,
                out,
            );
        }
        if translucent_layer(m, layer) {
            mark_oit(&mut out[start..]);
        }
        // The same marks on what the GPU path posed of the layer (only eyes
        // layers get there among the marked ones): the tags in the overlay word,
        // no normal.
        if let (Some(skin), Some(first)) = (skin.as_deref_mut(), first_job) {
            let tagged = translucent_layer(m, layer);
            let eyes = layer.full_bright && m.species() != Species::Warden;
            for job in &mut skin.frame.jobs[first..] {
                if tagged {
                    job.overlay |= crate::gfx::OIT_ENTITY;
                }
                if eyes {
                    job.overlay |= u32::from(EYES_TIER) << 16;
                    job.mode = crate::gfx::entity_skin::SkinJob::FLAT;
                }
            }
        }
        // The eyes render type's glowing eyes also shine in HDR output, at
        // `EYES_TIER` in the overlay's high bits (entity.wgsl hdr_entity_mark).
        if layer.full_bright && m.species() != Species::Warden {
            for vertex in &mut out[start..] {
                vertex.normal = [0; 4];
                vertex.layer |= u32::from(EYES_TIER) << 16;
            }
        }
        // The warden's heart and spots shine too, by how far their layers have faded in.
        if layer.full_bright && m.species() == Species::Warden {
            for vertex in &mut out[start..] {
                vertex.layer |= u32::from(EYES_TIER) << 16;
            }
        }
    }
    // EnergySwirlLayer: the armour model, additive, scrolling over the sprite.
    if let Some((model, texture, x_offset)) = energy_swirl(m, s.age)
        && let Some(part) = m.kind.layer(model)
        && let Some([u0, v0, u1, v1]) = atlas(texture)
    {
        let lp = layer_poses.get_or_insert_with(Poses::pooled);
        lp.reset(part);
        anim::setup(m, &s, lp, &data.animations);
        // `RenderTypes.energySwirl`'s texture offset, as the sprite-local UV.
        let (du, dv) = (x_offset % 1.0, s.age * 0.01 % 1.0);
        let start = out.len();
        draw(
            part,
            base,
            lp,
            [du, dv, 1.0 + du, 1.0 + dv],
            crate::breaking::stage_light([u1 - u0, v1 - v0]),
            crate::breaking::stage_light([u0, v0]),
            crate::gfx::ENERGY_SWIRL_OVERLAY,
            out,
        );
        // No lighting: the colour bytes carry the sprite's size (fs_energy_swirl).
        for vertex in &mut out[start..] {
            vertex.normal = [0; 4];
        }
    }
    if shell_last {
        draw_body(out, &mut skin);
    }
}
/// `EnergySwirlLayer`'s model, texture and `xOffset(t)` for a charged creeper
/// (`CreeperPowerLayer`) or a powered wither (`WitherArmorLayer`); `t` is `ageInTicks`.
fn energy_swirl(m: &Mob, t: f32) -> Option<(&'static str, &'static str, f32)> {
    match m.species() {
        Species::Creeper if m.extra.charged => {
            Some(("creeper#armor", "entity/creeper/creeper_armor", t * 0.01))
        }
        Species::Wither if super::wither::powered(m) => Some((
            "wither#armor",
            "entity/wither/wither_armor",
            (t * 0.02).mcos() * 3.0,
        )),
        _ => None,
    }
}
/// The equipment slot of a `HumanoidArmorLayer` model (`zombie#helmet`).
fn armor_slot(model: &str) -> Option<usize> {
    Some(match model.rsplit('#').next()? {
        "helmet" => super::gear::HEAD,
        "chestplate" => super::gear::CHEST,
        "leggings" => super::gear::LEGS,
        "boots" => super::gear::FEET,
        _ => return None,
    })
}
/// Render with a fixed texture rect (the earlier API).
pub fn render_single(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    uv: [f32; 4],
    light: u32,
    out: &mut Vec<Vertex>,
) {
    let base = base_transform(m, alpha, camera);
    let overlay = body_overlay(m, alpha);
    let s = anim::state(m, alpha);
    let model = m.kind.model.clone();
    let mut poses = Poses::new(&model);
    anim::setup(m, &s, &mut poses, &std::collections::HashMap::new());
    draw(&model, base, &poses, uv, 0xFFFF_FFFF, light, overlay, out);
}

/// The transform of a named part (ModelPart.translateAndRotate down the tree).
pub fn part_matrix(
    part: &Part,
    parent: glam::Mat4,
    poses: &Poses,
    name: &str,
    x_offset: f32,
) -> Option<glam::Mat4> {
    let mut p = poses.pose(part);
    if part.name == name {
        p[0] += x_offset;
    }
    let m = parent
        * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
    if part.name == name {
        return Some(m);
    }
    part.children
        .iter()
        .find_map(|c| part_matrix(c, m, poses, name, x_offset))
}
/// The first part named `name`, depth first.
pub(super) fn find_part<'a>(part: &'a Part, name: &str) -> Option<&'a Part> {
    if part.name == name {
        return Some(part);
    }
    part.children.iter().find_map(|c| find_part(c, name))
}
/// The matrix up to a part that declares an item anchor (`attachments` in the pack's file,
/// EMF's `EMFAttachment`): the walk the positioner takes — every part on the way applies its
/// pose, like `part_matrix` — then the anchor's own offset, in model pixels.
fn attachment_matrix(
    part: &Part,
    parent: glam::Mat4,
    poses: &Poses,
    anchors: &[(String, String, [f32; 3])],
    key: &str,
) -> Option<glam::Mat4> {
    if anchors.is_empty() {
        return None;
    }
    let p = poses.pose(part);
    let m = parent
        * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
        * glam::Mat4::from_quat(glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            p[5],
            p[4],
            p[3],
        ))
        * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
    if let Some((_, _, [x, y, z])) = anchors
        .iter()
        .find(|(bone, kind, _)| *kind == key && part.name == *bone)
    {
        return Some(m * glam::Mat4::from_translation(glam::Vec3::new(*x, *y, *z) / 16.0));
    }
    part.children
        .iter()
        .find_map(|c| attachment_matrix(c, m, poses, anchors, key))
}
/// `AllayModel.translateToHand`: the root and body, then the right arm's
/// pitch, a 0.7 scale and a shift. Both hands use the right arm's pitch.
fn allay_hand(model: &Part, base: glam::Mat4, poses: &Poses) -> Option<glam::Mat4> {
    let body = part_matrix(model, base, poses, "body", 0.0)?;
    let x_rot = poses.pose(find_part(model, "right_arm")?)[3];
    Some(
        body * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0625, 0.1875))
            * glam::Mat4::from_rotation_x(x_rot)
            * glam::Mat4::from_scale(glam::Vec3::splat(0.7))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0625, 0.0, 0.0)),
    )
}
/// The hand of a mob in model space, up to the item's own transform
/// (`ItemInHandLayer.submitArmWithItem`).
fn hand_base(
    m: &Mob,
    model: &Part,
    base: glam::Mat4,
    poses: &Poses,
    anchors: &[(String, String, [f32; 3])],
    left: bool,
) -> Option<glam::Mat4> {
    // A pack's `attachments` anchor (`right_handheld_item`...) replaces the whole hand
    // anchor: the walk to the part that declares it and its offset, none of the vanilla
    // hand's own transforms — EMF's one-off override during `translateToHand`. Without one
    // the vanilla anchor stays.
    if let Some(t) = attachment_matrix(
        model,
        base,
        poses,
        anchors,
        if left {
            "left_handheld_item"
        } else {
            "right_handheld_item"
        },
    ) {
        return Some(t);
    }
    let skeleton = matches!(
        m.species(),
        Species::Skeleton
            | Species::Stray
            | Species::Bogged
            | Species::Parched
            | Species::WitherSkeleton
    );
    let arm = if left { "left_arm" } else { "right_arm" };
    // `SkeletonModel.translateToHand` shifts the arm by 1, `PlayerModel.translateToHand`
    // by 0.5 for slim arms: the hand is the middle of the arm.
    let offset = if skeleton {
        if left { -1.0 } else { 1.0 }
    } else if avatar(m) && m.slim {
        if left { -0.5 } else { 0.5 }
    } else {
        0.0
    };
    let mut t = if m.species() == Species::Allay {
        allay_hand(model, base, poses)?
    } else {
        part_matrix(model, base, poses, arm, offset)?
    };
    if m.species() == Species::Vex {
        // `VexModel.translateToHand`: the held item is drawn at 0.55 scale.
        t *= glam::Mat4::from_scale(glam::Vec3::splat(0.55));
        t *= glam::Mat4::from_translation(glam::Vec3::new(
            if left { -0.046_875 } else { 0.046_875 },
            -0.156_25,
            0.078_125,
        ));
    }
    if m.species() == Species::CopperGolem {
        // `CopperGolemModel.translateToHand`: a still golem holds the item
        // across the arm, a working one scaled down in front of it.
        if m.extra.variant3 == super::copper_golem::IDLE {
            t *= glam::Mat4::from_rotation_y((if left { 90.0f32 } else { -90.0 }).to_radians())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.125));
        } else {
            t *= glam::Mat4::from_scale(glam::Vec3::splat(0.55))
                * glam::Mat4::from_translation(glam::Vec3::new(-0.125, 0.3125, -0.1875));
        }
    }
    let (ox, oy, oz) = if m.is_baby() {
        (0.0, 1.0, -4.5)
    } else {
        (1.0, 2.0, -10.0)
    };
    t *= glam::Mat4::from_rotation_x((-90.0f32).to_radians()) * glam::Mat4::from_rotation_y(PI);
    t *= glam::Mat4::from_translation(glam::Vec3::new(
        if left { -ox } else { ox } / 16.0,
        oy / 16.0,
        oz / 16.0,
    ));
    Some(t)
}
/// The transform of one of a mob's model parts, in camera space
/// (`ModelPart.translateAndRotate` down the tree).
pub fn part_transform(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    part: &str,
) -> Option<glam::Mat4> {
    let base = base_transform(m, alpha, camera);
    let (model, poses, _) = posed_body(m, alpha, data);
    part_matrix(&model, base, &poses, part, 0.0)
}
/// `PlayerItemInHandLayer.renderItemHeldToEye` up to the item: the root and
/// the head with its pitch clamped to -30 to 90 degrees, `translateToHead`
/// (`CustomHeadLayer.translateToHead`), then the hand's side of the eye.
pub fn held_to_eye_matrix(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    left: bool,
) -> Option<glam::Mat4> {
    use glam::{Mat4, Vec3};
    let base = base_transform(m, alpha, camera);
    let (model, mut poses, _) = posed_body(m, alpha, data);
    let pitch =
        poses.get("head")[3].clamp(-std::f32::consts::FRAC_PI_6, std::f32::consts::FRAC_PI_2);
    poses.xrot("head", pitch);
    let head = part_matrix(&model, base, &poses, "head", 0.0)?;
    let side = if left { -2.5 / 16.0 } else { 2.5 / 16.0 };
    Some(
        head * Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
            * Mat4::from_rotation_y(std::f32::consts::PI)
            * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
            * Mat4::from_translation(Vec3::new(side, -0.0625, 0.0)),
    )
}
/// `FoxHeldItemLayer`: the item sits at the head's position under the render
/// state's head roll, yaw and pitch (not the posed head's, which differ while
/// it sleeps), pushed out to the mouth.
fn fox_item_transform(m: &Mob, alpha: f32, camera: glam::DVec3, data: &MobData) -> glam::Mat4 {
    use glam::{Mat4, Vec3};
    let s = anim::state(m, alpha);
    let (model, mut poses, anchors) = posed_body(m, alpha, data);
    let head = *poses.get("head");
    let fox = super::fox::pose(m, alpha);
    let baby = m.is_baby();
    let mut t = base_transform(m, alpha, camera)
        * Mat4::from_translation(Vec3::new(head[0], head[1], head[2]) / 16.0);
    if baby {
        t *= Mat4::from_scale(Vec3::splat(0.75));
    }
    t *= Mat4::from_rotation_z(fox.head_roll_angle)
        * Mat4::from_rotation_y(s.y_rot.to_radians())
        * Mat4::from_rotation_x(s.x_rot.to_radians());
    // A pack's `fox_item` anchor (Fresh Animations' fox declares one under its rebuilt
    // head) replaces the mouth offset: the walk to the part that declares it and its
    // offset, as EMF's fox offset injects the positioner in place of the layer's first
    // translate. The layer's own rotations (the item's upright) keep applying after
    // either way.
    if let Some(anchor) = attachment_matrix(&model, t, &poses, &anchors, "fox_item") {
        t = anchor;
    } else {
        let (x, y, z) = match (baby, fox.sleeping) {
            (true, true) => (0.4, 0.26, 0.15),
            (true, false) => (0.06, 0.26, -0.5),
            (false, true) => (0.46, 0.26, 0.22),
            (false, false) => (0.06, 0.27, -0.5),
        };
        t *= Mat4::from_translation(Vec3::new(x, y, z));
    }
    t *= Mat4::from_rotation_x(90.0f32.to_radians());
    if fox.sleeping {
        t *= Mat4::from_rotation_z(90.0f32.to_radians());
    }
    t
}
/// `CrossedArmsItemLayer.applyTranslation`: `translateToArms`, then the item's
/// placement over the folded arms.
fn crossed_arms_matrix(model: &Part, base: glam::Mat4, poses: &Poses) -> Option<glam::Mat4> {
    let arms = part_matrix(model, base, poses, "arms", 0.0)?;
    Some(
        arms * glam::Mat4::from_rotation_x(0.75)
            * glam::Mat4::from_scale(glam::Vec3::splat(1.07))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.13, -0.34))
            * glam::Mat4::from_rotation_x(std::f32::consts::PI),
    )
}
/// The pose a mob's held item is submitted under (`ItemInHandLayer`:
/// `translateToHand`, then the hand offset; the item model adds its own
/// display transform), or `None` when the mob shows no held items.
pub fn held_item_transform(
    m: &Mob,
    alpha: f32,
    camera: glam::DVec3,
    data: &MobData,
    left: bool,
) -> Option<glam::Mat4> {
    // PandaHoldsItemLayer: the main hand's item in its mouth.
    if m.species() == Species::Panda {
        return super::panda::item_offset(m, alpha)
            .filter(|_| !left)
            .map(|offset| base_transform(m, alpha, camera) * offset);
    }
    // FoxHeldItemLayer: the main hand's item in its mouth.
    if m.species() == Species::Fox {
        return (!left).then(|| fox_item_transform(m, alpha, camera, data));
    }
    // DolphinCarryingItemLayer: the main hand's item at the snout, lifted and
    // pulled in as the dolphin pitches (up or down) toward 60 degrees.
    if m.species() == Species::Dolphin {
        return (!left).then(|| {
            let pitch = anim::state(m, alpha).x_rot;
            let percent = pitch.abs() / 60.0;
            let offset = if pitch < 0.0 {
                glam::Vec3::new(0.0, 1.0 - percent * 0.5, -1.0 + percent * 0.5)
            } else {
                glam::Vec3::new(0.0, 1.0 + percent * 0.8, -1.0 + percent * 0.2)
            };
            base_transform(m, alpha, camera) * glam::Mat4::from_translation(offset)
        });
    }
    if !super::illager::shows_item(m) {
        return None;
    }
    let base = base_transform(m, alpha, camera);
    let s = anim::state(m, alpha);
    let (model, poses, anchors) = posed_body(m, alpha, data);
    // WitchItemLayer.applyTranslation: a potion is held at the nose, any other
    // item as CrossedArmsItemLayer holds it; only the main hand's item is drawn.
    if m.species() == Species::Witch {
        let item = m.extra.main_hand_id()?;
        if left != m.main_arm_left() {
            return None;
        }
        if super::witch::holds_potion(item) {
            let nose = part_matrix(&model, base, &poses, "nose", 0.0)?;
            return Some(
                nose * glam::Mat4::from_translation(glam::Vec3::new(0.0625, 0.25, 0.0))
                    * glam::Mat4::from_rotation_z(180.0f32.to_radians())
                    * glam::Mat4::from_rotation_x(140.0f32.to_radians())
                    * glam::Mat4::from_rotation_z(10.0f32.to_radians())
                    * glam::Mat4::from_rotation_x(180.0f32.to_radians()),
            );
        }
        return crossed_arms_matrix(&model, base, &poses);
    }
    // CrossedArmsItemLayer (villagers and wandering traders): the main hand's
    // item over the folded arms.
    if matches!(m.species(), Species::Villager | Species::WanderingTrader) {
        m.extra.main_hand_id()?;
        if left != m.main_arm_left() {
            return None;
        }
        return crossed_arms_matrix(&model, base, &poses);
    }
    let mut t = hand_base(m, &model, base, &poses, &anchors, left)?;
    // ItemInHandLayer: a STAB swing of this arm thrusts its item out
    // (SpearAnimations.thirdPersonAttackItem, the main hand's forward
    // movement), and a spear in use moves with it (ArmPose.SPEAR's
    // animateUseItem).
    let swing_left = m.extra.swing_right_arm.map_or(m.main_arm_left(), |r| !r);
    if m.extra.swing_stab && swing_left == left {
        let forward = m
            .extra
            .main_hand_id()
            .and_then(crate::spear::Kinetic::of)
            .map_or(0.0, |k| k.forward_movement);
        t *= crate::spear::third_person_attack_item(s.swing, forward);
    }
    let off_hand = left != m.main_arm_left();
    if super::spear_use::is_using_item(m)
        && m.extra.using_off_hand == off_hand
        && let Some(kinetic) = (if off_hand {
            m.extra.off_hand_id()
        } else {
            m.extra.main_hand_id()
        })
        .and_then(crate::spear::Kinetic::of)
    {
        let time_held = m.extra.using_item as f32 + s.partial;
        t *= crate::spear::third_person_use_item(&kinetic, time_held, s.swing, 0.0, !left);
    }
    Some(t)
}

/// `ArmedEntityRenderState.extractArmedEntityRenderState`: each arm's item
/// model reads the stack held by that arm: each hand has its own slot, so
/// the off hand never inherits the main hand's item or components.
pub fn held_item_stack(extra: &super::Extra, off_hand: bool) -> Option<crate::items::ItemStack> {
    use horizon_items::equipment::{MAINHAND, OFFHAND};
    extra
        .slot_stack(if off_hand { OFFHAND } else { MAINHAND })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_hidden_parts_report_once_per_kind_and_name() {
        let mut reports = HiddenPartReports::default();
        assert!(reports.first("minecraft:cow", "nose"));
        for _ in 0..10 {
            assert!(!reports.first("minecraft:cow", "nose"));
        }
        assert!(reports.first("minecraft:pig", "nose"));
        assert!(reports.first("minecraft:cow", "hat_rim"));
        assert!(!reports.first("minecraft:pig", "nose"));
        assert!(!reports.first("minecraft:cow", "hat_rim"));
        assert_eq!(reports.count, 3);

        for i in 3..HiddenPartReports::LIMIT {
            assert!(reports.first("minecraft:cow", &format!("part_{i}")));
        }
        assert!(!reports.first("minecraft:cow", "over_budget"));
        assert!(!reports.first("minecraft:sheep", "nose"));
        assert!(!reports.first("minecraft:cow", "nose"));
        assert_eq!(reports.count, HiddenPartReports::LIMIT);
        assert_eq!(reports.names.len(), 2);
    }

    #[test]
    fn profession_texture_preserves_nested_namespaces() {
        assert_eq!(
            villager_client::profession_texture("entity/villager", "minecraft:farmer"),
            "entity/villager/profession/farmer"
        );
        assert_eq!(
            villager_client::profession_texture(
                "entity/zombie_villager",
                "example:nested/engineer"
            ),
            "example:entity/zombie_villager/profession/nested/engineer"
        );
    }

    /// A zombie whose model is a humanoid built here (head with its hat, body, arms, legs: one
    /// face each), and empty mob data: what the posed-parts tests draw, without `.local`.
    fn humanoid_zombie() -> (Mob, MobData) {
        let cube = |x: f32| serde_json::json!({ "pose": [x, 0.0, 0.0], "polygons": [[[x, 0.0, 0.0, 0.0, 0.0], [x + 1.0, 0.0, 0.0, 1.0, 0.0], [x + 1.0, 1.0, 0.0, 1.0, 1.0], [x, 1.0, 0.0, 0.0, 1.0]]] });
        let mut head = cube(0.0);
        head["children"] = serde_json::json!({ "hat": cube(0.0) });
        let model = serde_json::json!({ "children": {
            "head": head, "body": cube(0.0), "right_arm": cube(-5.0), "left_arm": cube(5.0),
            "right_leg": cube(-2.0), "left_leg": cube(2.0),
        }});
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = Species::Zombie;
        kind.anim = super::super::Anim::Zombie;
        kind.model = std::sync::Arc::new(Part::parse("root", &model));
        let mut mob = Mob::new(
            1,
            std::sync::Arc::new(kind),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        );
        mob.age = 37;
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            resolved: Default::default(),
            mod_spawns: Default::default(),
        };
        (mob, data)
    }

    // The mods' post hooks' posed parts are kept from the body's own draw, on every entry the game draws
    // a mob or player through (CPU, GPU, layers only), and keeping them changes nothing drawn
    // (arismoko/horizon#2060, arismoko/horizon#2464).
    include!("render_pose_tests.rs");

    /// A part `render_player_pre` hid leaves the body, after Skin Customization, while the armour
    /// layered on it keeps the part (arismoko/horizon#2060, part 2).
    #[test]
    fn a_hidden_part_leaves_the_body_but_not_its_armour() {
        let (mob, data) = humanoid_zombie();
        let atlas = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
        let armour = mob.kind.model.clone();
        let extra = [(&*armour, [0.0, 0.0, 1.0, 1.0], 0xFFFF_FFFF, 0, false)];
        let hide = |parts: &[mod_api::HumanoidPart]| {
            let mut hidden = mod_api::HiddenParts::default();
            parts.iter().for_each(|part| hidden.hide(*part));
            hidden
        };
        let draw_parts = |hidden: mod_api::HiddenParts,
                          extra: &[(&Part, [f32; 4], u32, u32, bool)]| {
            let (mut out, mut parts) = (Vec::new(), None);
            render_with_hidden(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &Default::default(),
                extra,
                hidden,
                Some(&mut parts),
                &mut out,
            );
            (out.len(), parts.expect("a humanoid"))
        };
        let draw =
            |hidden, extra: &[(&Part, [f32; 4], u32, u32, bool)]| draw_parts(hidden, extra).0;
        let whole = draw(hide(&[]), &[]);
        let one_face = 4;
        assert_eq!(
            draw(hide(&[mod_api::HumanoidPart::RightArm]), &[]),
            whole - one_face
        );
        assert_eq!(
            draw(hide(&[mod_api::HumanoidPart::Head]), &[]),
            whole - 2 * one_face,
            "the head's child, the hat, goes with it"
        );
        assert_eq!(
            draw(hide(&[]), &extra),
            2 * whole,
            "the armour layer copies every part"
        );
        assert_eq!(
            draw(hide(&[mod_api::HumanoidPart::RightArm]), &extra),
            2 * whole - one_face,
            "only the body loses the arm"
        );

        // `render_living_post` reads the parts as the body drew them: the hidden arm doesn't draw, and
        // nothing else changed (arismoko/horizon#2464).
        let (_, shown) = draw_parts(hide(&[]), &[]);
        let (_, armless) = draw_parts(hide(&[mod_api::HumanoidPart::RightArm]), &[]);
        for part in mod_api::HumanoidPart::ALL {
            let (a, b) = (shown.get(part), armless.get(part));
            if part == mod_api::HumanoidPart::RightArm {
                let (a, b) = (a.unwrap(), b.unwrap());
                assert!(a.visible && !b.visible, "the hidden arm doesn't draw");
                assert_eq!(a.pose, b.pose, "still posed where vanilla posed it");
            } else {
                assert_eq!(a, b, "{part:?}");
            }
        }
    }

    /// A mod's `render_worn_equipment` (horizon-rust/horizon#11072): inside its scope a mob's
    /// worn armour is left out, drawn without its glint or tinted, slot by slot, while the body
    /// and the other slots draw as before; outside it everything draws again.
    #[test]
    fn an_equipment_scope_hides_unglints_and_tints_a_mobs_armour() {
        use horizon_items::equipment::{FEET, HEAD};
        use mod_api::{EquipmentRender, EquipmentRenderPart as Drawn, EquipmentRenderSlot as Worn};
        let (zombie, data) = humanoid_zombie();
        let mut kind = (*zombie.kind).clone();
        // The armour models: the body's own, one face a part.
        for layer in ["zombie#helmet", "zombie#boots"] {
            kind.layers.insert(layer.into(), kind.model.clone());
        }
        let mut mob = Mob::new(
            1,
            std::sync::Arc::new(kind),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        );
        mob.age = 37;
        let atlas = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
        // Iron's one equipment layer, as the game resolves it for each slot that wears it.
        let iron = [("entity/equipment/humanoid/iron".to_owned(), None)];
        let draw = |mob: &Mob| {
            let worn: WornArmor = std::array::from_fn(|i| {
                mob.extra
                    .slot_stack(horizon_items::equipment::ARMOR[i])
                    .map(|_| &iron[..])
            });
            let mut out = Vec::new();
            render_with_hidden(
                mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &worn,
                &[],
                Default::default(),
                None,
                &mut out,
            );
            out
        };
        let whole = draw(&mob).len();
        assert!(whole > 0);
        let armour = |name: &str| Some(horizon_client::items::ItemStack::new(name, 1));
        mob.extra
            .equipment
            .set(HEAD, armour("minecraft:iron_helmet"));
        mob.extra
            .equipment
            .set(FEET, armour("minecraft:iron_boots"));
        // An enchanted helmet: its layer has the glint.
        let protection = vec![("minecraft:protection".into(), 1)];
        crate::mobs::gear::set(&mut mob, crate::mobs::gear::HEAD, protection);
        let worn = draw(&mob);
        assert_eq!(worn.len(), 3 * whole, "the body, the helmet, the boots");
        let (helmet, boots) = (&worn[whole..2 * whole], &worn[2 * whole..]);
        assert!(
            helmet
                .iter()
                .all(|v| v.layer == ARMOR_GLINT && v.color == 0xFFFF_FFFF)
        );
        assert!(boots.iter().all(|v| v.layer == 0 && v.color == 0xFFFF_FFFF));
        let scoped = |shown: EquipmentRender| {
            let _scope = EquipmentScope::enter(shown);
            draw(&mob)
        };
        // The helmet hidden: the boots follow the body.
        let mut shown = EquipmentRender::new();
        shown.hide(Worn::Head, Drawn::Base);
        let hidden = scoped(shown);
        assert_eq!(hidden.len(), 2 * whole);
        assert!(hidden[whole..].iter().all(|v| v.layer == 0));
        // Its glint hidden: the helmet draws plain.
        let mut shown = EquipmentRender::new();
        shown.hide(Worn::Head, Drawn::Glint);
        let plain = scoped(shown);
        assert_eq!(plain.len(), 3 * whole);
        assert!(
            plain[whole..2 * whole]
                .iter()
                .all(|v| v.layer == 0 && v.glint_uv == [0.0, 0.0])
        );
        // The boots tinted red: their green and blue go, red and alpha stay; the rest is as it was.
        let mut shown = EquipmentRender::new();
        shown.tint(Worn::Feet, 0xFF_0000);
        let red = scoped(shown);
        assert_eq!(red.len(), 3 * whole);
        assert!(red[2 * whole..].iter().all(|v| v.color == 0xFF00_00FF));
        let colors = |v: &[Vertex]| v.iter().map(|v| v.color).collect::<Vec<_>>();
        assert_eq!(colors(&red[..2 * whole]), colors(&worn[..2 * whole]));
        // A slot it doesn't wear changes nothing, and outside the scope everything draws again.
        let mut shown = EquipmentRender::new();
        shown.hide(Worn::Chest, Drawn::Base);
        assert_eq!(scoped(shown).len(), 3 * whole);
        assert_eq!(shown_equipment(), EquipmentRender::new());
        assert_eq!(colors(&draw(&mob)), colors(&worn));
    }

    #[test]
    fn creeper_swelling_uses_local_fuse_and_keeps_java_zero_division() {
        let mut mob = Mob::new(
            1,
            crate::mobs::sensor_scan_tests::kind(),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        );
        mob.previous_swell = 19;
        mob.swell = 20;
        assert_eq!(creeper_swelling(&mob, 0.5), 19.5 / 28.0);
        mob.max_swell = 40;
        assert_eq!(creeper_swelling(&mob, 0.5), 19.5 / 38.0);
        mob.max_swell = 2;
        assert!(creeper_swelling(&mob, 0.5).is_infinite());
        mob.previous_swell = 0;
        mob.swell = 0;
        assert!(creeper_swelling(&mob, 0.5).is_nan());
    }

    #[test]
    fn offhand_does_not_inherit_mainhand_components() {
        use crate::items::ItemStack;
        let mut extra = super::super::Extra::default();
        let mut main = ItemStack::new("minecraft:crossbow", 1);
        main.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Main hand".into()));
        extra.set_main_hand_stack(Some(main));
        extra.set_slot_id(
            horizon_items::equipment::OFFHAND,
            Some("minecraft:crossbow".into()),
        );
        assert_eq!(
            held_item_stack(&extra, false)
                .unwrap()
                .custom_name()
                .as_deref(),
            Some("Main hand")
        );
        assert!(
            held_item_stack(&extra, true)
                .unwrap()
                .custom_name()
                .is_none()
        );
        // An admiring piglin holds gold beside its crossbow. Both the item
        // id and the model components must come from the appropriate hand.
        extra.set_slot_id(
            horizon_items::equipment::OFFHAND,
            Some("minecraft:gold_ingot".into()),
        );
        assert_eq!(
            &*held_item_stack(&extra, true).unwrap().item,
            "minecraft:gold_ingot"
        );
        assert_eq!(
            &*held_item_stack(&extra, false).unwrap().item,
            "minecraft:crossbow"
        );
        extra.set_slot_id(horizon_items::equipment::OFFHAND, None);
        assert!(held_item_stack(&extra, true).is_none());
    }

    /// The elytra's flight yaw (`AvatarRenderer.extractFlightData`) lerps
    /// the velocity of the previous tick towards this tick's, so the model
    /// does not step once per tick while the view turns (#4104).
    #[test]
    fn the_elytra_flight_yaw_lerps_the_velocity_between_ticks() {
        use crate::mobs::{Anim, Brain, Flags, MobType, Part};
        use horizon_core::geometry::Vec3;
        use serde_json::json;
        use std::sync::Arc;

        let model = Arc::new(Part::parse("root", &json!({"children": {"head": {}}})));
        let kind = Arc::new(MobType {
            shadow: None,
            id: "minecraft:player".into(),
            species: Species::Zombie,
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            category: "misc".into(),
            max_health: 20.0,
            speed: 0.1,
            flying_speed: 0.02,
            attack_damage: 1.0,
            attack_knockback: 0.0,
            follow_range: 0.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            item_stack_limits: Default::default(),
            attribute_bounds: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Flags::default(),
            model,
            layers: Default::default(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Camel,
            sounds: [""; 3],
            sound_overrides: Default::default(),
            food: Arc::default(),
            tempt: Arc::default(),
            passenger_attachments: Vec::new(),
        });
        let gliding = |previous: Vec3, current: Vec3| {
            let mut m = Mob::new(1, kind.clone(), Vec3::ZERO, 0.0);
            m.extra.fall_flying = true;
            m.extra.fall_fly_ticks = 20;
            m.extra.previous_velocity = previous;
            m.velocity = current;
            m
        };
        let from = Vec3::new(0.4, 0.0, 0.4);
        let to = Vec3::new(0.0, 0.0, 0.5);
        let at = |m: &Mob, alpha: f32| base_transform_at(m, alpha, glam::Mat4::IDENTITY);
        let differs = |a: glam::Mat4, b: glam::Mat4| {
            a.to_cols_array()
                .iter()
                .zip(b.to_cols_array())
                .any(|(x, y)| (x - y).abs() > 1.0e-4)
        };
        let m = gliding(from, to);
        // The ends are the two ticks' velocities.
        assert!(!differs(at(&m, 0.0), at(&gliding(from, from), 0.0)));
        assert!(!differs(at(&m, 1.0), at(&gliding(to, to), 1.0)));
        // Between them the yaw is that of the lerped velocity, which is
        // neither end: the current tick's alone made it snap.
        let mid = Vec3::new(0.2, 0.0, 0.45);
        let half = at(&m, 0.5);
        assert!(!differs(half, at(&gliding(mid, mid), 0.5)));
        assert!(differs(half, at(&gliding(to, to), 0.5)));
        assert!(differs(half, at(&gliding(from, from), 0.5)));
    }

    /// Six faces whose corners span `min` to `max`.
    fn cube(min: [f32; 3], max: [f32; 3]) -> Vec<super::super::Polygon> {
        let corner = |p: [f32; 3]| [p[0], p[1], p[2], 0.0, 0.0];
        (0..6)
            .map(|_| super::super::Polygon {
                normal: [0.0; 3],
                vertices: [corner(min), corner(max), corner(min), corner(max)],
                cull_back: false,
            })
            .collect()
    }

    /// `StuckInBodyLayer.submit`: the copies drawn are the old loop's first
    /// ones, in its random order, and no more than `MAX_STUCK_DRAWN` however
    /// large the count a server sends.
    #[test]
    fn stuck_arrows_draw_the_first_copies_up_to_a_bound() {
        use horizon_core::random::{LegacyRandom, RandomSource};
        // Cubes per part of `BODY_PARTS`; the legs' second cube is a test's.
        let cubes = |part: usize| [1, 1, 1, 1, 2, 2][part];
        // The draws as the loop over the whole count made them.
        let unbounded = |id: i32, count: i32, on_surface: bool| {
            let mut random = LegacyRandom::new(i64::from(id));
            let mut out = Vec::new();
            for _ in 0..count {
                let part = random.next_int_bound(6).unwrap() as usize;
                let cube = random.next_int_bound(cubes(part) as i32).unwrap() as usize;
                let mut mid = [
                    random.next_float(),
                    random.next_float(),
                    random.next_float(),
                ];
                if on_surface {
                    let plane = random.next_int_bound(3).unwrap() as usize;
                    mid[plane] = snap_to_face(mid[plane]);
                }
                out.push(StuckPlacement { part, cube, mid });
            }
            out
        };
        for (id, on_surface) in [(7, false), (7, true), (-123_456, false)] {
            for count in [0, 1, 5, 40, MAX_STUCK_DRAWN] {
                let drawn: Vec<_> = stuck_placements(id, count, on_surface, cubes).collect();
                assert_eq!(drawn, unbounded(id, count, on_surface), "{id} {count}");
            }
            let at_most = unbounded(id, MAX_STUCK_DRAWN, on_surface);
            for count in [MAX_STUCK_DRAWN + 1, i32::MAX] {
                let drawn: Vec<_> = stuck_placements(id, count, on_surface, cubes).collect();
                assert_eq!(drawn, at_most, "{id} {count}");
            }
        }
        // A part with no cube ends the copies there.
        let headless = stuck_placements(7, 40, false, |part| usize::from(part != 0)).count();
        let first_head = unbounded(7, 40, false).iter().position(|s| s.part == 0);
        assert_eq!(Some(headless), first_head);
    }

    /// `ModelPart.cubes`: the corners of each cube of a part, six faces apiece.
    #[test]
    fn cube_bounds_follow_the_faces_of_each_cube() {
        let mut polygons = cube([-4.0, -8.0, -2.0], [4.0, 0.0, 2.0]);
        polygons.extend(cube([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]));
        let part = Part {
            name: "body".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons,
            children: Vec::new(),
        };
        assert_eq!(
            cube_bounds(&part),
            vec![
                [-4.0, -8.0, -2.0, 4.0, 0.0, 2.0],
                [0.0, 0.0, 0.0, 1.0, 2.0, 3.0]
            ]
        );
    }

    /// `StuckInBodyLayer.snapToFace`.
    #[test]
    fn a_stuck_item_snaps_to_the_far_or_the_middle_face() {
        assert_eq!(snap_to_face(0.75), 1.0);
        assert_eq!(snap_to_face(0.5), 0.5);
        assert_eq!(snap_to_face(0.1), 0.5);
    }

    #[test]
    fn shoulder_parrot_textures_follow_the_variant() {
        assert_eq!(shoulder_parrot_texture(0), "entity/parrot/parrot_red_blue");
        assert_eq!(shoulder_parrot_texture(4), "entity/parrot/parrot_grey");
        // Parrot.Variant.byId clamps an id out of range.
        assert_eq!(shoulder_parrot_texture(5), "entity/parrot/parrot_grey");
        assert_eq!(shoulder_parrot_texture(-1), "entity/parrot/parrot_red_blue");
    }

    /// VillagerProfessionLayer's textures (type, or baby for a baby;
    /// profession; level) of every villager and zombie villager are in the
    /// prepared resources: a missing one draws nothing.
    #[test]
    fn villager_layers_name_textures_that_exist() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        for kind in ["minecraft:villager", "minecraft:zombie_villager"] {
            let kind = std::sync::Arc::new(types.types[kind].clone());
            for baby in [false, true] {
                for t in 0..VILLAGER_TYPES.len() {
                    for p in 0..crate::mobs::villager::profession_table().entries().len() {
                        let mut m =
                            Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
                        m.extra.age = if baby { -24000 } else { 0 };
                        (m.extra.variant, m.extra.villager_level) = (t as u8, 5);
                        m.extra.profession =
                            super::super::villager::ProfessionState::from_index(p as u8);
                        let layers = layers(&m, m.age as f32, &Default::default());
                        let group = if baby { "/baby/" } else { "/type/" };
                        assert!(
                            layers.iter().any(|l| l.texture.contains(group)),
                            "{kind:?} baby={baby}: no {group} layer"
                        );
                        for layer in layers {
                            let png = root
                                .join(format!("assets/minecraft/textures/{}.png", layer.texture));
                            assert!(png.is_file(), "missing {}", png.display());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sulfur_cube_opaque_core_is_submitted_before_its_translucent_shell() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let mut types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types.remove("minecraft:sulfur_cube").unwrap());
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            resolved: Default::default(),
        };
        let atlas = |name: &str| {
            Some(if name.contains("_inner") {
                [0.0, 0.0, 0.25, 0.25]
            } else {
                [0.5, 0.5, 0.75, 0.75]
            })
        };
        for baby in [false, true] {
            let mut mob = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.age = if baby { -24000 } else { 0 };
            super::super::sulfur_cube::finalize(&mut mob);
            assert_eq!(mob.is_baby(), baby);
            for posed in [false, true] {
                let mut vertices = Vec::new();
                if posed {
                    render_posed(
                        &mob,
                        0.5,
                        glam::Mat4::IDENTITY,
                        &atlas,
                        0xf0f0,
                        &data,
                        &Default::default(),
                        &mut vertices,
                    );
                } else {
                    render(
                        &mob,
                        0.5,
                        glam::DVec3::ZERO,
                        &atlas,
                        0xf0f0,
                        &data,
                        &mut vertices,
                    );
                }
                assert_eq!(
                    vertices.len(),
                    48,
                    "both real six-face models must be drawn"
                );
                assert!(
                    vertices[..24].iter().all(|v| v.uv[0] <= 0.25),
                    "opaque core must precede shell"
                );
                assert!(vertices[24..].iter().all(|v| v.uv[0] >= 0.5));
            }
            mob.extra.set_body(Some("minecraft:stone".into()));
            let mut vertices = Vec::new();
            render(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &mut vertices,
            );
            assert_eq!(
                vertices.len(),
                24,
                "a swallowed block still hides the default core"
            );
            assert!(vertices.iter().all(|v| v.uv[0] >= 0.5));
        }
    }
    /// CopperGolemRenderer: the emissive eye layer draws the eye texture on
    /// the body's model, every polygon of it, at each oxidation level. The
    /// prepared `copper_golem#eyes` model has no polygons, so the eyes
    /// showed the body texture's black sockets.
    #[test]
    fn copper_golem_eyes_are_drawn_on_the_body_model() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:copper_golem"].clone());
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            resolved: Default::default(),
        };
        let atlas = |name: &str| {
            Some(if name.contains("copper_golem_eyes") {
                [0.5, 0.5, 0.75, 0.75]
            } else {
                [0.0, 0.0, 0.25, 0.25]
            })
        };
        for variant in 0..4 {
            let mut mob = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            mob.extra.variant = variant;
            let mut vertices = Vec::new();
            render(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &mut vertices,
            );
            let body = vertices.iter().filter(|v| v.uv[0] <= 0.25).count();
            let eyes = vertices.iter().filter(|v| v.uv[0] >= 0.5).count();
            assert!(body > 0, "variant {variant}: no body drawn");
            assert_eq!(
                eyes, body,
                "variant {variant}: the eye layer covers the body"
            );
        }
    }
    /// `VexModel.translateToHand`: the held item is drawn at 0.55 scale and
    /// moved by the vex's own offset, so the vex's sword is the tiny one
    /// vanilla shows. It was drawn at the full size, like a zombie's.
    #[test]
    fn the_vex_holds_its_item_at_the_small_scale() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:vex"].clone());
        let mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            resolved: Default::default(),
        };
        for left in [false, true] {
            let t = held_item_transform(&mob, 0.5, glam::DVec3::ZERO, &data, left).unwrap();
            // The model's own part scale is 1, so the item's scale is the
            // renderer's: `poseStack.scale(0.55F, 0.55F, 0.55F)`.
            for axis in [t.x_axis, t.y_axis, t.z_axis] {
                let scale = axis.truncate().length();
                assert!(
                    (scale - 0.55).abs() < 1.0e-5,
                    "the vex's item is drawn at 0.55 scale, got {scale}"
                );
            }
        }
    }

    /// `CrossedArmsItemLayer`: a villager and a wandering trader show their
    /// main hand's item over the folded arms, and nothing in the off hand.
    #[test]
    fn villagers_and_traders_hold_items_in_crossed_arms() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let data = MobData::load(
            &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            &facts,
        );
        for id in ["minecraft:villager", "minecraft:wandering_trader"] {
            let kind = std::sync::Arc::new(types.types[id].clone());
            let mut mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
            let main = mob.main_arm_left();
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, main).is_none());
            mob.extra.set_main_hand(Some("minecraft:emerald".into()));
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, main).is_some());
            assert!(held_item_transform(&mob, 1.0, glam::DVec3::ZERO, &data, !main).is_none());
        }
    }
    /// PhantomRenderer.scale moves the model down into its 0.5-high box:
    /// the model's origin ends 1.501 - 1.3125 blocks above the feet (times
    /// the size's scale), not 1.501 like other mobs (#3008).
    #[test]
    fn the_phantom_model_sits_in_its_box() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:phantom"].clone());
        let mut mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        for (size, scale) in [(0, 1.0f32), (5, 1.75)] {
            mob.extra.size = size;
            let origin =
                base_transform(&mob, 1.0, glam::DVec3::ZERO).transform_point3(glam::Vec3::ZERO);
            let y = (1.501 - 1.3125) * scale;
            assert!(
                (origin.y - y).abs() < 1.0e-4,
                "size {size}: origin {origin}"
            );
            assert!(
                (origin.z.abs() - 0.1875 * scale).abs() < 1.0e-4,
                "size {size}: origin {origin}"
            );
        }
    }
    /// `EquipmentLayerRenderer.renderLayers`: the extra armour layers (a
    /// player's) take their own overlay, `NO_OVERLAY` or the armour glint,
    /// never the hurt tint of the body, and the glint samples the model UV.
    #[test]
    fn extra_armour_layers_carry_their_glint_not_the_hurt_tint() {
        // The batch gives these overlays their own materials (13, the water
        // mask, draws depth only): the glint must not be one of them.
        use crate::gfx::{
            ENERGY_SWIRL_OVERLAY, SHADOW_OVERLAY, TEXT_POLYGON_OFFSET_OVERLAY,
            TRANSLUCENT_GLINT_ITEM_OVERLAY, TRANSLUCENT_ITEM_OVERLAY, TRANSLUCENT_PARTICLE_OVERLAY,
            WATER_MASK_OVERLAY,
        };
        assert!(
            ![
                WATER_MASK_OVERLAY,
                SHADOW_OVERLAY,
                TRANSLUCENT_PARTICLE_OVERLAY,
                TRANSLUCENT_ITEM_OVERLAY,
                TRANSLUCENT_GLINT_ITEM_OVERLAY,
                ENERGY_SWIRL_OVERLAY,
                TEXT_POLYGON_OFFSET_OVERLAY,
            ]
            .contains(&ARMOR_GLINT)
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
        let mut mob = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
        mob.hurt_time = 5;
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            resolved: Default::default(),
        };
        let atlas = |_: &str| Some([0.0, 0.0, 0.25, 0.25]);
        let part = &kind.model;
        for overlay in [ARMOR_GLINT, 0] {
            let mut vertices = Vec::new();
            render_with(
                &mob,
                0.5,
                glam::DVec3::ZERO,
                &atlas,
                0xf0f0,
                &data,
                &Default::default(),
                &[(part, [0.5, 0.5, 0.75, 0.75], 0xFFFF_FFFF, overlay, false)],
                &mut vertices,
            );
            let (armour, body): (Vec<&Vertex>, Vec<&Vertex>) =
                vertices.iter().partition(|v| v.uv[0] >= 0.5);
            assert!(!armour.is_empty() && !body.is_empty());
            // The hurt body keeps its red overlay.
            let hurt = body_overlay(&mob, 0.5);
            assert_ne!(hurt, 0);
            assert!(body.iter().all(|v| v.layer == hurt));
            assert!(armour.iter().all(|v| v.layer == overlay));
            // The glint scrolls the layer's model UV (0..1 over the texture).
            for v in &armour {
                if overlay == ARMOR_GLINT {
                    assert!((v.glint_uv[0] - (v.uv[0] - 0.5) * 4.0).abs() < 1e-4);
                    assert!((v.glint_uv[1] - (v.uv[1] - 0.5) * 4.0).abs() < 1e-4);
                } else {
                    assert_eq!(v.glint_uv, [0.0, 0.0]);
                }
            }
        }
    }
    #[test]
    fn a_dyed_leather_armour_keeps_its_colour() {
        // EquipmentLayerRenderer.getColorForLayer: a dyeable layer takes
        // the stack's dye colour, the colour it has when undyed otherwise.
        let mut stack = crate::items::ItemStack {
            item: "minecraft:leather_helmet".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        // Undyed leather: the colour the leather layer has when undyed.
        assert_eq!(armor_tint(Some(0xA06540), Some(&stack)), abgr(0xA06540));
        // Dyed: the stack's own colour.
        stack.data_mut().dyed_color = Some(0x0000FF);
        assert_eq!(armor_tint(Some(0xA06540), Some(&stack)), abgr(0x0000FF));
        // A layer that is not dyeable is not tinted.
        assert_eq!(armor_tint(None, Some(&stack)), 0xFFFF_FFFF);
    }

    /// `ColorLerper.getLerpedColor(Type.SHEEP, ageInTicks)`: the points the
    /// 26.3 jar gave — 0, half a step, one whole step, a step boundary, the
    /// last step back into white, and the 400-tick (16-step) wrap.
    #[test]
    fn jeb_wool_lerps_through_the_sixteen_dye_colours() {
        assert_eq!(jeb_wool_rgb(0.0), 0xE6E6E6);
        assert_eq!(jeb_wool_rgb(12.5), 0xD0A37D);
        assert_eq!(jeb_wool_rgb(25.0), 0xBA6015);
        assert_eq!(jeb_wool_rgb(25.0), sheep_wool_rgb(1));
        assert_eq!(jeb_wool_rgb(50.0), 0x953A8D);
        assert_eq!(jeb_wool_rgb(399.0), 0xDDDDDD);
        assert_eq!(jeb_wool_rgb(400.0), 0xE6E6E6);
    }

    fn sheep_layer(m: &Mob, model: &str, age: f32) -> Option<Layer> {
        layers(m, age, &Default::default())
            .into_iter()
            .find(|l| l.model == model)
    }

    /// SheepRenderer.extractRenderState's `jeb_` magic name: the wool layer
    /// takes the lerped colour and cycles with the age, a white `jeb_` sheep
    /// keeps its undercoat (SheepWoolUndercoatLayer draws for it too), and
    /// the dye colour, the baby model and the shears still win.
    #[test]
    fn a_jeb_sheep_cycles_its_wool_colour_and_keeps_the_undercoat() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:sheep"].clone());
        let mut mob = Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0);
        mob.custom_name = Some("jeb_".into());
        let young = sheep_layer(&mob, "sheep#wool", 0.0)
            .expect("an unsheared sheep draws its wool")
            .color;
        assert_eq!(young, abgr(0xE6E6E6));
        let older = sheep_layer(&mob, "sheep#wool", 100.0)
            .expect("an unsheared sheep draws its wool")
            .color;
        assert_ne!(young, older, "the colour cycles with the age");
        let undercoat = sheep_layer(&mob, "sheep#wool_undercoat", 0.0)
            .expect("a jeb_ sheep keeps its undercoat");
        assert_eq!(undercoat.color, abgr(0xE6E6E6));
        // A name that is not exactly "jeb_" leaves a dyed sheep's colour, and
        // a non-white sheep has its undercoat too.
        mob.custom_name = Some("jeb".into());
        mob.extra.color = 14;
        let dyed = abgr(sheep_wool_rgb(14));
        assert_eq!(
            sheep_layer(&mob, "sheep#wool", 0.0)
                .expect("an unsheared sheep draws its wool")
                .color,
            dyed
        );
        assert_eq!(
            sheep_layer(&mob, "sheep#wool_undercoat", 0.0)
                .expect("a dyed sheep keeps its undercoat")
                .color,
            dyed
        );
        // A baby `jeb_` sheep gets the lerped colour on the baby wool and
        // never an undercoat.
        mob.custom_name = Some("jeb_".into());
        mob.extra.color = 0;
        mob.extra.age = -24000;
        assert!(mob.is_baby());
        assert_eq!(
            sheep_layer(&mob, "sheep_baby#wool", 50.0)
                .expect("a baby sheep draws its wool")
                .color,
            abgr(0x953A8D)
        );
        assert!(sheep_layer(&mob, "sheep#wool_undercoat", 50.0).is_none());
        // Sheared: no wool layer at all.
        mob.extra.age = 0;
        mob.extra.sheared = true;
        assert!(sheep_layer(&mob, "sheep#wool", 0.0).is_none());
    }
}

#[cfg(test)]
mod scroll_tests {
    #[test]
    fn a_pufferfish_bobs_by_eight_hundredths() {
        // PufferfishRenderer.setupRotations: cos(ageInTicks * 0.05) * 0.08.
        assert!((super::pufferfish_bob_y(0.0) - 0.08).abs() < 1e-6);
        let quarter = std::f32::consts::FRAC_PI_2 / 0.05;
        assert!(super::pufferfish_bob_y(quarter).abs() < 1e-3);
        assert!(super::pufferfish_bob_y(2.0 * quarter) < -0.0799);
    }

    use super::*;

    #[test]
    fn the_scrolling_texture_wraps_around() {
        // RenderTypes.breezeWind: the breeze's wind layer's texture scrolls
        // by 0.02 of the texture per tick, the wind charge's by 0.03.
        assert_eq!(scroll_offset(0.0, BREEZE_WIND_SCROLL), 0.0);
        assert!((scroll_offset(25.0, BREEZE_WIND_SCROLL) - 0.5).abs() < 1e-6);
        assert!((scroll_offset(50.0, BREEZE_WIND_SCROLL) - 0.0).abs() < 1e-6);
        assert!((scroll_offset(50.0, WIND_CHARGE_SCROLL) - 0.5).abs() < 1e-6);
        assert!((scroll_offset(100.0, WIND_CHARGE_SCROLL) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn the_scrolled_u_wraps_inside_the_texture() {
        // Without a scroll the U is the vertex's own.
        assert_eq!(scroll_u(0.25, 0.0), 0.25);
        assert_eq!(scroll_u(1.0, 0.0), 1.0);
        // With one, it moves and wraps around the texture.
        assert!((scroll_u(0.25, 0.5) - 0.75).abs() < 1e-6);
        assert!((scroll_u(0.75, 0.5) - 0.25).abs() < 1e-6);
        assert!((scroll_u(1.0, 0.5) - 0.5).abs() < 1e-6);
    }

    /// `SheepWoolLayer.submit` returns early on a sheared sheep, but
    /// `SheepWoolUndercoatLayer.submit` has no isSheared guard: a sheared
    /// dyed sheep keeps its undercoat and loses only its wool.
    #[test]
    fn a_sheared_dyed_sheep_keeps_its_wool_undercoat() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(types.types["minecraft:sheep"].clone());
        let sheep = |color: u8, sheared: bool, baby: bool| {
            let mut m = Mob::new(1, kind.clone(), horizon_core::geometry::Vec3::ZERO, 0.0);
            m.extra.color = color;
            m.extra.sheared = sheared;
            m.extra.age = if baby { -24000 } else { 0 };
            m
        };
        // The sheep's wool layers, model, texture and colour, in submission order.
        let wool = |m: &Mob| -> Vec<(String, String, u32)> {
            layers(m, 0.0, &Default::default())
                .into_iter()
                .filter(|l| l.model.contains("#wool"))
                .map(|l| (l.model.into_owned(), l.texture.into_owned(), l.color))
                .collect()
        };
        let dyed = abgr(sheep_wool_rgb(1));
        let layer = |model: &str, texture: &str| (model.to_owned(), texture.to_owned(), dyed);

        // The regression: the pre-fix single `!sheared` guard skipped both
        // layers, so a sheared dyed sheep drew nothing. It must keep the
        // undercoat alone, in its own colour and texture.
        assert_eq!(
            wool(&sheep(1, true, false)),
            [layer(
                "sheep#wool_undercoat",
                "entity/sheep/sheep_wool_undercoat"
            )]
        );
        // Guards, already green before the fix: a sheared white sheep has no
        // undercoat to keep, a baby has no undercoat at all, and an unsheared
        // dyed sheep draws both layers as before.
        assert!(wool(&sheep(0, true, false)).is_empty());
        assert_eq!(
            wool(&sheep(1, false, true)),
            [layer("sheep_baby#wool", "entity/sheep/sheep_wool_baby")]
        );
        assert!(wool(&sheep(1, true, true)).is_empty());
        assert_eq!(
            wool(&sheep(1, false, false)),
            [
                layer("sheep#wool_undercoat", "entity/sheep/sheep_wool_undercoat"),
                layer("sheep#wool", "entity/sheep/sheep_wool")
            ]
        );
    }

    /// `anim::Poses` keys parts by a hash of their name: within any vanilla model, two
    /// different names never share one (they would share a pose).
    #[test]
    fn no_two_part_names_share_a_key() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        fn names<'a>(p: &'a Part, out: &mut Vec<&'a str>) {
            out.push(&p.name);
            for c in &p.children {
                names(c, out);
            }
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let mut models = 0;
        for kind in types.types.values() {
            for model in std::iter::once(&kind.model).chain(kind.layers.values()) {
                let mut all = Vec::new();
                names(model, &mut all);
                let mut keys = std::collections::HashMap::new();
                for name in all {
                    let other = *keys.entry(super::super::anim::key(name)).or_insert(name);
                    assert_eq!(other, name, "{} shares a key", kind.id);
                }
                models += 1;
            }
        }
        assert!(models > 100);
    }

    /// The CPU cost of posing a mob farm (tools/stress_scenes.py `stress_mobs`): 1000 mobs of
    /// 8 kinds through `render_with`, as `mob_geometry` draws them each frame. Prints the median
    /// thread CPU time per frame (busy machines stretch wall time), the vertices and a hash of them (the same hash before and after a speed-up
    /// means the same picture). `cargo test --profile fast -p horizon-client --lib
    /// benchmark_mob_farm_geometry -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual benchmark; needs the prepared resources"]
    fn benchmark_mob_farm_geometry() {
        use std::{hash::Hasher, hint::black_box, time::Instant};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client/26.3");
        if !root.join("entity-models.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        let kinds = [
            "husk", "creeper", "cow", "pig", "sheep", "chicken", "villager", "spider",
        ]
        .map(|k| std::sync::Arc::new(types.types[&format!("minecraft:{k}")].clone()));
        let mobs: Vec<Mob> = (0..1000u32)
            .map(|i| {
                let (layer, row, col) = (i / 200, i / 20 % 10, i % 20);
                let at = horizon_core::geometry::Vec3::new(
                    -14.25 + 1.5 * f64::from(col),
                    1.0 + 2.5 * f64::from(layer),
                    8.0 + 2.0 * f64::from(row),
                );
                let mut m = Mob::new(i + 1, kinds[i as usize % 8].clone(), at, 180.0);
                m.previous = at;
                m.age = i;
                m
            })
            .collect();
        let data = MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            resolved: Default::default(),
        };
        let atlas = |name: &str| {
            let h = name.len() as f32 / 64.0;
            Some([h, h, h + 0.25, h + 0.25])
        };
        let camera = glam::DVec3::new(0.5, 9.6, 0.5);
        let mut out = Vec::new();
        let frame = |out: &mut Vec<Vertex>| {
            out.clear();
            for m in &mobs {
                render_with(
                    m,
                    0.5,
                    camera,
                    &atlas,
                    0xf0f0,
                    &data,
                    &Default::default(),
                    &[],
                    out,
                );
            }
        };
        frame(&mut out);
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        hash.write(bytemuck::cast_slice(&out));
        // The thread's own CPU time where the platform gives it, else wall time.
        let timed = |f: &mut dyn FnMut()| {
            let (wall, cpu) = (Instant::now(), horizon_client_render::spans::thread_cpu());
            f();
            match (horizon_client_render::spans::thread_cpu(), cpu) {
                (Some(end), Some(start)) => (end - start).as_secs_f64() * 1000.0,
                _ => wall.elapsed().as_secs_f64() * 1000.0,
            }
        };
        let mut times = Vec::new();
        for _ in 0..30 {
            times.push(timed(&mut || frame(black_box(&mut out))));
        }
        times.sort_by(f64::total_cmp);
        println!(
            "mob farm: 1000 mobs, {} vertices, {:.3} ms/frame (median of 30, min {:.3}), hash {:016x}",
            out.len(),
            times[15],
            times[0],
            hash.finish()
        );
        // Where it goes: the body's poses alone (state, pose map, setupAnim), then the
        // body's vertices alone from those poses.
        let median = |mut f: Box<dyn FnMut() + '_>| {
            let mut t: Vec<f64> = (0..30).map(|_| timed(&mut f)).collect();
            t.sort_by(f64::total_cmp);
            t[15]
        };
        let posed: Vec<(std::sync::Arc<Part>, Poses)> = mobs
            .iter()
            .map(|m| {
                let model = m.kind.model.clone();
                let mut poses = Poses::new(&model);
                anim::setup(m, &anim::state(m, 0.5), &mut poses, &data.animations);
                (model, poses)
            })
            .collect();
        let poses_ms = median(Box::new(|| {
            for m in &mobs {
                let model = m.kind.model.clone();
                let mut poses = Poses::new(&model);
                anim::setup(m, &anim::state(m, 0.5), &mut poses, &data.animations);
                black_box(&poses);
            }
        }));
        let mut body = Vec::new();
        let draw_ms = median(Box::new(|| {
            body.clear();
            for (m, (model, poses)) in mobs.iter().zip(&posed) {
                let base = base_transform(m, 0.5, camera);
                draw(
                    model,
                    base,
                    poses,
                    [0.0, 0.0, 0.25, 0.25],
                    u32::MAX,
                    0xf0f0,
                    0,
                    &mut body,
                );
            }
            black_box(&body);
        }));
        println!(
            "  body poses only {poses_ms:.3} ms; body vertices only {draw_ms:.3} ms ({} vertices)",
            body.len()
        );
    }
}

#[cfg(test)]
mod emissive_tests {
    use super::*;
    use crate::mobs::{Polygon, Species};
    use horizon_optifine::{blink::Blinks, emissive::Emissive, random_textures::RandomTextures};
    use std::sync::{Arc, RwLock};

    /// A silverfish (drawn with its type's texture) whose model is one quad.
    fn silverfish() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.species = Species::Silverfish;
        k.texture = "entity/silverfish".into();
        k.model = Arc::new(Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![Polygon {
                normal: [0.0, 0.0, -1.0],
                cull_back: false,
                vertices: [
                    [0.0, 0.0, 0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0, 1.0, 1.0],
                    [0.0, 1.0, 0.0, 0.0, 1.0],
                ],
            }],
            children: Vec::new(),
        });
        Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0)
    }

    fn data(files: &[&str]) -> MobData {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-render-emissive-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            // Rules files pick variant 2; images are only their names.
            let text = if file.ends_with(".properties") {
                "textures.1=2"
            } else {
                ""
            };
            std::fs::write(path, text).unwrap();
        }
        let random = RandomTextures::scan(
            &horizon_assets::resource_source::ResourceSource::directory(root.to_owned()),
        );
        let blinks = Blinks::scan(
            &horizon_assets::resource_source::ResourceSource::directory(root.to_owned()),
            &random,
        );
        let mut emissive = Emissive::scan(
            &horizon_assets::resource_source::ResourceSource::directory(root.to_owned()),
            &random,
        );
        emissive.pair_beside(
            &horizon_assets::resource_source::ResourceSource::directory(root.to_owned()),
            blinks.files(),
        );
        let _ = std::fs::remove_dir_all(&root);
        MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: RwLock::new(random),
            emissive: RwLock::new(Arc::new(emissive)),
            blinks: RwLock::new(blinks),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            resolved: Default::default(),
        }
    }

    /// Every sprite the test atlas has, each at its own rect.
    fn atlas(name: &str) -> Option<[f32; 4]> {
        match name {
            "entity/silverfish" => Some([0.0, 0.0, 0.5, 0.5]),
            "entity/silverfish_e" => Some([0.5, 0.5, 1.0, 1.0]),
            "entity/silverfish_blink" => Some([0.0, 0.5, 0.5, 1.0]),
            _ => None,
        }
    }

    #[test]
    fn an_emissive_overlay_draws_the_body_again_at_full_brightness() {
        let lit = (3 * 16) | ((5 * 16) << 8);
        let m = silverfish();
        let draw = |data: &MobData| {
            let mut out = Vec::new();
            render_with(
                &m,
                0.0,
                glam::DVec3::ZERO,
                &atlas,
                lit,
                data,
                &Default::default(),
                &[],
                &mut out,
            );
            out
        };
        // No `_e` file: one quad, as vanilla.
        let plain = draw(&data(&["assets/minecraft/textures/entity/silverfish.png"]));
        assert_eq!(plain.len(), 4);
        assert!(plain.iter().all(|v| v.light == lit));
        let glowing = draw(&data(&[
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_e.png",
        ]));
        assert_eq!(glowing.len(), 8);
        let (body, glow) = glowing.split_at(4);
        assert!(body.iter().all(|v| v.light == lit));
        assert!(glow.iter().all(|v| v.light == FULL_BRIGHT));
        // The overlay samples its own sprite, and keeps the body's shading normals.
        assert!(glow.iter().all(|v| v.uv[0] >= 0.5 && v.uv[1] >= 0.5));
        assert!(body.iter().all(|v| v.uv[0] <= 0.5 && v.uv[1] <= 0.5));
        assert_eq!(
            glow.iter().map(|v| v.normal).collect::<Vec<_>>(),
            body.iter().map(|v| v.normal).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_blink_frame_replaces_the_texture_for_a_few_ticks() {
        let data = data(&[
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_blink.png",
        ]);
        let uvs = |age: u32| {
            let mut m = silverfish();
            m.age = age;
            let mut out = Vec::new();
            render_with(
                &m,
                0.0,
                glam::DVec3::ZERO,
                &atlas,
                0,
                &data,
                &Default::default(),
                &[],
                &mut out,
            );
            out.iter().map(|v| v.uv).collect::<Vec<_>>()
        };
        // Entity 1's phase is 0x9E3779B1 % 150 = 61: of every 150 ticks, it blinks for the
        // default 5 from age 89 (89 + 61 = 150).
        let shut = |uv: &Vec<[f32; 2]>| uv.iter().all(|uv| uv[0] <= 0.5 && uv[1] >= 0.5);
        let open = |uv: &Vec<[f32; 2]>| uv.iter().all(|uv| uv[0] <= 0.5 && uv[1] <= 0.5);
        assert!(open(&uvs(88)));
        assert!(shut(&uvs(89)));
        assert!(shut(&uvs(93)));
        assert!(open(&uvs(94)));
        assert!(shut(&uvs(89 + 150)));
    }

    #[test]
    fn a_blink_shows_its_own_glow_or_none() {
        let atlas = |name: &str| {
            let u = match name {
                "entity/silverfish" => 0.0,
                "entity/silverfish_e" => 0.25,
                "entity/silverfish_blink" => 0.5,
                "entity/silverfish_blink_e" => 0.75,
                _ => return None,
            };
            Some([u, 0.0, u + 0.25, 1.0])
        };
        let base = [
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_e.png",
            "assets/minecraft/textures/entity/silverfish_blink.png",
        ];
        let at = |data: &MobData, age: u32| {
            let mut m = silverfish();
            m.age = age;
            drawn(&m, data, &atlas)
        };
        // Age 89 is shut (see above), 0 open.
        let plain = data(&base);
        let open = at(&plain, 0);
        assert_eq!(open.len(), 8);
        assert!(inside(&open[4..], 0.25), "the texture's glow");
        let shut = at(&plain, 89);
        assert_eq!(shut.len(), 4, "no glowing open eyes over the shut ones");
        assert!(inside(&shut, 0.5));
        let mut own = base.to_vec();
        own.push("assets/minecraft/textures/entity/silverfish_blink_e.png");
        let shut = at(&data(&own), 89);
        assert_eq!(shut.len(), 8);
        assert!(inside(&shut[4..], 0.75), "the frame's own glow");
    }

    /// Draws `m` and returns each vertex's (uv, light).
    fn drawn(m: &Mob, data: &MobData, atlas: Atlas) -> Vec<([f32; 2], u32)> {
        let mut out = Vec::new();
        render_with(
            m,
            0.0,
            glam::DVec3::ZERO,
            atlas,
            7,
            data,
            &Default::default(),
            &[],
            &mut out,
        );
        out.iter().map(|v| (v.uv, v.light)).collect()
    }

    /// Whether every vertex samples inside the rect starting at `at` (each rect is 0.25 wide).
    fn inside(vertices: &[([f32; 2], u32)], at: f32) -> bool {
        vertices
            .iter()
            .all(|(uv, _)| (at..=at + 0.25).contains(&uv[0]))
    }

    #[test]
    fn a_layer_glows_with_its_own_overlay() {
        let mut m = silverfish();
        let k = Arc::make_mut(&mut m.kind);
        k.species = Species::Sheep;
        k.layers.insert("sheep#wool".into(), k.model.clone());
        let data = data(&[
            "assets/minecraft/textures/entity/sheep/sheep.png",
            "assets/minecraft/textures/entity/sheep/sheep_wool.png",
            "assets/minecraft/textures/entity/sheep/sheep_wool_e.png",
        ]);
        let atlas = |name: &str| {
            let u = match name {
                "entity/sheep/sheep" => 0.0,
                "entity/sheep/sheep_wool" => 0.25,
                "entity/sheep/sheep_wool_e" => 0.5,
                _ => return None,
            };
            Some([u, 0.0, u + 0.25, 1.0])
        };
        let v = drawn(&m, &data, &atlas);
        assert_eq!(v.len(), 12, "body, wool, wool's glow");
        assert!(inside(&v[..4], 0.0) && inside(&v[4..8], 0.25) && inside(&v[8..], 0.5));
        assert!(v[..8].iter().all(|(_, light)| *light == 7));
        assert!(v[8..].iter().all(|(_, light)| *light == FULL_BRIGHT));
    }

    #[test]
    fn a_random_variant_glows_with_its_own_overlay_only() {
        const RANDOM: &str = "assets/minecraft/optifine/random/entity";
        let atlas = |name: &str| {
            let u = match name {
                "entity/silverfish" => 0.0,
                "entity/silverfish_e" => 0.25,
                "minecraft:optifine/random/entity/silverfish2" => 0.5,
                "minecraft:optifine/random/entity/silverfish2_e" => 0.75,
                _ => return None,
            };
            Some([u, 0.0, u + 0.25, 1.0])
        };
        let seen = || crate::mobs::random_textures::Seen {
            biome: None,
            y: 64,
            blocks: Vec::new(),
            moon_phase: 0,
            day_time: 0,
            weather: horizon_optifine::random::Weather::Clear,
        };
        let props = format!("{RANDOM}/silverfish.properties");
        let variant = format!("{RANDOM}/silverfish2.png");
        let variant_e = format!("{RANDOM}/silverfish2_e.png");
        let base = [
            "assets/minecraft/textures/entity/silverfish.png",
            "assets/minecraft/textures/entity/silverfish_e.png",
        ];
        for own_overlay in [true, false] {
            let mut files = vec![base[0], base[1], props.as_str(), variant.as_str()];
            if own_overlay {
                files.push(variant_e.as_str());
            }
            let data = data(&files);
            let mut m = silverfish();
            {
                let textures = data.random_textures.read().unwrap();
                crate::mobs::random_textures::choose(
                    &mut m,
                    &textures,
                    &data,
                    &Default::default(),
                    Some(seen()),
                    0,
                    &|_| None,
                );
            }
            let v = drawn(&m, &data, &atlas);
            assert!(inside(&v[..4], 0.5), "the variant");
            if own_overlay {
                assert_eq!(v.len(), 8);
                assert!(inside(&v[4..], 0.75), "its own overlay, not silverfish_e");
            } else {
                assert_eq!(v.len(), 4, "silverfish_e doesn't glow over silverfish2");
            }
        }
    }

    #[test]
    fn worn_armour_glows_when_its_extra_layer_says_so() {
        let m = silverfish();
        let data = data(&[]);
        let atlas = |_: &str| Some([0.0, 0.0, 0.5, 0.5]);
        let part = m.kind.model.clone();
        let extra = [
            (part.as_ref(), [0.5, 0.0, 1.0, 0.5], 0xFFFF_FFFF, 0, false),
            (part.as_ref(), [0.5, 0.5, 1.0, 1.0], 0xFFFF_FFFF, 0, true),
        ];
        let mut out = Vec::new();
        render_with(
            &m,
            0.0,
            glam::DVec3::ZERO,
            &atlas,
            7,
            &data,
            &Default::default(),
            &extra,
            &mut out,
        );
        assert_eq!(out.len(), 12, "body, armour, armour's glow");
        assert!(out[..8].iter().all(|v| v.light == 7));
        assert!(out[8..].iter().all(|v| v.light == FULL_BRIGHT));
    }
}

#[cfg(test)]
mod random_texture_tests {
    use super::*;
    use crate::mobs::{Polygon, Species};
    use horizon_optifine::random_textures::RandomTextures;
    use std::sync::{Arc, RwLock};

    /// A silverfish (drawn with its type's texture) whose model is one quad.
    fn silverfish() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.species = Species::Silverfish;
        k.texture = "entity/silverfish".into();
        k.model = Arc::new(Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: vec![Polygon {
                normal: [0.0, 0.0, -1.0],
                cull_back: false,
                vertices: [
                    [0.0, 0.0, 0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0, 1.0, 1.0],
                    [0.0, 1.0, 0.0, 0.0, 1.0],
                ],
            }],
            children: Vec::new(),
        });
        Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0)
    }

    /// A pack whose silverfish has a second variant, every silverfish wearing it.
    fn data() -> MobData {
        let root =
            std::env::temp_dir().join(format!("horizon-render-random-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("assets/minecraft/optifine/random/entity");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("silverfish.properties"), "textures.1=2").unwrap();
        std::fs::write(dir.join("silverfish2.png"), "").unwrap();
        let random = RandomTextures::scan(
            &horizon_assets::resource_source::ResourceSource::directory(root.to_owned()),
        );
        let _ = std::fs::remove_dir_all(&root);
        MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: RwLock::new(random),
            emissive: Default::default(),
            blinks: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
            mod_spawns: Default::default(),
            resolved: Default::default(),
        }
    }

    fn uvs(m: &Mob, data: &MobData, atlas: Atlas) -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        render_with(
            m,
            0.0,
            glam::DVec3::ZERO,
            atlas,
            0,
            data,
            &Default::default(),
            &[],
            &mut out,
        );
        out.iter().map(|v| v.uv).collect()
    }

    #[test]
    fn a_mob_draws_its_variant_and_the_texture_when_the_atlas_lacks_it() {
        let data = data();
        let mut m = silverfish();
        {
            let textures = data.random_textures.read().unwrap();
            let seen = crate::mobs::random_textures::Seen {
                biome: None,
                y: 64,
                blocks: Vec::new(),
                moon_phase: 0,
                day_time: 0,
                weather: horizon_optifine::random::Weather::Clear,
            };
            crate::mobs::random_textures::choose(
                &mut m,
                &textures,
                &data,
                &Default::default(),
                Some(seen),
                0,
                &|_| None,
            );
        }
        let both = |name: &str| match name {
            "entity/silverfish" => Some([0.0, 0.0, 0.5, 0.5]),
            "minecraft:optifine/random/entity/silverfish2" => Some([0.5, 0.5, 1.0, 1.0]),
            _ => None,
        };
        let drawn = uvs(&m, &data, &both);
        assert_eq!(drawn.len(), 4);
        assert!(drawn.iter().all(|uv| uv[0] >= 0.5 && uv[1] >= 0.5));
        let vanilla_only =
            |name: &str| (name == "entity/silverfish").then_some([0.0, 0.0, 0.5, 0.5]);
        let drawn = uvs(&m, &data, &vanilla_only);
        assert_eq!(drawn.len(), 4, "still drawn");
        assert!(drawn.iter().all(|uv| uv[0] <= 0.5 && uv[1] <= 0.5));
    }

    /// `EndermanRenderer.getRenderOffset` moves a creepy enderman by
    /// `0.02 * state.scale` blocks sideways: at `minecraft:scale` 2 the same
    /// gaussian offset doubles, and at the default it is the old 0.02.
    #[test]
    fn a_scaled_creepy_endermans_render_offset_scales() {
        use horizon_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:enderman".into();
        kind.species = Species::Enderman;
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        // Enderman.isCreepy: the angry flag that adds the offset.
        m.extra.flag = true;
        m.extra.model_scale = 1.0;
        let one = enderman_render_offset(&m, 0.5);
        assert!(one.length() > 0.0, "the offset is the renderer's gaussian");
        m.extra.model_scale = 2.0;
        let two = enderman_render_offset(&m, 0.5);
        assert!((two.x - 2.0 * one.x).abs() < 1.0e-12, "{two:?} vs {one:?}");
        assert!((two.z - 2.0 * one.z).abs() < 1.0e-12, "{two:?} vs {one:?}");
        assert_eq!(two.y, 0.0);
    }

    /// `LivingEntityRenderer.submit` translates a sleeper toward its pillow
    /// before `poseStack.scale(state.scale)`: the bed offset is not scaled and
    /// the pose after it is. The uniform scale after `setupRotations` instead
    /// would leave the pose's own translations unscaled; the one before the
    /// sleeping translation would scale the bed.
    #[test]
    fn a_scaled_sleepers_bed_offset_is_not_scaled() {
        use crate::mobs::Species;
        use horizon_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:villager".into();
        kind.species = Species::Villager;
        (kind.width, kind.height, kind.eye_height) = (0.6, 1.95, 1.62);
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        // Villager.sleeping: the bed faces north, so the head goes to +z.
        m.extra.ints[1] = 1;
        // The eye height of the standing pose, as `submit` extracts it.
        let bed = |m: &Mob| {
            let eye = m.awake_dims().2 as f32;
            glam::Vec3::new(0.0, 0.0, eye - 0.1)
        };
        let unscaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let unscaled_pose = glam::Mat4::from_translation(-bed(&m)) * unscaled;
        m.extra.model_scale = 2.0;
        let scaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let scaled_pose = glam::Mat4::from_translation(-bed(&m)) * scaled;
        let expected = glam::Mat4::from_scale(glam::Vec3::splat(2.0)) * unscaled_pose;
        let got = scaled_pose.to_cols_array();
        let want = expected.to_cols_array();
        for i in 0..16 {
            assert!(
                (got[i] - want[i]).abs() < 1.0e-4,
                "element {i}: {} != {}\nscaled: {got:?}\nunscaled x2: {want:?}",
                got[i],
                want[i]
            );
        }
    }

    /// A sleeping avatar's bed offset is `getEyeHeight(Pose.STANDING) - 0.1`,
    /// and the standing eye height carries `minecraft:scale`: at scale 2 the
    /// bed offset is 3.14, not the type's 1.52, and the pose after it doubles.
    #[test]
    fn a_scaled_sleeping_avatar_uses_its_scaled_eye_height() {
        use horizon_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:player".into();
        (kind.width, kind.height, kind.eye_height) = (0.6, 1.8, 1.62);
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.ints[1] = 1;
        let bed = |m: &Mob| {
            let eye = m.kind.eye_height as f32 * m.model_scale();
            glam::Vec3::new(0.0, 0.0, eye - 0.1)
        };
        let unscaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let unscaled_pose = glam::Mat4::from_translation(-bed(&m)) * unscaled;
        m.extra.model_scale = 2.0;
        let scaled = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        let scaled_pose = glam::Mat4::from_translation(-bed(&m)) * scaled;
        let expected = glam::Mat4::from_scale(glam::Vec3::splat(2.0)) * unscaled_pose;
        let got = scaled_pose.to_cols_array();
        let want = expected.to_cols_array();
        for i in 0..16 {
            assert!(
                (got[i] - want[i]).abs() < 1.0e-4,
                "element {i}: {} != {}\nscaled: {got:?}\nunscaled x2: {want:?}",
                got[i],
                want[i]
            );
        }
    }

    /// The avatar's 0.9375 `AvatarRenderer.scale` and the entity's own
    /// `minecraft:scale` are two factors: at scale 2 the model is 1.875 times
    /// the model's, not 3.75, so the submitted scale is not applied twice.
    #[test]
    fn an_avatar_at_scale_two_scales_its_model_once() {
        use horizon_core::geometry::Vec3;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:player".into();
        let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        m.extra.model_scale = 2.0;
        let transform = base_transform_at(&m, 0.5, glam::Mat4::IDENTITY);
        for axis in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z] {
            let length = transform.transform_vector3(axis).length();
            assert!(
                (length - 1.875).abs() < 1.0e-5,
                "{axis:?} scales by {length}, not 2 * 0.9375"
            );
        }
    }
}

#[cfg(test)]
mod texture_table_tests {
    use super::{VILLAGER_TYPES, body, layers};
    use crate::mobs::{Mob, Species};
    use std::sync::Arc;

    fn mob_of_species(species: Species) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.species = species;
        Mob::new(1, Arc::new(kind), horizon_core::geometry::Vec3::ZERO, 0.0)
    }

    #[test]
    fn rabbit_texture_table_matches_every_variant_toast_and_age() {
        let mut m = mob_of_species(Species::Rabbit);
        for variant in [0u8, 1, 2, 3, 4, 5, 6, 99, 255] {
            m.extra.variant = variant;
            let name = match variant {
                1 => "white",
                2 => "black",
                3 => "white_splotched",
                4 => "gold",
                5 => "salt",
                99 => "caerbannog",
                _ => "brown",
            };
            for baby in [false, true] {
                m.extra.age = if baby { -24_000 } else { 0 };
                assert_eq!(m.is_baby(), baby);
                let suffix = if baby { "_baby" } else { "" };
                let expected = format!("entity/rabbit/rabbit_{name}{suffix}");
                assert_eq!(
                    body(&m).1.as_ref(),
                    expected.as_str(),
                    "rabbit variant={variant} baby={baby}"
                );
            }
        }
        m.custom_name = Some("Toast".into());
        for variant in [0u8, 1, 99] {
            m.extra.variant = variant;
            for baby in [false, true] {
                m.extra.age = if baby { -24_000 } else { 0 };
                let suffix = if baby { "_baby" } else { "" };
                let expected = format!("entity/rabbit/rabbit_toast{suffix}");
                assert_eq!(
                    body(&m).1.as_ref(),
                    expected.as_str(),
                    "toast rabbit variant={variant} baby={baby}"
                );
            }
        }
    }

    #[test]
    fn fox_texture_table_matches_every_variant_sleep_and_age() {
        let mut m = mob_of_species(Species::Fox);
        for variant in [0u8, 1, 2, 255] {
            m.extra.variant = variant;
            let snow = if variant == 0 { "" } else { "_snow" };
            for sleeping in [false, true] {
                m.extra.ints[0] = if sleeping {
                    crate::mobs::fox::SLEEPING
                } else {
                    0
                };
                let sleep = if sleeping { "_sleep" } else { "" };
                for baby in [false, true] {
                    m.extra.age = if baby { -24_000 } else { 0 };
                    assert_eq!(m.is_baby(), baby);
                    let suffix = if baby { "_baby" } else { "" };
                    let expected = format!("entity/fox/fox{snow}{sleep}{suffix}");
                    assert_eq!(
                        body(&m).1.as_ref(),
                        expected.as_str(),
                        "fox variant={variant} sleeping={sleeping} baby={baby}"
                    );
                }
            }
        }
    }

    #[test]
    fn pig_cow_and_chicken_texture_tables_match_every_variant_and_age() {
        for (species, stem) in [
            (Species::Pig, "pig"),
            (Species::Cow, "cow"),
            (Species::Chicken, "chicken"),
        ] {
            let mut m = mob_of_species(species);
            for variant in [0u8, 1, 2, 3, 255] {
                m.extra.variant = variant;
                let climate = match variant {
                    1 => "warm",
                    2 => "cold",
                    _ => "temperate",
                };
                for baby in [false, true] {
                    m.extra.age = if baby { -24_000 } else { 0 };
                    assert_eq!(m.is_baby(), baby);
                    let suffix = if baby { "_baby" } else { "" };
                    let expected = format!("entity/{stem}/{stem}_{climate}{suffix}");
                    assert_eq!(
                        body(&m).1.as_ref(),
                        expected.as_str(),
                        "{stem} variant={variant} baby={baby}"
                    );
                }
            }
        }
    }

    #[test]
    fn axolotl_parrot_and_frog_texture_tables_match_every_variant_and_age() {
        const AXOLOTLS: [&str; 5] = ["lucy", "wild", "gold", "cyan", "blue"];
        let mut axolotl = mob_of_species(Species::Axolotl);
        for variant in (0u8..=6).chain([255]) {
            axolotl.extra.variant = variant;
            let name = AXOLOTLS[usize::from(variant).min(AXOLOTLS.len() - 1)];
            for baby in [false, true] {
                axolotl.extra.age = if baby { -24_000 } else { 0 };
                assert_eq!(axolotl.is_baby(), baby);
                let suffix = if baby { "_baby" } else { "" };
                let expected = format!("entity/axolotl/axolotl_{name}{suffix}");
                assert_eq!(
                    body(&axolotl).1.as_ref(),
                    expected.as_str(),
                    "axolotl variant={variant} baby={baby}"
                );
            }
        }

        const PARROTS: [&str; 5] = ["red_blue", "blue", "green", "yellow_blue", "grey"];
        let mut parrot = mob_of_species(Species::Parrot);
        for variant in (0u8..=6).chain([255]) {
            parrot.extra.variant = variant;
            let name = PARROTS[usize::from(variant).min(PARROTS.len() - 1)];
            let expected = format!("entity/parrot/parrot_{name}");
            assert_eq!(
                body(&parrot).1.as_ref(),
                expected.as_str(),
                "parrot variant={variant}"
            );
        }

        let mut frog = mob_of_species(Species::Frog);
        for variant in [0u8, 1, 2, 3, 255] {
            frog.extra.variant = variant;
            let name = match variant {
                1 => "warm",
                2 => "cold",
                _ => "temperate",
            };
            let expected = format!("entity/frog/frog_{name}");
            assert_eq!(
                body(&frog).1.as_ref(),
                expected.as_str(),
                "frog variant={variant}"
            );
        }
    }

    #[test]
    fn villager_layer_texture_tables_match_every_type_level_and_age() {
        const BIOMES: [&str; 7] = [
            "plains", "desert", "jungle", "savanna", "snow", "swamp", "taiga",
        ];
        const LEVELS: [&str; 5] = ["stone", "iron", "gold", "emerald", "diamond"];
        assert_eq!(VILLAGER_TYPES, BIOMES);
        for (species, dir) in [
            (Species::Villager, "villager"),
            (Species::ZombieVillager, "zombie_villager"),
        ] {
            let mut m = mob_of_species(species);
            for variant in 0u8..=14 {
                m.extra.variant = variant;
                let biome = BIOMES[usize::from(variant) % BIOMES.len()];
                for baby in [false, true] {
                    m.extra.age = if baby { -24_000 } else { 0 };
                    assert_eq!(m.is_baby(), baby);
                    let sub = if baby { "baby" } else { "type" };
                    let expected = format!("entity/{dir}/{sub}/{biome}");
                    let out = layers(&m, 0.0, &[None; 4]);
                    assert_eq!(
                        out[0].texture.as_ref(),
                        expected.as_str(),
                        "{dir} variant={variant} baby={baby}"
                    );
                }
            }
            m.extra.age = 0;
            m.extra.variant = 2;
            m.extra.profession =
                crate::mobs::villager::ProfessionState::from_id("minecraft:librarian");
            for level in [-1i32, 0, 1, 2, 3, 4, 5, 6, 99] {
                m.extra.villager_level = level;
                let tier = LEVELS[(level.clamp(1, 5) - 1) as usize];
                let expected = format!("entity/{dir}/profession_level/{tier}");
                let out = layers(&m, 0.0, &[None; 4]);
                assert_eq!(out.len(), 3, "{dir} level={level}");
                assert_eq!(
                    out[2].texture.as_ref(),
                    expected.as_str(),
                    "{dir} level={level}"
                );
            }
        }
    }

    #[test]
    fn companion_mob_texture_tables_match_expected_paths() {
        const DYE_COLORS: [&str; 16] = [
            "white",
            "orange",
            "magenta",
            "light_blue",
            "yellow",
            "lime",
            "pink",
            "gray",
            "light_gray",
            "cyan",
            "purple",
            "blue",
            "brown",
            "green",
            "red",
            "black",
        ];

        // Copper golem body & eyes.
        let mut golem = mob_of_species(Species::CopperGolem);
        for (variant, suffix) in [
            (0u8, ""),
            (1, "_exposed"),
            (2, "_weathered"),
            (3, "_oxidized"),
            (4, "_oxidized"),
            (255, "_oxidized"),
        ] {
            golem.extra.variant = variant;
            let expected_body = format!("entity/copper_golem/copper_golem{suffix}");
            let expected_eyes = format!("entity/copper_golem/copper_golem_eyes{suffix}");
            assert_eq!(
                crate::mobs::copper_golem::texture(&golem),
                expected_body.as_str()
            );
            assert_eq!(
                crate::mobs::copper_golem::eye_texture(&golem),
                expected_eyes.as_str()
            );
        }

        // Equine body & markings.
        const HORSE_COLORS: [&str; 7] = [
            "white",
            "creamy",
            "chestnut",
            "brown",
            "black",
            "gray",
            "darkbrown",
        ];
        const HORSE_MARKINGS: [&str; 5] = ["", "white", "whitefield", "whitedots", "blackdots"];
        assert_eq!(crate::mobs::equine::COLORS, HORSE_COLORS);
        assert_eq!(crate::mobs::equine::MARKINGS, HORSE_MARKINGS);
        let mut horse = mob_of_species(Species::Horse);
        for variant in (0u8..=8).chain([255]) {
            horse.extra.variant = variant;
            let color = HORSE_COLORS[usize::from(variant).min(HORSE_COLORS.len() - 1)];
            for baby in [false, true] {
                horse.extra.age = if baby { -24_000 } else { 0 };
                let suffix = if baby { "_baby" } else { "" };
                let expected = format!("entity/horse/horse_{color}{suffix}");
                assert_eq!(crate::mobs::equine::texture(&horse), expected.as_str());
            }
        }
        for variant2 in (0u8..=6).chain([255]) {
            horse.extra.variant2 = variant2;
            let mk = HORSE_MARKINGS[usize::from(variant2).min(HORSE_MARKINGS.len() - 1)];
            for baby in [false, true] {
                horse.extra.age = if baby { -24_000 } else { 0 };
                let suffix = if baby { "_baby" } else { "" };
                let expected =
                    (!mk.is_empty()).then(|| format!("entity/horse/horse_markings_{mk}{suffix}"));
                assert_eq!(crate::mobs::equine::markings(&horse), expected.as_deref());
            }
        }
        for (species, stem) in [
            (Species::Donkey, "entity/horse/donkey"),
            (Species::Mule, "entity/horse/mule"),
            (Species::SkeletonHorse, "entity/horse/horse_skeleton"),
            (Species::Camel, "entity/camel/camel"),
            (Species::ZombieHorse, "entity/horse/horse_zombie"),
        ] {
            let mut m = mob_of_species(species);
            for baby in [false, true] {
                m.extra.age = if baby { -24_000 } else { 0 };
                assert_eq!(m.is_baby(), baby);
                let suffix = if baby { "_baby" } else { "" };
                let expected = format!("{stem}{suffix}");
                assert_eq!(crate::mobs::equine::texture(&m), expected.as_str());
            }
        }
        assert_eq!(
            crate::mobs::equine::texture(&mob_of_species(Species::CamelHusk)),
            "entity/camel/camel_husk"
        );

        // Llama coat & decor.
        const LLAMA_COATS: [&str; 4] = ["creamy", "white", "brown", "gray"];
        assert_eq!(crate::mobs::llama::VARIANTS, LLAMA_COATS);
        let mut llama = mob_of_species(Species::Llama);
        for variant in (0u8..=5).chain([255]) {
            llama.extra.variant = variant;
            let coat = LLAMA_COATS[usize::from(variant).min(LLAMA_COATS.len() - 1)];
            for baby in [false, true] {
                llama.extra.age = if baby { -24_000 } else { 0 };
                let suffix = if baby { "_baby" } else { "" };
                let expected = format!("entity/llama/llama_{coat}{suffix}");
                assert_eq!(crate::mobs::llama::texture(&llama), expected.as_str());
            }
        }
        llama.extra.age = 0;
        for color in DYE_COLORS.iter().copied().chain(["custom"]) {
            llama
                .extra
                .set_body(Some(format!("minecraft:{color}_carpet").into()));
            let decor = crate::mobs::llama::decor(&llama).expect("carpet decor");
            let expected = format!("entity/equipment/llama_body/{color}");
            assert_eq!(decor.0, "llama#decor");
            assert_eq!(decor.1.as_ref(), expected.as_str());
        }
        llama.extra.age = -24_000;
        assert!(crate::mobs::llama::decor(&llama).is_none());
        let mut trader = mob_of_species(Species::TraderLlama);
        for (baby, model, tex) in [
            (
                false,
                "llama#decor",
                "entity/equipment/llama_body/trader_llama",
            ),
            (
                true,
                "llama_baby#decor",
                "entity/equipment/llama_body/trader_llama_baby",
            ),
        ] {
            trader.extra.age = if baby { -24_000 } else { 0 };
            let decor = crate::mobs::llama::decor(&trader).expect("trader decor");
            assert_eq!(decor.0, model);
            assert_eq!(decor.1.as_ref(), tex);
        }

        // Panda adult & baby genes.
        const PANDA_GENES: [&str; 7] = [
            "normal",
            "lazy",
            "worried",
            "playful",
            "brown",
            "weak",
            "aggressive",
        ];
        assert_eq!(crate::mobs::panda::GENES, PANDA_GENES);
        let mut panda = mob_of_species(Species::Panda);
        for gene in (0u8..=8).chain([255]) {
            panda.extra.variant = gene;
            panda.extra.variant2 = gene;
            let name = PANDA_GENES[usize::from(gene).min(PANDA_GENES.len() - 1)];
            for baby in [false, true] {
                panda.extra.age = if baby { -24_000 } else { 0 };
                assert_eq!(panda.is_baby(), baby);
                let expected = match (baby, name) {
                    (false, "normal") => "entity/panda/panda".to_string(),
                    (false, g) => format!("entity/panda/panda_{g}"),
                    (true, "normal") => "entity/panda/panda_baby".to_string(),
                    (true, g) => format!("entity/panda/{g}_panda_baby"),
                };
                assert_eq!(
                    crate::mobs::panda::texture(&panda),
                    expected.as_str(),
                    "panda gene={gene} baby={baby}"
                );
            }
        }

        // Shulker shell colors (0..=15 dyed, 16..=127 undyed, negative i8 -> white).
        let mut shulker = mob_of_species(Species::Shulker);
        for (i, color) in DYE_COLORS.iter().enumerate() {
            shulker.extra.color = i as u8;
            let expected = format!("entity/shulker/shulker_{color}");
            assert_eq!(crate::mobs::shulker::texture(&shulker), expected.as_str());
        }
        for undyed in [16u8, 17, 127] {
            shulker.extra.color = undyed;
            assert_eq!(
                crate::mobs::shulker::texture(&shulker),
                "entity/shulker/shulker"
            );
        }
        for neg in [128u8, 255] {
            shulker.extra.color = neg;
            assert_eq!(
                crate::mobs::shulker::texture(&shulker),
                "entity/shulker/shulker_white"
            );
        }

        // Happy ghast harnesses.
        for color in DYE_COLORS.iter().copied().chain(["custom"]) {
            let item = format!("minecraft:{color}_harness");
            let tex = crate::mobs::happy_ghast::harness_texture(&item).expect("harness");
            let expected = format!("entity/equipment/happy_ghast_body/{color}_harness");
            assert_eq!(tex.as_ref(), expected.as_str());
        }
        assert!(crate::mobs::happy_ghast::harness_texture("minecraft:saddle").is_none());

        // Nautilus body & armour.
        let mut nautilus = mob_of_species(Species::Nautilus);
        for (baby, expected) in [
            (false, "entity/nautilus/nautilus"),
            (true, "entity/nautilus/nautilus_baby"),
        ] {
            nautilus.extra.age = if baby { -24_000 } else { 0 };
            assert_eq!(crate::mobs::nautilus::texture(&nautilus), expected);
        }
        let mut zombie_nautilus = mob_of_species(Species::ZombieNautilus);
        for (variant, expected) in [
            (0u8, "entity/nautilus/zombie_nautilus"),
            (1, "entity/nautilus/zombie_nautilus_coral"),
            (2, "entity/nautilus/zombie_nautilus"),
        ] {
            zombie_nautilus.extra.variant = variant;
            assert_eq!(crate::mobs::nautilus::texture(&zombie_nautilus), expected);
        }
        for (material, tex_name) in [
            ("copper", "copper"),
            ("iron", "iron"),
            ("golden", "gold"),
            ("diamond", "diamond"),
            ("netherite", "netherite"),
            ("custom", "custom"),
        ] {
            let item = format!("minecraft:{material}_nautilus_armor");
            let tex = crate::mobs::nautilus::armor_texture(&item).expect("nautilus armor");
            let expected = format!("entity/equipment/nautilus_body/{tex_name}");
            assert_eq!(tex.as_ref(), expected.as_str());
        }
        assert!(crate::mobs::nautilus::armor_texture("minecraft:iron_horse_armor").is_none());
    }
}
