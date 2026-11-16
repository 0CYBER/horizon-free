//! The `MobEffects` and `Potions` registries (code in vanilla, ported as
//! tables): each effect's colour and category, each potion's effects, and the
//! potion colour mix. horizon-client re-exports them from `brewing`.
use std::{
    collections::HashMap,
    sync::{OnceLock, RwLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Beneficial,
    Harmful,
    Neutral,
}
/// A `MobEffect`: id (vanilla's without namespace, a mod's `<ns>:<path>`),
/// colour, category, instantaneous.
#[derive(Clone, Copy, Debug)]
pub struct EffectDef {
    pub id: &'static str,
    /// Position in the process-owned effect registry, resolved when an instance is created.
    pub index: usize,
    pub color: u32,
    pub category: Category,
    pub instant: bool,
    /// `IMobEffectExtension.getSortOrder`: where the effect sits in the HUD and inventory
    /// lists. The colour, unless a content mod declared an order.
    pub sort_order: i32,
}
/// The colours resource packs give effects (Polytone's `effect.<id>`), by the table's id.
static PACK_COLORS: RwLock<Option<HashMap<String, u32>>> = RwLock::new(None);
/// Takes the packs' effect colours (none: vanilla's).
pub fn set_pack_colors(colors: Vec<(String, u32)>) {
    let colors = (!colors.is_empty()).then(|| colors.into_iter().collect());
    *PACK_COLORS.write().unwrap_or_else(|e| e.into_inner()) = colors;
}
impl EffectDef {
    /// `MobEffect.getColor` as the packs show it: potions, tipped arrows and effect particles.
    pub fn shown_color(&self) -> u32 {
        let pack = PACK_COLORS.read().unwrap_or_else(|e| e.into_inner());
        pack.as_ref()
            .and_then(|colors| colors.get(self.id))
            .map_or(self.color, |c| c & 0xFF_FFFF)
    }
}
const fn fx(index: usize, id: &'static str, color: u32, category: Category) -> EffectDef {
    EffectDef {
        index,
        id,
        color,
        category,
        instant: false,
        sort_order: color as i32,
    }
}
use Category::{Beneficial as B, Harmful as H, Neutral as N};
/// `MobEffects` in registration order.
pub const EFFECTS: &[EffectDef] = &[
    fx(0, "speed", 3402751, B),
    fx(1, "slowness", 9154528, H),
    fx(2, "haste", 14270531, B),
    fx(3, "mining_fatigue", 4866583, H),
    fx(4, "strength", 16762624, B),
    EffectDef {
        instant: true,
        ..fx(5, "instant_health", 16262179, B)
    },
    EffectDef {
        instant: true,
        ..fx(6, "instant_damage", 11101546, H)
    },
    fx(7, "jump_boost", 16646020, B),
    fx(8, "nausea", 5578058, H),
    fx(9, "regeneration", 13458603, B),
    fx(10, "resistance", 9520880, B),
    fx(11, "fire_resistance", 16750848, B),
    fx(12, "water_breathing", 10017472, B),
    fx(13, "invisibility", 16185078, B),
    fx(14, "blindness", 2039587, H),
    fx(15, "night_vision", 12779366, B),
    fx(16, "hunger", 5797459, H),
    fx(17, "weakness", 4738376, H),
    fx(18, "poison", 8889187, H),
    fx(19, "wither", 7561558, H),
    fx(20, "health_boost", 16284963, B),
    fx(21, "absorption", 2445989, B),
    EffectDef {
        instant: true,
        ..fx(22, "saturation", 16262179, B)
    },
    fx(23, "glowing", 9740385, N),
    fx(24, "levitation", 13565951, H),
    fx(25, "luck", 5882118, B),
    fx(26, "unluck", 12624973, H),
    fx(27, "slow_falling", 15978425, B),
    fx(28, "conduit_power", 1950417, B),
    fx(29, "dolphins_grace", 8954814, B),
    fx(30, "bad_omen", 745784, N),
    fx(31, "hero_of_the_village", 4521796, B),
    fx(32, "darkness", 2696993, H),
    fx(33, "trial_omen", 1484454, N),
    fx(34, "raid_omen", 14565464, N),
    fx(35, "wind_charged", 12438015, H),
    fx(36, "weaving", 7891290, H),
    fx(37, "oozing", 10092451, H),
    fx(38, "infested", 9214860, H),
    fx(39, "breath_of_the_nautilus", 65518, B),
];
/// The effects content mods declare, registered after vanilla's.
static MOD_EFFECTS: OnceLock<Vec<EffectDef>> = OnceLock::new();
/// Registers the content mods' effects, `<ns>:<path>` ids in registry order.
/// The list is set once per process: false when it already was, and the
/// effects stay those of the first call until the game restarts.
/// Each comes with its declared sort order, none for its colour.
pub fn register_mod_effects(
    effects: impl IntoIterator<Item = (String, u32, Category, Option<i32>)>,
) -> bool {
    if MOD_EFFECTS.get().is_some() {
        return false;
    }
    let defs: Vec<EffectDef> = effects
        .into_iter()
        .enumerate()
        .map(|(index, (id, color, category, sort_order))| {
            debug_assert!(id.contains(':'), "{id} has no namespace");
            let def = fx(
                EFFECTS.len() + index,
                Box::leak(id.into_boxed_str()),
                color,
                category,
            );
            EffectDef {
                sort_order: sort_order.unwrap_or(def.sort_order),
                ..def
            }
        })
        .collect();
    MOD_EFFECTS.set(defs).is_ok()
}
/// The content mods' effects, in registry order.
pub fn mod_effects() -> &'static [EffectDef] {
    MOD_EFFECTS.get().map_or(&[], Vec::as_slice)
}
/// Every registered effect: vanilla's, then the content mods'.
pub fn all_effects() -> impl Iterator<Item = &'static EffectDef> {
    EFFECTS.iter().chain(mod_effects())
}
pub fn effect(id: &str) -> Option<&'static EffectDef> {
    let short = id.trim_start_matches("minecraft:");
    match EFFECTS.iter().find(|e| e.id == short) {
        Some(e) => Some(e),
        None => mod_effect(id),
    }
}
/// A content mod's effect id (`<ns>:<path>`), not vanilla's with or
/// without `minecraft:`.
#[inline]
pub fn is_mod_effect(id: &str) -> bool {
    !id.starts_with("minecraft:") && id.bytes().any(|b| b == b':')
}
/// Out of line, so vanilla's lookups (every frame, every tick) stay as they were.
#[cold]
#[inline(never)]
fn mod_effect(id: &str) -> Option<&'static EffectDef> {
    mod_effects().iter().find(|e| e.id == id)
}
/// `MobEffect.getDescriptionId` (`Util.makeDescriptionId("effect", key)`):
/// `effect.<namespace>.<path>`, `/` in the path as `.`.
pub fn description_id(id: &str) -> String {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("effect.{namespace}.{}", path.replace('/', "."))
}
/// The effect's sprite in the GUI atlas (`Hud.getMobEffectSprite`: the key
/// with `mob_effect/` before its path): `mob_effect/<path>` for vanilla's,
/// `<ns>:mob_effect/<path>` for a mod's.
pub fn sprite(id: &str) -> String {
    let mut out = String::new();
    push_sprite(&mut out, id);
    out
}
/// [`sprite`], appended to `out`.
pub fn push_sprite(out: &mut String, id: &str) {
    let path = match id.split_once(':') {
        None => id,
        Some(("minecraft", path)) => path,
        Some((namespace, path)) => {
            out.push_str(namespace);
            out.push(':');
            path
        }
    };
    out.push_str("mob_effect/");
    out.push_str(path);
}
/// Namespaced effect ids for mob effect storage; a mod's id already is.
pub fn namespaced(id: &'static str) -> &'static str {
    static IDS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    let key = id.strip_prefix("minecraft:").unwrap_or(id);
    IDS.get_or_init(|| {
        EFFECTS
            .iter()
            .map(|e| {
                (
                    e.id,
                    &*Box::leak(format!("minecraft:{}", e.id).into_boxed_str()),
                )
            })
            .collect()
    })
    .get(key)
    .copied()
    .unwrap_or(id)
}
/// A `Potion`: id, name suffix (`item.minecraft.potion.effect.<name>`) and
/// effects as (effect, duration ticks, amplifier).
#[derive(Clone, Copy, Debug)]
pub struct PotionDef {
    pub id: &'static str,
    pub name: &'static str,
    pub effects: &'static [(&'static str, i32, i32)],
}
const fn p(
    id: &'static str,
    name: &'static str,
    effects: &'static [(&'static str, i32, i32)],
) -> PotionDef {
    PotionDef { id, name, effects }
}
/// `Potions` in registration order.
pub const POTIONS: &[PotionDef] = &[
    p("water", "water", &[]),
    p("mundane", "mundane", &[]),
    p("thick", "thick", &[]),
    p("awkward", "awkward", &[]),
    p("night_vision", "night_vision", &[("night_vision", 3600, 0)]),
    p(
        "long_night_vision",
        "night_vision",
        &[("night_vision", 9600, 0)],
    ),
    p("invisibility", "invisibility", &[("invisibility", 3600, 0)]),
    p(
        "long_invisibility",
        "invisibility",
        &[("invisibility", 9600, 0)],
    ),
    p("leaping", "leaping", &[("jump_boost", 3600, 0)]),
    p("long_leaping", "leaping", &[("jump_boost", 9600, 0)]),
    p("strong_leaping", "leaping", &[("jump_boost", 1800, 1)]),
    p(
        "fire_resistance",
        "fire_resistance",
        &[("fire_resistance", 3600, 0)],
    ),
    p(
        "long_fire_resistance",
        "fire_resistance",
        &[("fire_resistance", 9600, 0)],
    ),
    p("swiftness", "swiftness", &[("speed", 3600, 0)]),
    p("long_swiftness", "swiftness", &[("speed", 9600, 0)]),
    p("strong_swiftness", "swiftness", &[("speed", 1800, 1)]),
    p("slowness", "slowness", &[("slowness", 1800, 0)]),
    p("long_slowness", "slowness", &[("slowness", 4800, 0)]),
    p("strong_slowness", "slowness", &[("slowness", 400, 3)]),
    p(
        "turtle_master",
        "turtle_master",
        &[("slowness", 400, 3), ("resistance", 400, 2)],
    ),
    p(
        "long_turtle_master",
        "turtle_master",
        &[("slowness", 800, 3), ("resistance", 800, 2)],
    ),
    p(
        "strong_turtle_master",
        "turtle_master",
        &[("slowness", 400, 5), ("resistance", 400, 3)],
    ),
    p(
        "water_breathing",
        "water_breathing",
        &[("water_breathing", 3600, 0)],
    ),
    p(
        "long_water_breathing",
        "water_breathing",
        &[("water_breathing", 9600, 0)],
    ),
    p("healing", "healing", &[("instant_health", 1, 0)]),
    p("strong_healing", "healing", &[("instant_health", 1, 1)]),
    p("harming", "harming", &[("instant_damage", 1, 0)]),
    p("strong_harming", "harming", &[("instant_damage", 1, 1)]),
    p("poison", "poison", &[("poison", 900, 0)]),
    p("long_poison", "poison", &[("poison", 1800, 0)]),
    p("strong_poison", "poison", &[("poison", 432, 1)]),
    p("regeneration", "regeneration", &[("regeneration", 900, 0)]),
    p(
        "long_regeneration",
        "regeneration",
        &[("regeneration", 1800, 0)],
    ),
    p(
        "strong_regeneration",
        "regeneration",
        &[("regeneration", 450, 1)],
    ),
    p("strength", "strength", &[("strength", 3600, 0)]),
    p("long_strength", "strength", &[("strength", 9600, 0)]),
    p("strong_strength", "strength", &[("strength", 1800, 1)]),
    p("weakness", "weakness", &[("weakness", 1800, 0)]),
    p("long_weakness", "weakness", &[("weakness", 4800, 0)]),
    p("luck", "luck", &[("luck", 6000, 0)]),
    p("slow_falling", "slow_falling", &[("slow_falling", 1800, 0)]),
    p(
        "long_slow_falling",
        "slow_falling",
        &[("slow_falling", 4800, 0)],
    ),
    p("wind_charged", "wind_charged", &[("wind_charged", 3600, 0)]),
    p("weaving", "weaving", &[("weaving", 3600, 0)]),
    p("oozing", "oozing", &[("oozing", 3600, 0)]),
    p("infested", "infested", &[("infested", 3600, 0)]),
];
pub fn potion(id: &str) -> Option<&'static PotionDef> {
    let id = id.trim_start_matches("minecraft:");
    POTIONS.iter().find(|p| p.id == id)
}
/// `PotionContents.BASE_POTION_COLOR` (ARGB -13083194) as RGB.
pub const BASE_POTION_COLOR: u32 = 0x385DC6;
/// `PotionContents.getColor`: amplifier-weighted mix of visible effect colours.
pub fn potion_color(id: Option<&str>) -> u32 {
    let Some(def) = id.and_then(potion) else {
        return BASE_POTION_COLOR;
    };
    effects_color(def.effects.iter().map(|(id, _, amp)| (*id, *amp)))
}
/// `PotionContents.getColorOptional` of some effects, `(id, amplifier)`, all visible: the mix of
/// their colours weighted by amplifier + 1, else the base potion colour.
pub fn effects_color<'a>(effects: impl Iterator<Item = (&'a str, i32)>) -> u32 {
    let (mut r, mut g, mut b, mut total) = (0u32, 0u32, 0u32, 0u32);
    for (e, amp) in effects {
        let Some(e) = effect(e) else { continue };
        let w = amp as u32 + 1;
        let color = e.shown_color();
        r += w * (color >> 16 & 255);
        g += w * (color >> 8 & 255);
        b += w * (color & 255);
        total += w;
    }
    match (
        r.checked_div(total),
        g.checked_div(total),
        b.checked_div(total),
    ) {
        (Some(r), Some(g), Some(b)) => (r << 16) | (g << 8) | b,
        _ => BASE_POTION_COLOR,
    }
}
/// Borrow an effect by its completed engine registry index.
pub fn effect_by_index(index: usize) -> Option<&'static EffectDef> {
    if index < EFFECTS.len() {
        EFFECTS.get(index)
    } else {
        mod_effects().get(index - EFFECTS.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_ids_and_sprites_keep_the_namespace() {
        assert_eq!(description_id("speed"), "effect.minecraft.speed");
        assert_eq!(description_id("minecraft:speed"), "effect.minecraft.speed");
        assert_eq!(description_id("ruby:curse/dark"), "effect.ruby.curse.dark");
        assert_eq!(sprite("speed"), "mob_effect/speed");
        assert_eq!(sprite("minecraft:speed"), "mob_effect/speed");
        assert_eq!(sprite("ruby:curse/dark"), "ruby:mob_effect/curse/dark");
    }

    /// `PotionContents.getColorOptional`: each visible effect's colour weighted by its
    /// amplifier + 1, the channels averaged by integer division; none gives the base colour.
    #[test]
    fn effects_mix_their_colours_by_amplifier() {
        let c = |id: &str| effect(id).unwrap().color;
        assert_eq!(effects_color([("poison", 0)].into_iter()), c("poison"));
        let (p, s) = (c("poison"), c("speed"));
        let mix = |shift: u32| (((p >> shift & 255) + 3 * (s >> shift & 255)) / 4) << shift;
        assert_eq!(
            effects_color([("poison", 0), ("speed", 2)].into_iter()),
            mix(16) | mix(8) | mix(0)
        );
        assert_eq!(effects_color(std::iter::empty()), BASE_POTION_COLOR);
    }

    /// The only test that registers: the registry is set once per process.
    #[test]
    fn mod_effects_follow_vanilla_once_registered() {
        assert!(register_mod_effects([
            ("ruby:glow".to_owned(), 0xFF0000, Category::Neutral, None),
            (
                "ruby:first".to_owned(),
                0x0000FF,
                Category::Neutral,
                Some(-5)
            ),
        ]));
        assert!(!register_mod_effects([(
            "ruby:late".to_owned(),
            1,
            Category::Harmful,
            None
        )]));
        let glow = effect("ruby:glow").unwrap();
        assert_eq!(
            (glow.color, glow.category, glow.instant),
            (0xFF0000, Category::Neutral, false)
        );
        // The HUD and inventory sort by this: the colour unless the mod declared an order.
        assert_eq!(glow.sort_order, 0xFF0000);
        assert_eq!(effect("ruby:first").unwrap().sort_order, -5);
        assert_eq!(effect("speed").unwrap().sort_order, 3402751);
        assert!(effect("glow").is_none() && effect("ruby:late").is_none());
        assert!(is_mod_effect(glow.id) && !is_mod_effect("speed"));
        assert!(!is_mod_effect(namespaced("speed")));
        assert_eq!(namespaced(glow.id), "ruby:glow");
        assert_eq!(effect("minecraft:speed").unwrap().id, "speed");
        let ids: Vec<_> = all_effects().map(|e| e.id).collect();
        assert_eq!(ids.len(), EFFECTS.len() + 2);
        assert_eq!(ids.last(), Some(&"ruby:first"));
    }
}
