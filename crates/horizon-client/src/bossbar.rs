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
/// `BossBarColor.getFormatting`.
pub fn color_rgb(color: &str) -> u32 {
    let name = match color {
        "pink" => "red",
        "red" => "dark_red",
        "purple" => "dark_purple",
        other => other,
    };
    text::named_color(name).unwrap_or(text::WHITE)
}

#[derive(Clone, Debug, PartialEq)]
pub struct BossBar {
    pub id: String,
    pub name: Comp,
    pub color: String,
    pub style: String,
    pub value: i32,
    pub max: i32,
    pub visible: bool,
    /// Whether the (single) local player is in the bar's player set.
    pub player: bool,
}
impl BossBar {
    pub fn new(id: &str, name: Comp) -> Self {
        Self {
            id: id.to_owned(),
            name,
            color: "white".into(),
            style: "progress".into(),
            value: 0,
            max: 100,
            visible: true,
            player: false,
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
    /// The saved compound; the player's UUID stands for "the local player".
    pub fn pack(&self, player: Option<[i32; 4]>) -> Tag {
        let mut c = Compound::new();
        for b in &self.bars {
            let mut e = vec![
                ("Name", json_to_tag(&b.name)),
                ("Visible", Tag::Byte(i8::from(b.visible))),
                ("Value", Tag::Int(b.value)),
                ("Max", Tag::Int(b.max)),
                ("Color", Tag::String(b.color.as_str().into())),
                ("Overlay", Tag::String(b.style.as_str().into())),
                ("DarkenScreen", Tag::Byte(0)),
                ("PlayBossMusic", Tag::Byte(0)),
                ("CreateWorldFog", Tag::Byte(0)),
            ];
            let players = match (b.player, player) {
                (true, Some(u)) => vec![Tag::IntArray(u.to_vec())],
                _ => Vec::new(),
            };
            e.push(("Players", Tag::List(players)));
            c.insert(b.id.as_str().into(), Tag::compound(e));
        }
        Tag::Compound(c)
    }
    pub fn unpack(data: &Tag, player: Option<[i32; 4]>) -> Self {
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
                let in_players = match (get("Players"), player) {
                    (Some(Tag::List(l)), Some(u)) => l
                        .iter()
                        .any(|t| matches!(t, Tag::IntArray(a) if a.as_slice() == u)),
                    _ => false,
                };
                bars.push(BossBar {
                    id: id.to_string_lossy(),
                    name: get("Name").map_or_else(|| literal(""), tag_to_json),
                    color: text("Color", "white"),
                    style: text("Overlay", "progress"),
                    value: num("Value", 0),
                    max: num("Max", 100),
                    visible: num("Visible", 0) != 0,
                    player: in_players,
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
        bar.player = true;
        let uuid = Some([1, 2, 3, 4]);
        let back = BossBars::unpack(&b.pack(uuid), uuid);
        assert_eq!(back, b);
        // Another player's UUID does not include the local player.
        assert!(!BossBars::unpack(&b.pack(uuid), Some([9, 9, 9, 9])).bars[0].player);
        let bar = b.get("test:x").unwrap();
        assert_eq!(bar.progress(), 0.5);
        assert_eq!(bar.display_name().plain(), "chat.square_brackets");
        assert_eq!(color_rgb("pink"), text::RED);
        assert_eq!(color_rgb("purple"), 0xAA00AA);
        b.remove("test:x");
        assert!(b.bars.is_empty());
    }
}
