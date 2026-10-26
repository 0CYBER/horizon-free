//! Custom boss bars (`CustomBossEvents`): the `/bossbar` store, its saved
//! form (`data/minecraft/custom_boss_events.dat`) and how a bar is shown.
use crate::{
    scoreboard::{Comp, bracketed, comp_text, json_to_tag, literal, tag_to_json},
    text::{self, Style, Text},
};
use horizon_world::nbt::{Compound, Tag};

pub const COLORS: [&str; 7] = ["pink", "blue", "red", "green", "yellow", "purple", "white"];
pub const STYLES: [&str; 5] = [
    "progress",
    "notched_6",
    "notched_10",
    "notched_12",
    "notched_20",
];
/// `BossBarColor.getFormatting`. Purple is dark blue in 26.3.
pub fn color_rgb(color: &str) -> u32 {
    let name = match color {
        "pink" => "red",
        "red" => "dark_red",
        "purple" => "dark_blue",
        other => other,
    };
    text::named_color(name).unwrap_or(text::WHITE)
}

/// A version-4 shaped id from a resource id. Saved boss events do not keep
/// their UUID (`CustomBossEvents` mints a new one on load).
pub fn stable_uuid(id: &str) -> u128 {
    let mut h = 0xcbf29ce484222325u64;
    for b in id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    let most = (h as i64) & -61441 | 16384;
    let least = (h.rotate_left(17) as i64) & 4_611_686_018_427_387_903 | i64::MIN;
    ((most as u64 as u128) << 64) | (least as u64 as u128)
}

#[derive(Clone, Debug, PartialEq)]
pub struct BossBar {
    pub id: String,
    /// The `ServerBossEvent` UUID. Not saved.
    pub uuid: u128,
    pub name: Comp,
    pub color: String,
    pub style: String,
    pub value: i32,
    pub max: i32,
    /// `ServerBossEvent.visible`. A new bar is visible; nobody sees it until
    /// a player is added.
    pub visible: bool,
    pub darken: bool,
    pub music: bool,
    pub fog: bool,
    /// `CustomBossEvent.players`, the UUIDs told about the bar.
    pub players: Vec<[i32; 4]>,
}
impl BossBar {
    pub fn new(id: &str, name: Comp) -> Self {
        Self {
            id: id.to_owned(),
            uuid: stable_uuid(id),
            name,
            color: "white".into(),
            style: "progress".into(),
            value: 0,
            max: 100,
            visible: true,
            darken: false,
            music: false,
            fog: false,
            players: Vec::new(),
        }
    }
    /// `CustomBossEvent.getDisplayName`.
    pub fn display_name(&self) -> Text {
        let style = Style {
            color: Some(color_rgb(&self.color)),
            hover: Some(Box::new(Text::literal(self.id.clone()))),
            ..Style::default()
        };
        // ComponentUtils.wrapInSquareBrackets(name).withStyle(...): the
        // brackets take the colour too.
        bracketed(comp_text(&self.name)).with_style(&style)
    }
    /// The filled fraction of the bar (`BossEvent.getProgress`).
    pub fn progress(&self) -> f32 {
        (self.value as f32 / self.max.max(1) as f32).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BossBars {
    pub bars: Vec<BossBar>,
}
impl BossBars {
    pub fn get(&self, id: &str) -> Option<&BossBar> {
        self.bars.iter().find(|b| b.id == id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut BossBar> {
        self.bars.iter_mut().find(|b| b.id == id)
    }
    pub fn add(&mut self, id: &str, name: Comp) -> bool {
        if self.get(id).is_some() {
            return false;
        }
        self.bars.push(BossBar::new(id, name));
        true
    }
    pub fn remove(&mut self, id: &str) {
        self.bars.retain(|b| b.id != id);
    }
    pub fn ids(&self) -> Vec<String> {
        self.bars.iter().map(|b| b.id.clone()).collect()
    }
    /// The saved compound (`CustomBossEvent.Packed`).
    pub fn pack(&self) -> Tag {
        let mut c = Compound::new();
        for b in &self.bars {
            let players = b
                .players
                .iter()
                .map(|u| Tag::IntArray(u.to_vec()))
                .collect();
            c.insert(
                b.id.as_str().into(),
                Tag::compound([
                    ("Name", json_to_tag(&b.name)),
                    ("Visible", Tag::Byte(i8::from(b.visible))),
                    ("Value", Tag::Int(b.value)),
                    ("Max", Tag::Int(b.max)),
                    ("Color", Tag::String(b.color.as_str().into())),
                    ("Overlay", Tag::String(b.style.as_str().into())),
                    ("DarkenScreen", Tag::Byte(i8::from(b.darken))),
                    ("PlayBossMusic", Tag::Byte(i8::from(b.music))),
                    ("CreateWorldFog", Tag::Byte(i8::from(b.fog))),
                    ("Players", Tag::List(players)),
                ]),
            );
        }
        Tag::Compound(c)
    }
    pub fn unpack(data: &Tag) -> Self {
        let mut bars = Vec::new();
        if let Tag::Compound(c) = data {
            for (id, e) in c {
                let get = |k: &str| e.get(k);
                let text = |k: &str, d: &str| match get(k) {
                    Some(Tag::String(s)) => s.to_string_lossy(),
                    _ => d.to_owned(),
                };
                let num = |k: &str, d: i32| match get(k) {
                    Some(Tag::Int(v)) => *v,
                    Some(Tag::Short(v)) => i32::from(*v),
                    Some(Tag::Byte(v)) => i32::from(*v),
                    _ => d,
                };
                let players = match get("Players") {
                    Some(Tag::List(l)) => l
                        .iter()
                        .filter_map(|t| match t {
                            Tag::IntArray(a) if a.len() == 4 => Some([a[0], a[1], a[2], a[3]]),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let id = id.to_string_lossy();
                bars.push(BossBar {
                    uuid: stable_uuid(&id),
                    id,
                    name: get("Name").map_or_else(|| literal(""), tag_to_json),
                    color: text("Color", "white"),
                    style: text("Overlay", "progress"),
                    value: num("Value", 0),
                    max: num("Max", 100),
                    visible: num("Visible", 0) != 0,
                    darken: num("DarkenScreen", 0) != 0,
                    music: num("PlayBossMusic", 0) != 0,
                    fog: num("CreateWorldFog", 0) != 0,
                    players,
                });
            }
        }
        Self { bars }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_round_trip_and_show() {
        let mut b = BossBars::default();
        assert!(b.add("test:x", literal("Hello")));
        assert!(!b.add("test:x", literal("Again")));
        let bar = b.get_mut("test:x").unwrap();
        bar.color = "pink".into();
        bar.style = "notched_10".into();
        bar.value = 25;
        bar.max = 50;
        bar.visible = true;
        bar.darken = true;
        bar.players = vec![[1, 2, 3, 4]];
        let back = BossBars::unpack(&b.pack());
        assert_eq!(back, b);
        let bar = b.get("test:x").unwrap();
        assert_eq!(bar.progress(), 0.5);
        assert_eq!(bar.display_name().plain(), "chat.square_brackets");
        assert_eq!(color_rgb("pink"), text::RED);
        assert_eq!(color_rgb("purple"), text::named_color("dark_blue").unwrap());
        b.remove("test:x");
        assert!(b.bars.is_empty());
    }
}
