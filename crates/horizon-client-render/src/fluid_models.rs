//! What each mod's world fluid draws with in the world: the sprite and tint its look declares
//! (`data/<id>/horizon/fluid/`), completed by what mods register (`register_fluid_models`).
//! There is no second appearance store: a model only names sprites a look can't, and every
//! resource load reads this again, so a reload draws the new pack's textures.
//!
//! An item's fluid layer (`neoforge:fluid_container`) reads the same data: [`item_appearance`]
//! gives a fluid's still sprite and item tint, and [`held_fluid`] the fluid a vanilla bucket holds.
//! Items are drawn every frame, so their answers are resolved once per load, by the same two
//! calls that store the looks and models ([`FluidData`]).
use horizon_assets::item_model::FluidAppearance;
use horizon_mod_package::content::{FluidLook, FluidModel, namespaced_id};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

static DATA: LazyLock<RwLock<FluidData>> = LazyLock::new(|| RwLock::new(FluidData::new()));

/// The world fluids' looks, the models mods register, and what every fluid draws as an item,
/// resolved from both. Only [`FluidData::set_looks`] and [`FluidData::set_registered`] change
/// it, the two steps of every resource load, and each rebuilds `items`: the index is never
/// older than the data it was built from, and needs no invalidation of its own.
struct FluidData {
    /// The world fluids' looks: the only ones the world renderer and the atlas resolve.
    looks: Vec<FluidLook>,
    /// The looks of fluids only a tank holds (`world: None`): no block, no flowing form and
    /// no world model, but an item can still hold one and draw its sprite.
    tank_looks: Vec<FluidLook>,
    registered: Vec<FluidModel>,
    /// [`item_appearance`]'s answers by fluid id, vanilla's included.
    items: HashMap<String, FluidAppearance>,
}

impl FluidData {
    fn new() -> Self {
        let mut data = Self {
            looks: Vec::new(),
            tank_looks: Vec::new(),
            registered: Vec::new(),
            items: HashMap::new(),
        };
        data.reindex();
        data
    }
    fn set_looks<'a>(&mut self, looks: impl IntoIterator<Item = &'a FluidLook>) {
        (self.looks, self.tank_looks) = looks
            .into_iter()
            .cloned()
            .partition(|look| look.world.is_some());
        self.reindex();
    }
    fn set_registered(&mut self, models: Vec<FluidModel>) {
        self.registered = models;
        self.reindex();
    }
    /// Resolves every fluid's item appearance (see [`item_appearance`]). A mod's world fluid
    /// draws its world model's still sprite with its fixed tint made opaque, or water's item
    /// colour when it takes the biome's. Vanilla's fluids come from the world renderer's own
    /// table and win over a look of the same id. `FluidStateModelSet.bake` maps a fluid and
    /// its flowing form to one model, so both ids answer alike, a mod's as vanilla's.
    fn reindex(&mut self) {
        let mut items: HashMap<String, FluidAppearance> = self
            .looks
            .iter()
            .flat_map(|look| {
                // The load's `resolved` already warned about bad sprite ids.
                let model = resolve_look(look, &self.registered, false);
                let tint = match model.tint {
                    FluidTint::Fixed(rgb) => (0xFF00_0000 | rgb) as i32,
                    FluidTint::Biome => WATER_ITEM_TINT,
                };
                let appearance = FluidAppearance {
                    still: model.still,
                    tint,
                };
                [
                    (look.flowing_id(), appearance.clone()),
                    (look.id.clone(), appearance),
                ]
            })
            .collect();
        // A tank-only fluid has no registered model (a model naming one is reported as naming
        // no world fluid), so its look alone decides, by the same rule as a world fluid's.
        for look in &self.tank_looks {
            let model = resolve_look(look, &[], false);
            let tint = match model.tint {
                FluidTint::Fixed(rgb) => (0xFF00_0000 | rgb) as i32,
                FluidTint::Biome => WATER_ITEM_TINT,
            };
            let appearance = FluidAppearance {
                still: model.still,
                tint,
            };
            items.entry(look.id.clone()).or_insert(appearance);
        }
        for (fluid, block) in [
            ("minecraft:water", "minecraft:water"),
            ("minecraft:flowing_water", "minecraft:water"),
            ("minecraft:lava", "minecraft:lava"),
            ("minecraft:flowing_lava", "minecraft:lava"),
        ] {
            if let Some((still, tint, _)) = crate::block_sprites::fluid_faces(block) {
                let appearance = FluidAppearance {
                    still: format!("minecraft:{still}"),
                    // Lava's model has no tint source: `FluidContentsTint` answers -1.
                    tint: if matches!(tint, Some(crate::mesher::TintSource::Water)) {
                        WATER_ITEM_TINT
                    } else {
                        -1
                    },
                };
                items.insert(fluid.to_owned(), appearance);
            }
        }
        self.items = items;
    }
}

fn data() -> std::sync::RwLockReadGuard<'static, FluidData> {
    DATA.read().unwrap_or_else(|e| e.into_inner())
}

/// The colour a mod fluid's sprites are multiplied by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidTint {
    /// The biome's water colour: a fluid that declares neither sprite nor tint looks like water.
    Biome,
    /// `0xRRGGBB`; a look's alpha is not drawn in the world.
    Fixed(u32),
}

/// A mod fluid's model, with every sprite named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedFluidModel {
    pub fluid: String,
    pub still: String,
    pub flowing: String,
    /// What it draws on sides against glass and leaves, as water's overlay.
    pub overlay: Option<String>,
    pub tint: FluidTint,
}

/// Remembers the mods' fluid looks; every resource load calls this before it reads them. The
/// world resolves only the world fluids' ([`resolved`]); an item's fluid layer reads all of them.
pub fn set_world_looks<'a>(looks: impl IntoIterator<Item = &'a FluidLook>) {
    DATA.write()
        .unwrap_or_else(|e| e.into_inner())
        .set_looks(looks);
}

/// Replaces the models recollected before each resource bake, including reloads.
pub fn register(models: Vec<FluidModel>) {
    DATA.write()
        .unwrap_or_else(|e| e.into_inner())
        .set_registered(models);
}

/// Every world fluid's model.
pub fn resolved() -> Vec<ResolvedFluidModel> {
    let data = data();
    resolve(&data.looks, &data.registered)
}

fn resolve(looks: &[FluidLook], registered: &[FluidModel]) -> Vec<ResolvedFluidModel> {
    for model in registered {
        if !looks.iter().any(|look| look.id == model.fluid) {
            log::warn!(
                "Horizon: a fluid model names {}, which is no world fluid",
                model.fluid
            );
        }
    }
    looks
        .iter()
        .map(|look| resolve_look(look, registered, true))
        .collect()
}

/// One world fluid's model. `warn` logs a bad sprite id; the resource load does, while an item
/// drawn every frame does not repeat it.
fn resolve_look(look: &FluidLook, registered: &[FluidModel], warn: bool) -> ResolvedFluidModel {
    let model = registered.iter().find(|model| model.fluid == look.id);
    let sprite = |name: Option<&String>| {
        name.filter(|name| {
            let valid = namespaced_id(name);
            if !valid && warn {
                log::warn!("Horizon: fluid {}: bad sprite id {name:?}", look.id);
            }
            valid
        })
        .cloned()
    };
    let overlay = sprite(model.and_then(|model| model.overlay.as_ref()));
    let flowing = sprite(model.and_then(|model| model.flowing.as_ref()));
    let still = sprite(model.and_then(|model| model.still.as_ref()))
        .or_else(|| sprite(look.sprite.as_ref()));
    let fixed = look.tint.map(|tint| FluidTint::Fixed(tint & 0xFF_FFFF));
    match still {
        Some(still) => ResolvedFluidModel {
            fluid: look.id.clone(),
            flowing: flowing.unwrap_or_else(|| still.clone()),
            still,
            overlay,
            tint: fixed.unwrap_or(FluidTint::Fixed(0xFF_FFFF)),
        },
        None => ResolvedFluidModel {
            fluid: look.id.clone(),
            still: "minecraft:block/water_still".into(),
            flowing: flowing.unwrap_or_else(|| "minecraft:block/water_flow".into()),
            overlay: overlay.or_else(|| Some("minecraft:block/water_overlay".into())),
            tint: fixed.unwrap_or(FluidTint::Biome),
        },
    }
}

/// Water's colour as an item: NeoForge's `FluidTintSources.water()`, whose `color(FluidState)`
/// is the constant `-12618012` (`FluidTintSources$3`), the default biome water colour opaque.
/// `FluidContentsTint` reaches it through `colorAsStack`, so an item's water has no biome.
pub const WATER_ITEM_TINT: i32 = 0xFF3F_76E4_u32 as i32;

/// What `fluid` draws with as an item's fluid layer: `FluidStateModelSet.get(fluid)`'s still
/// sprite (`DynamicFluidContainerModel.bakeModelForFluid`) and its tint as `FluidContentsTint`
/// gives it (`fluidTintSource().colorAsStack(stack)`, white with no tint source).
///
/// Vanilla's fluids come from the world renderer's own table ([`crate::block_sprites::fluid_faces`]),
/// a mod's fluid from its world model, or its look alone when only a tank holds it (see
/// [`FluidData::reindex`]). `None` for `minecraft:empty` and every fluid no model knows: the
/// item resolver then draws the missing model's sprite (`FluidStateModelSet.get`'s
/// `missingModel`), or nothing for empty.
///
/// One read lock and one hash lookup: the answers are resolved when the looks or models load.
/// A resource pack that replaces a sprite's texture keeps its name, so the answer stays right
/// and the bake reads the new pixels from the atlas.
pub fn item_appearance(fluid: &str) -> Option<FluidAppearance> {
    data().items.get(fluid).cloned()
}

/// The fluid a vanilla bucket holds: NeoForge registers `BucketResourceHandler` for every item
/// whose class is exactly `BucketItem` (`CapabilityHooks.registerFallbackVanillaProviders`), and
/// its `getResourceFrom` answers `BucketItem.content`. The empty bucket's content is
/// `Fluids.EMPTY`, which `FluidUtil.getFirstStackContained` reads as holding none. Mob buckets
/// are subclasses and get no handler; the milk bucket holds NeoForge's milk only when a mod
/// enables it, which Horizon has no fluid for.
pub fn held_fluid(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:water_bucket" => Some("minecraft:water"),
        "minecraft:lava_bucket" => Some("minecraft:lava"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_mod_package::content::WorldFluid;

    fn look(id: &str, sprite: Option<&str>, tint: Option<u32>) -> FluidLook {
        FluidLook {
            id: id.into(),
            sprite: sprite.map(Into::into),
            tint,
            world: Some(WorldFluid::default()),
        }
    }
    fn model(fluid: &str) -> FluidModel {
        FluidModel {
            fluid: fluid.into(),
            ..FluidModel::default()
        }
    }

    #[test]
    fn a_look_alone_draws_its_sprite_and_tint_on_every_surface() {
        let drawn = resolve(
            &[look("a:oil", Some("a:block/oil"), Some(0xFF11_2233))],
            &[],
        );
        assert_eq!(
            drawn,
            [ResolvedFluidModel {
                fluid: "a:oil".into(),
                still: "a:block/oil".into(),
                flowing: "a:block/oil".into(),
                overlay: None,
                tint: FluidTint::Fixed(0x11_2233),
            }]
        );
    }

    #[test]
    fn a_sprite_without_a_tint_is_drawn_as_it_is() {
        let drawn = resolve(&[look("a:oil", Some("a:block/oil"), None)], &[]);
        assert_eq!(drawn[0].tint, FluidTint::Fixed(0xFF_FFFF));
    }

    #[test]
    fn no_sprite_draws_waters_textures_with_the_look_tint_or_the_biomes() {
        let plain = &resolve(&[look("a:plain", None, None)], &[])[0];
        assert_eq!(plain.still, "minecraft:block/water_still");
        assert_eq!(plain.flowing, "minecraft:block/water_flow");
        assert_eq!(plain.tint, FluidTint::Biome);
        let tinted = &resolve(&[look("a:tinted", None, Some(0xFF00_0010))], &[])[0];
        assert_eq!(tinted.still, "minecraft:block/water_still");
        assert_eq!(tinted.tint, FluidTint::Fixed(0x10));
    }

    #[test]
    fn a_model_names_what_the_look_cannot_and_keeps_the_looks_tint() {
        let drawn = resolve(
            &[
                look("a:oil", Some("a:block/oil"), Some(0xFF11_2233)),
                look("a:other", Some("a:block/other"), None),
            ],
            &[FluidModel {
                flowing: Some("a:block/oil_flow".into()),
                overlay: Some("a:block/oil_overlay".into()),
                ..model("a:oil")
            }],
        );
        assert_eq!(drawn[0].still, "a:block/oil");
        assert_eq!(drawn[0].flowing, "a:block/oil_flow");
        assert_eq!(drawn[0].overlay.as_deref(), Some("a:block/oil_overlay"));
        assert_eq!(drawn[0].tint, FluidTint::Fixed(0x11_2233));
        assert_eq!(drawn[1].flowing, "a:block/other", "its model is its own");
    }

    /// `FluidStateModelSet`'s water and lava models, as `DynamicFluidContainerModel` reads
    /// them: `block/water_still` with `FluidTintSources.water()`'s item colour (the constant
    /// `-12618012` in `FluidTintSources$3.color(FluidState)`), and `block/lava_still` with no
    /// tint source, which `FluidContentsTint` answers with -1. A flowing fluid shares its
    /// source's model; the empty fluid and unknown fluids have none.
    #[test]
    fn vanilla_fluids_draw_their_still_sprite_with_their_item_tint() {
        let data = FluidData::new();
        let appearance = |fluid| data.items.get(fluid).cloned();
        let water = FluidAppearance {
            still: "minecraft:block/water_still".into(),
            tint: -12618012,
        };
        assert_eq!(appearance("minecraft:water"), Some(water.clone()));
        assert_eq!(appearance("minecraft:flowing_water"), Some(water));
        let lava = FluidAppearance {
            still: "minecraft:block/lava_still".into(),
            tint: -1,
        };
        assert_eq!(appearance("minecraft:lava"), Some(lava.clone()));
        assert_eq!(appearance("minecraft:flowing_lava"), Some(lava));
        assert_eq!(appearance("minecraft:empty"), None);
        assert_eq!(
            appearance("minecraft:bubble_column"),
            None,
            "a block, not a fluid"
        );
        assert_eq!(appearance("a:unknown"), None);
    }

    /// A mod's world fluid draws in an item what it draws in the world: its model's still
    /// sprite, its fixed tint made opaque, or water's sprite and item colour when it declares
    /// neither.
    #[test]
    fn a_mod_fluid_draws_its_world_still_sprite_and_tint() {
        let looks = [
            look("a:oil", Some("a:block/oil"), Some(0x8011_2233)),
            look("a:plain", None, None),
        ];
        let mut data = FluidData::new();
        data.set_looks(&looks);
        data.set_registered(vec![FluidModel {
            still: Some("a:block/oil_still".into()),
            ..model("a:oil")
        }]);
        let appearance = |fluid| data.items.get(fluid).cloned();
        assert_eq!(
            appearance("a:oil"),
            Some(FluidAppearance {
                still: "a:block/oil_still".into(),
                tint: 0xFF11_2233_u32 as i32,
            })
        );
        assert_eq!(
            appearance("a:plain"),
            Some(FluidAppearance {
                still: "minecraft:block/water_still".into(),
                tint: -12618012,
            })
        );
    }

    /// The item index follows both steps of a resource load: a reload that changes a look,
    /// a client mod's model naming another sprite (`register_fluid_models`, recollected
    /// before each bake), and a reload that drops the fluid each change what the item draws.
    #[test]
    fn a_reload_changes_what_an_items_fluid_draws() {
        let mut data = FluidData::new();
        let appearance = |data: &FluidData| data.items.get("a:oil").cloned();
        assert_eq!(appearance(&data), None, "not loaded yet");
        data.set_looks(&[look("a:oil", Some("a:block/oil"), Some(0x11_2233))]);
        assert_eq!(
            appearance(&data),
            Some(FluidAppearance {
                still: "a:block/oil".into(),
                tint: 0xFF11_2233_u32 as i32,
            })
        );
        data.set_looks(&[look("a:oil", Some("a:block/tar"), None)]);
        assert_eq!(
            appearance(&data),
            Some(FluidAppearance {
                still: "a:block/tar".into(),
                tint: -1,
            })
        );
        data.set_registered(vec![FluidModel {
            still: Some("a:block/tar_still".into()),
            ..model("a:oil")
        }]);
        assert_eq!(
            appearance(&data).map(|a| a.still),
            Some("a:block/tar_still".into())
        );
        data.set_looks(&[]);
        assert_eq!(appearance(&data), None);
        assert_eq!(
            data.items.get("minecraft:water").map(|a| a.tint),
            Some(-12618012),
            "vanilla's stay through every reload"
        );
    }

    /// A container that declares a mod fluid's flowing form draws what its source draws:
    /// `FluidStateModelSet.bake` maps both ids to one model. The game's `ItemEnv` passes the
    /// declared id straight to [`item_appearance`], so the index answers for both.
    #[test]
    fn a_mod_fluids_flowing_form_draws_its_source_appearance() {
        let mut data = FluidData::new();
        data.set_looks(&[look("a:oil", Some("a:block/oil"), Some(0x11_2233))]);
        let oil = Some(FluidAppearance {
            still: "a:block/oil".into(),
            tint: 0xFF11_2233_u32 as i32,
        });
        assert_eq!(data.items.get("a:flowing_oil").cloned(), oil);
        assert_eq!(data.items.get("a:oil").cloned(), oil);
    }

    /// A fluid only a tank holds (`world: None`) has no world model, but an item holding or
    /// declaring it draws its look's sprite and tint; the world still resolves world fluids
    /// only, and a tank-only fluid has no flowing form.
    #[test]
    fn a_tank_only_fluid_draws_its_look_in_an_item_but_not_in_the_world() {
        let mut data = FluidData::new();
        let goo = FluidLook {
            world: None,
            ..look("a:goo", Some("a:block/goo"), Some(0x8044_5566))
        };
        data.set_looks(&[look("a:oil", Some("a:block/oil"), None), goo]);
        assert_eq!(
            data.items.get("a:goo").cloned(),
            Some(FluidAppearance {
                still: "a:block/goo".into(),
                tint: 0xFF44_5566_u32 as i32,
            })
        );
        assert_eq!(data.items.get("a:flowing_goo"), None);
        let world: Vec<String> = resolve(&data.looks, &data.registered)
            .into_iter()
            .map(|model| model.fluid)
            .collect();
        assert_eq!(world, ["a:oil"]);
    }

    /// `BucketResourceHandler` for the items whose class is exactly `BucketItem`: the water
    /// and lava buckets hold their fluid, the empty bucket (`Fluids.EMPTY`) holds none, and a
    /// mob bucket (a `MobBucketItem`) has no handler at all.
    #[test]
    fn vanilla_buckets_hold_their_fluid() {
        assert_eq!(
            held_fluid("minecraft:water_bucket"),
            Some("minecraft:water")
        );
        assert_eq!(held_fluid("minecraft:lava_bucket"), Some("minecraft:lava"));
        assert_eq!(held_fluid("minecraft:bucket"), None);
        assert_eq!(held_fluid("minecraft:cod_bucket"), None);
        assert_eq!(held_fluid("minecraft:powder_snow_bucket"), None);
    }

    #[test]
    fn a_model_for_no_world_fluid_draws_nothing_and_a_bad_sprite_is_ignored() {
        let drawn = resolve(
            &[look("a:oil", Some("a:block/oil"), None)],
            &[
                FluidModel {
                    still: Some("no namespace".into()),
                    ..model("a:oil")
                },
                model("a:ghost"),
            ],
        );
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].still, "a:block/oil");
    }
}
