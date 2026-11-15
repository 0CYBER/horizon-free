//! `special` item models: the block-entity style renderers an item can ask
//! for (`SpecialModelRenderers`), their codec fields and what each reads from
//! the stack (`extractArgument`).
use super::{Result, float, get, ident, object, required, string};
use crate::json::error as err;
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum SpecialKind {
    Bell,
    Banner {
        color: String,
        wall: bool,
    },
    Book {
        open_angle: f32,
        page1: f32,
        page2: f32,
    },
    Conduit,
    Chest {
        texture: String,
        openness: f32,
        chest_type: String,
    },
    CopperGolemStatue {
        texture: String,
        pose: String,
    },
    DecoratedPot,
    EndCube {
        effect: String,
    },
    Head {
        kind: String,
        texture: Option<String>,
        animation: f32,
    },
    PlayerHead,
    ShulkerBox {
        texture: String,
        openness: f32,
    },
    Shield,
    Trident,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SpecialModel {
    pub kind: SpecialKind,
}

const DYES: [&str; 16] = [
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
fn one_of(v: &Value, names: &[&str]) -> Result<String> {
    let s = string(v)?;
    if names.contains(&s.as_str()) {
        Ok(s)
    } else {
        Err(err("invalid enum name"))
    }
}

/// `SkullBlock.Type`: vanilla's kinds and those native mods registered
/// (`register_skull_kinds`).
fn skull_kind(v: &Value) -> Result<String> {
    const VANILLA: [&str; 7] = [
        "skeleton",
        "wither_skeleton",
        "player",
        "zombie",
        "creeper",
        "piglin",
        "dragon",
    ];
    let s = string(v)?;
    if VANILLA.contains(&s.as_str()) || super::is_skull_kind(&s) {
        Ok(s)
    } else {
        Err(err("invalid enum name"))
    }
}

/// Canonical builtin special-model type identity, shared by decoding and ownership reporting.
enum BuiltinSpecialType {
    Bell,
    Banner,
    Book,
    Conduit,
    Chest,
    CopperGolemStatue,
    DecoratedPot,
    EndCube,
    Head,
    PlayerHead,
    ShulkerBox,
    Shield,
    Trident,
}
impl BuiltinSpecialType {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "minecraft:bell" => Some(Self::Bell),
            "minecraft:banner" => Some(Self::Banner),
            "minecraft:book" => Some(Self::Book),
            "minecraft:conduit" => Some(Self::Conduit),
            "minecraft:chest" => Some(Self::Chest),
            "minecraft:copper_golem_statue" => Some(Self::CopperGolemStatue),
            "minecraft:decorated_pot" => Some(Self::DecoratedPot),
            "minecraft:end_cube" => Some(Self::EndCube),
            "minecraft:head" => Some(Self::Head),
            "minecraft:player_head" => Some(Self::PlayerHead),
            "minecraft:shulker_box" => Some(Self::ShulkerBox),
            "minecraft:shield" => Some(Self::Shield),
            "minecraft:trident" => Some(Self::Trident),
            _ => None,
        }
    }
}

impl SpecialModel {
    /// Whether `id` belongs to the game's builtin special-model decoder, before mod replacement.
    pub fn is_builtin_type(id: &str) -> bool {
        BuiltinSpecialType::from_id(id).is_some()
    }

    pub fn decode(v: &Value) -> Result<Self> {
        let m = object(v)?;
        let float_or = |key: &str, default: f32| get(m, key).map_or(Ok(default), float);
        let id = ident(required(m, "type")?)?;
        let builtin = BuiltinSpecialType::from_id(&id).ok_or_else(|| {
            err(&format!(
                "unknown special model type {id}: no mod registered it"
            ))
        })?;
        let kind = match builtin {
            BuiltinSpecialType::Bell => SpecialKind::Bell,
            BuiltinSpecialType::Banner => SpecialKind::Banner {
                color: one_of(required(m, "color")?, &DYES)?,
                wall: match get(m, "attachment") {
                    None => false,
                    Some(a) => one_of(a, &["wall", "ground"])? == "wall",
                },
            },
            BuiltinSpecialType::Book => SpecialKind::Book {
                open_angle: float(required(m, "open_angle")?)?,
                page1: float(required(m, "page1")?)?,
                page2: float(required(m, "page2")?)?,
            },
            BuiltinSpecialType::Conduit => SpecialKind::Conduit,
            BuiltinSpecialType::Chest => SpecialKind::Chest {
                texture: ident(required(m, "texture")?)?,
                openness: float_or("openness", 0.0)?,
                chest_type: match get(m, "chest_type") {
                    None => "single".to_owned(),
                    Some(t) => one_of(t, &["single", "left", "right"])?,
                },
            },
            BuiltinSpecialType::CopperGolemStatue => SpecialKind::CopperGolemStatue {
                texture: ident(required(m, "texture")?)?,
                pose: one_of(
                    required(m, "pose")?,
                    &["standing", "sitting", "running", "star"],
                )?,
            },
            BuiltinSpecialType::DecoratedPot => SpecialKind::DecoratedPot,
            BuiltinSpecialType::EndCube => SpecialKind::EndCube {
                effect: one_of(required(m, "effect")?, &["portal", "gateway"])?,
            },
            BuiltinSpecialType::Head => SpecialKind::Head {
                kind: skull_kind(required(m, "kind")?)?,
                texture: get(m, "texture").map(ident).transpose()?,
                animation: float_or("animation", 0.0)?,
            },
            BuiltinSpecialType::PlayerHead => SpecialKind::PlayerHead,
            BuiltinSpecialType::ShulkerBox => SpecialKind::ShulkerBox {
                texture: ident(required(m, "texture")?)?,
                openness: float_or("openness", 0.0)?,
            },
            BuiltinSpecialType::Shield => SpecialKind::Shield,
            BuiltinSpecialType::Trident => SpecialKind::Trident,
        };
        Ok(Self { kind })
    }
    /// The codec's encoding: optional fields equal to their default are omitted.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_owned(), v);
        };
        let ty = match &self.kind {
            SpecialKind::Bell => "bell",
            SpecialKind::Banner { color, wall } => {
                put("color", json!(color));
                if *wall {
                    put("attachment", json!("wall"));
                }
                "banner"
            }
            SpecialKind::Book {
                open_angle,
                page1,
                page2,
            } => {
                put("open_angle", json!(open_angle));
                put("page1", json!(page1));
                put("page2", json!(page2));
                "book"
            }
            SpecialKind::Conduit => "conduit",
            SpecialKind::Chest {
                texture,
                openness,
                chest_type,
            } => {
                put("texture", json!(texture));
                if *openness != 0.0 {
                    put("openness", json!(openness));
                }
                if chest_type != "single" {
                    put("chest_type", json!(chest_type));
                }
                "chest"
            }
            SpecialKind::CopperGolemStatue { texture, pose } => {
                put("texture", json!(texture));
                put("pose", json!(pose));
                "copper_golem_statue"
            }
            SpecialKind::DecoratedPot => "decorated_pot",
            SpecialKind::EndCube { effect } => {
                put("effect", json!(effect));
                "end_cube"
            }
            SpecialKind::Head {
                kind,
                texture,
                animation,
            } => {
                put("kind", json!(kind));
                if let Some(t) = texture {
                    put("texture", json!(t));
                }
                if *animation != 0.0 {
                    put("animation", json!(animation));
                }
                "head"
            }
            SpecialKind::PlayerHead => "player_head",
            SpecialKind::ShulkerBox { texture, openness } => {
                put("texture", json!(texture));
                if *openness != 0.0 {
                    put("openness", json!(openness));
                }
                "shulker_box"
            }
            SpecialKind::Shield => "shield",
            SpecialKind::Trident => "trident",
        };
        m.insert("type".to_owned(), json!(format!("minecraft:{ty}")));
        Value::Object(m)
    }
    pub fn type_name(&self) -> String {
        self.to_json()["type"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    /// What `extractArgument` reads: banners their pattern layers, shields
    /// their base colour and layers, decorated pots their sherds and player
    /// heads their profile. Absent components give `null`.
    pub fn argument(&self, component: &dyn Fn(&str) -> Option<Value>) -> Value {
        match &self.kind {
            SpecialKind::Banner { .. } => {
                component("minecraft:banner_patterns").unwrap_or(Value::Null)
            }
            SpecialKind::DecoratedPot => {
                component("minecraft:pot_decorations").unwrap_or(Value::Null)
            }
            SpecialKind::PlayerHead => component("minecraft:profile").unwrap_or(Value::Null),
            SpecialKind::Shield => {
                let mut m = Map::new();
                for key in ["minecraft:banner_patterns", "minecraft:base_color"] {
                    if let Some(v) = component(key) {
                        m.insert(key.to_owned(), v);
                    }
                }
                Value::Object(m)
            }
            _ => Value::Null,
        }
    }
}
