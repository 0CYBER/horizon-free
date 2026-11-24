//! OptiFine's [Better Grass pack format](https://github.com/sp614x/optifine/blob/master/OptiFineDoc/doc/bettergrass.properties).
//! Texture names here are inputs to the block atlas, not meshing decisions.

use crate::{
    Properties, ctm,
    random_textures::{MAX_PROPERTIES, decode},
    resolve,
};
use horizon_assets::resource_source::ResourceSource;
use horizon_core::level_id::{valid_level_namespace, valid_level_path};
use std::borrow::Cow;

const FILE: &str = "assets/minecraft/optifine/bettergrass.properties";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    /// Fancy when a pack supplies its Better Grass file; off otherwise.
    #[default]
    Auto,
    Off,
    Fast,
    Fancy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Surface {
    Grass,
    DirtPath,
    Mycelium,
    Podzol,
    CrimsonNylium,
    WarpedNylium,
}

impl Surface {
    pub const ALL: [Self; 6] = [
        Self::Grass,
        Self::DirtPath,
        Self::Mycelium,
        Self::Podzol,
        Self::CrimsonNylium,
        Self::WarpedNylium,
    ];

    pub const fn block_id(self) -> &'static str {
        match self {
            Self::Grass => "minecraft:grass_block",
            Self::DirtPath => "minecraft:dirt_path",
            Self::Mycelium => "minecraft:mycelium",
            Self::Podzol => "minecraft:podzol",
            Self::CrimsonNylium => "minecraft:crimson_nylium",
            Self::WarpedNylium => "minecraft:warped_nylium",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Grass => "grass",
            Self::DirtPath => "dirt_path",
            Self::Mycelium => "mycelium",
            Self::Podzol => "podzol",
            Self::CrimsonNylium => "crimson_nylium",
            Self::WarpedNylium => "warped_nylium",
        }
    }

    fn snow_key(self) -> Option<&'static str> {
        match self {
            Self::Grass => Some("grass.snow"),
            Self::Mycelium => Some("mycelium.snow"),
            Self::Podzol => Some("podzol.snow"),
            Self::DirtPath | Self::CrimsonNylium | Self::WarpedNylium => None,
        }
    }

    fn default_texture(self) -> &'static str {
        match self {
            Self::Grass => "minecraft:block/grass_block_top",
            Self::DirtPath => "minecraft:block/dirt_path_top",
            Self::Mycelium => "minecraft:block/mycelium_top",
            Self::Podzol => "minecraft:block/podzol_top",
            Self::CrimsonNylium => "minecraft:block/crimson_nylium",
            Self::WarpedNylium => "minecraft:block/warped_nylium",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rules {
    dry: [bool; Surface::ALL.len()],
    snowy: [bool; Surface::ALL.len()],
    textures: [Cow<'static, str>; Surface::ALL.len()],
    snow: Cow<'static, str>,
    grass_side: Cow<'static, str>,
    dirt_path_side: Cow<'static, str>,
    grass_multilayer: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            dry: [true; Surface::ALL.len()],
            snowy: Surface::ALL.map(|surface| surface.snow_key().is_some()),
            textures: Surface::ALL.map(|surface| Cow::Borrowed(surface.default_texture())),
            snow: Cow::Borrowed("minecraft:block/snow"),
            grass_side: Cow::Borrowed("minecraft:block/grass_block_side"),
            dirt_path_side: Cow::Borrowed("minecraft:block/dirt_path_side"),
            grass_multilayer: false,
        }
    }
}

impl Rules {
    /// Read the winning file from the resolved pack stack. Absence is distinct from an empty
    /// or unreadable file, which uses the built-ins. A higher pack's file replaces, not merges
    /// with, lower packs' settings.
    pub fn load(source: &ResourceSource) -> Option<Self> {
        let bytes = match source.read_limited(FILE, MAX_PROPERTIES) {
            Ok(bytes) => bytes,
            Err(_) => return source.contains(FILE).then(Self::default),
        };
        let properties = Properties::parse(&decode(bytes));
        let flag = |key, default| match properties.get(key) {
            Some(value) if value.eq_ignore_ascii_case("true") => true,
            Some(value) if value.eq_ignore_ascii_case("false") => false,
            _ => default,
        };
        let mut rules = Self {
            dry: Surface::ALL.map(|surface| flag(surface.key(), true)),
            snowy: Surface::ALL
                .map(|surface| surface.snow_key().is_some_and(|key| flag(key, true))),
            grass_multilayer: flag("grass.multilayer", false),
            ..Self::default()
        };
        for (key, value) in properties.with_prefix("texture.") {
            let target = match key {
                "snow" => &mut rules.snow,
                "grass_side" => &mut rules.grass_side,
                "dirt_path_side" => &mut rules.dirt_path_side,
                _ => {
                    let Some(surface) = Surface::ALL
                        .into_iter()
                        .find(|surface| surface.key() == key)
                    else {
                        continue;
                    };
                    &mut rules.textures[surface as usize]
                }
            };
            if let Some(sprite) = sprite(source, value) {
                *target = sprite.into();
            }
        }
        Some(rules)
    }

    /// Snowy grass, mycelium and podzol have separate switches from their dry forms.
    pub fn enabled(&self, surface: Surface, snowy: bool) -> bool {
        let flags = if snowy { &self.snowy } else { &self.dry };
        flags[surface as usize]
    }

    /// An atlas sprite ID, or a full resource path for a file outside `textures/`. Missing or
    /// malformed overrides retain the built-in name. Only snow-capable blocks use `texture.snow`.
    pub fn texture(&self, surface: Surface, snowy: bool) -> &str {
        if snowy && surface.snow_key().is_some() {
            &self.snow
        } else {
            &self.textures[surface as usize]
        }
    }

    pub fn grass_multilayer(&self) -> bool {
        self.grass_multilayer
    }

    pub fn grass_side(&self) -> &str {
        &self.grass_side
    }

    pub fn dirt_path_side(&self) -> &str {
        &self.dirt_path_side
    }
}

fn sprite(source: &ResourceSource, value: &str) -> Option<String> {
    let resource = if value.starts_with("./") || value.starts_with("~/") {
        let mut resource = resolve(FILE, value)?;
        if !resource.ends_with(".png") {
            resource.push_str(".png");
        }
        resource
    } else {
        if let Some((namespace, _)) = value.split_once(':')
            && !namespace.is_empty()
            && !valid_level_namespace(namespace)
        {
            return None;
        }
        ctm::sprite_path(value)?
    };
    let (namespace, path) = resource.strip_prefix("assets/")?.split_once('/')?;
    let stem = path.strip_suffix(".png")?;
    if !valid_level_path(resource.strip_suffix(".png")?) || !source.contains(&resource) {
        return None;
    }
    match stem.strip_prefix("textures/") {
        Some(name) => Some(format!("{namespace}:{name}")),
        None => Some(resource),
    }
}
