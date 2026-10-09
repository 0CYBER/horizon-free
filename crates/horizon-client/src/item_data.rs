//! Item stack components beyond count and damage: enchantments, stored
//! (book) enchantments, `repair_cost`, `custom_name`, `potion_contents` and
//! `trim`, with their NBT form in vanilla's `components` compound.
use crate::items::ItemStack;
use horizon_world::nbt::{JavaString, Tag};
use std::sync::Arc;

/// Enchantment id and level, in insertion order.
pub type Levels = Vec<(Arc<str>, u32)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackData {
    /// `minecraft:enchantments`.
    pub enchantments: Levels,
    /// `minecraft:stored_enchantments` (enchanted books).
    pub stored_enchantments: Levels,
    /// `minecraft:repair_cost` (anvil prior-work penalty).
    pub repair_cost: u32,
    /// `minecraft:custom_name` as plain text.
    pub custom_name: Option<String>,
    /// `minecraft:potion_contents.potion`.
    pub potion: Option<Arc<str>>,
    /// `minecraft:trim`: (material, pattern).
    pub trim: Option<(Arc<str>, Arc<str>)>,
    /// `minecraft:container` (shulker boxes): the non-empty slots.
    pub container: Vec<(u8, ItemStack)>,
    /// `minecraft:charged_projectiles` (a loaded crossbow).
    pub charged: Vec<ItemStack>,
    /// `minecraft:fireworks`: flight duration and explosions (None: the
    /// item's default).
    pub fireworks: Option<(u8, Vec<crate::particles::FireworkExplosion>)>,
    /// `minecraft:map_id` (filled maps).
    pub map_id: Option<i32>,
    /// `minecraft:writable_book_content` pages.
    pub writable_pages: Vec<String>,
    /// `minecraft:written_book_content`.
    pub written: Option<WrittenBook>,
    /// `minecraft:banner_patterns` (banners and shields).
    pub banner_patterns: Vec<crate::banner::BannerLayer>,
    /// `minecraft:base_color` (a shield's dye).
    pub base_color: Option<Arc<str>>,
    /// The raid captain's banner: `minecraft:banner_patterns` with the
    /// ominous layers, its item name, rarity and hidden tooltip.
    pub ominous_banner: bool,
    /// `minecraft:ominous_bottle_amplifier` (None: the item's default, 0).
    pub ominous_amplifier: Option<u8>,
}
/// `Raid.getBannerComponentPatch`: the ominous banner's pattern layers.
pub const OMINOUS_LAYERS: [(&str, &str); 8] = [
    ("rhombus", "cyan"),
    ("stripe_bottom", "light_gray"),
    ("stripe_center", "gray"),
    ("border", "light_gray"),
    ("stripe_middle", "black"),
    ("half_horizontal", "light_gray"),
    ("circle", "light_gray"),
    ("border", "black"),
];
/// `WrittenBookContent` with the pages as plain text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrittenBook {
    pub title: String,
    pub author: String,
    pub generation: u8,
    pub pages: Vec<String>,
}
impl StackData {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl ItemStack {
    pub fn new(item: &str, count: u32) -> Self {
        Self {
            item: Arc::from(item),
            count,
            damage: 0,
            extra: None,
        }
    }
    pub fn data(&self) -> Option<&StackData> {
        self.extra.as_deref()
    }
    pub fn data_mut(&mut self) -> &mut StackData {
        self.extra.get_or_insert_with(Box::default)
    }
    /// Drops an all-default component box so equal stacks compare equal.
    pub fn tidy(&mut self) {
        if self.extra.as_ref().is_some_and(|d| d.is_empty()) {
            self.extra = None;
        }
    }
    pub fn enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.enchantments)
    }
    pub fn stored_enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.stored_enchantments)
    }
    /// `EnchantmentHelper.getItemEnchantmentLevel` (0 when absent).
    pub fn enchantment_level(&self, id: &str) -> u32 {
        self.enchantments()
            .iter()
            .find(|(e, _)| &**e == id)
            .map_or(0, |(_, l)| *l)
    }
    /// The component an anvil/grindstone works on: stored for books.
    pub fn crafting_enchantments(&self) -> &[(Arc<str>, u32)] {
        if &*self.item == "minecraft:enchanted_book" {
            self.stored_enchantments()
        } else {
            self.enchantments()
        }
    }
    pub fn set_crafting_enchantments(&mut self, levels: Levels) {
        if &*self.item == "minecraft:enchanted_book" {
            self.data_mut().stored_enchantments = levels;
        } else {
            self.data_mut().enchantments = levels;
        }
        self.tidy();
    }
    /// `ItemStack.enchant`: upgrade to the higher level.
    pub fn enchant(&mut self, id: &str, level: u32) {
        let book = &*self.item == "minecraft:enchanted_book";
        let data = self.data_mut();
        let list = if book {
            &mut data.stored_enchantments
        } else {
            &mut data.enchantments
        };
        upgrade(list, id, level);
    }
    pub fn has_any_enchantments(&self) -> bool {
        !self.enchantments().is_empty() || !self.stored_enchantments().is_empty()
    }
    pub fn repair_cost(&self) -> u32 {
        self.data().map_or(0, |d| d.repair_cost)
    }
    pub fn custom_name(&self) -> Option<&str> {
        self.data().and_then(|d| d.custom_name.as_deref())
    }
    pub fn potion(&self) -> Option<&str> {
        self.data().and_then(|d| d.potion.as_deref())
    }
    pub fn with_potion(mut self, potion: &str) -> Self {
        self.data_mut().potion = Some(Arc::from(potion));
        self
    }
    /// `ItemStack.isSameItemSameComponents`.
    pub fn same_item_same_components(&self, other: &Self) -> bool {
        self.item == other.item && self.damage == other.damage && self.data() == other.data()
    }
    /// `ItemStack.transmuteCopy`: another item keeping every component.
    pub fn transmute(&self, item: &str, count: u32) -> Self {
        let mut stack = Self {
            item: Arc::from(item),
            count,
            ..self.clone()
        };
        stack.tidy();
        stack
    }
}
/// `ItemEnchantments.Mutable.upgrade`.
pub fn upgrade(list: &mut Levels, id: &str, level: u32) {
    if level == 0 {
        return;
    }
    match list.iter_mut().find(|(e, _)| &**e == id) {
        Some((_, l)) => *l = (*l).max(level),
        None => list.push((Arc::from(id), level)),
    }
}
/// `ItemEnchantments.Mutable.set` (level 0 removes).
pub fn set_level(list: &mut Levels, id: &str, level: u32) {
    if level == 0 {
        list.retain(|(e, _)| &**e != id);
    } else {
        match list.iter_mut().find(|(e, _)| &**e == id) {
            Some((_, l)) => *l = level,
            None => list.push((Arc::from(id), level)),
        }
    }
}

fn levels_tag(levels: &Levels) -> Tag {
    Tag::compound(levels.iter().map(|(id, l)| (&**id, Tag::Int(*l as i32))))
}
fn int(t: &Tag) -> Option<i64> {
    match t {
        Tag::Byte(v) => Some(i64::from(*v)),
        Tag::Short(v) => Some(i64::from(*v)),
        Tag::Int(v) => Some(i64::from(*v)),
        Tag::Long(v) => Some(*v),
        _ => None,
    }
}
fn string(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}
fn read_levels(t: Option<&Tag>) -> Levels {
    let Some(Tag::Compound(map)) = t else {
        return Vec::new();
    };
    // Pre-1.21.5 worlds wrap the map in {levels: {...}}.
    if let Some(Tag::Compound(inner)) = map.get(&JavaString::from("levels")) {
        return inner
            .iter()
            .filter_map(|(k, v)| {
                Some((
                    Arc::from(k.to_string_lossy().as_str()),
                    int(v)?.clamp(1, 255) as u32,
                ))
            })
            .collect();
    }
    map.iter()
        .filter_map(|(k, v)| {
            Some((
                Arc::from(k.to_string_lossy().as_str()),
                int(v)?.clamp(1, 255) as u32,
            ))
        })
        .collect()
}
/// A text component as stored by vanilla: a plain string or `{"text": ...}`.
/// A `Filterable<String/Component>`: the value itself or `{raw: value}`.
fn filterable_text(t: &Tag) -> Option<String> {
    match t.get("raw") {
        Some(raw) => read_text(raw),
        None => read_text(t),
    }
}
fn read_text(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        Tag::Compound(_) => t.get("text").and_then(string),
        _ => None,
    }
}

/// The `components` compound for a stack, or None when every component is default.
pub fn encode_components(stack: &ItemStack) -> Option<Tag> {
    components_tag(stack.damage, stack.data())
}
/// `encode_components` from the damage value and extra components.
pub fn components_tag(damage: u32, data: Option<&StackData>) -> Option<Tag> {
    let mut entries: Vec<(&str, Tag)> = Vec::new();
    if damage > 0 {
        entries.push(("minecraft:damage", Tag::Int(damage as i32)));
    }
    if let Some(d) = data {
        if !d.enchantments.is_empty() {
            entries.push(("minecraft:enchantments", levels_tag(&d.enchantments)));
        }
        if !d.stored_enchantments.is_empty() {
            entries.push((
                "minecraft:stored_enchantments",
                levels_tag(&d.stored_enchantments),
            ));
        }
        if d.repair_cost > 0 {
            entries.push(("minecraft:repair_cost", Tag::Int(d.repair_cost as i32)));
        }
        if let Some(name) = &d.custom_name {
            entries.push(("minecraft:custom_name", Tag::String(name.as_str().into())));
        }
        if let Some(potion) = &d.potion {
            entries.push((
                "minecraft:potion_contents",
                Tag::compound([("potion", Tag::String((&**potion).into()))]),
            ));
        }
        if let Some((material, pattern)) = &d.trim {
            entries.push((
                "minecraft:trim",
                Tag::compound([
                    ("material", Tag::String((&**material).into())),
                    ("pattern", Tag::String((&**pattern).into())),
                ]),
            ));
        }
        if let Some(id) = d.map_id {
            entries.push(("minecraft:map_id", Tag::Int(id)));
        }
        if !d.banner_patterns.is_empty() {
            entries.push((
                "minecraft:banner_patterns",
                crate::banner::layers_to_nbt(&d.banner_patterns),
            ));
        }
        if let Some(color) = &d.base_color {
            entries.push(("minecraft:base_color", Tag::String((&**color).into())));
        }
        // Filterable values are saved as {raw: ..}.
        let raw = |t: Tag| Tag::compound([("raw", t)]);
        if !d.writable_pages.is_empty() {
            let pages = d
                .writable_pages
                .iter()
                .map(|p| raw(Tag::String(p.as_str().into())))
                .collect();
            entries.push((
                "minecraft:writable_book_content",
                Tag::compound([("pages", Tag::List(pages))]),
            ));
        }
        if let Some(w) = &d.written {
            let pages = w
                .pages
                .iter()
                .map(|p| raw(Tag::String(p.as_str().into())))
                .collect();
            entries.push((
                "minecraft:written_book_content",
                Tag::compound([
                    ("title", raw(Tag::String(w.title.as_str().into()))),
                    ("author", Tag::String(w.author.as_str().into())),
                    ("generation", Tag::Int(i32::from(w.generation))),
                    ("pages", Tag::List(pages)),
                    ("resolved", Tag::Byte(1)),
                ]),
            ));
        }
        if let Some((flight, explosions)) = &d.fireworks {
            let list = explosions
                .iter()
                .map(|e| {
                    Tag::compound([
                        ("shape", Tag::String(e.shape.name().into())),
                        (
                            "colors",
                            Tag::IntArray(e.colors.iter().map(|c| *c as i32).collect()),
                        ),
                        (
                            "fade_colors",
                            Tag::IntArray(e.fade.iter().map(|c| *c as i32).collect()),
                        ),
                        ("has_trail", Tag::Byte(i8::from(e.trail))),
                        ("has_twinkle", Tag::Byte(i8::from(e.twinkle))),
                    ])
                })
                .collect();
            entries.push((
                "minecraft:fireworks",
                Tag::compound([
                    ("flight_duration", Tag::Byte(*flight as i8)),
                    ("explosions", Tag::List(list)),
                ]),
            ));
        }
        if !d.charged.is_empty() {
            entries.push((
                "minecraft:charged_projectiles",
                Tag::List(d.charged.iter().map(crate::save::item_to_nbt).collect()),
            ));
        }
        if d.ominous_banner {
            let layer = |pattern: &str, color: &str| {
                Tag::compound([
                    (
                        "pattern",
                        Tag::String(format!("minecraft:{pattern}").as_str().into()),
                    ),
                    ("color", Tag::String(color.into())),
                ])
            };
            entries.push((
                "minecraft:banner_patterns",
                Tag::List(OMINOUS_LAYERS.iter().map(|(p, c)| layer(p, c)).collect()),
            ));
            entries.push((
                "minecraft:tooltip_display",
                Tag::compound([(
                    "hidden_components",
                    Tag::List(vec![Tag::String("minecraft:banner_patterns".into())]),
                )]),
            ));
            entries.push((
                "minecraft:item_name",
                Tag::compound([(
                    "translate",
                    Tag::String("block.minecraft.ominous_banner".into()),
                )]),
            ));
            entries.push(("minecraft:rarity", Tag::String("uncommon".into())));
        }
        if let Some(a) = d.ominous_amplifier {
            entries.push(("minecraft:ominous_bottle_amplifier", Tag::Int(i32::from(a))));
        }
        if !d.container.is_empty() {
            let slots = d
                .container
                .iter()
                .map(|(slot, stack)| {
                    Tag::compound([
                        ("slot", Tag::Int(i32::from(*slot))),
                        ("item", crate::save::item_to_nbt(stack)),
                    ])
                })
                .collect();
            entries.push(("minecraft:container", Tag::List(slots)));
        }
    }
    (!entries.is_empty()).then(|| Tag::compound(entries))
}
/// Damage and extra components from a stack's `components` compound.
pub fn decode_components(components: Option<&Tag>) -> (u32, Option<Box<StackData>>) {
    let Some(c) = components else {
        return (0, None);
    };
    let damage = c.get("minecraft:damage").and_then(int).unwrap_or(0).max(0) as u32;
    let potion = match c.get("minecraft:potion_contents") {
        Some(Tag::String(s)) => Some(Arc::from(s.to_string_lossy().as_str())),
        Some(t) => t
            .get("potion")
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        None => None,
    };
    let data = StackData {
        enchantments: read_levels(c.get("minecraft:enchantments")),
        stored_enchantments: read_levels(c.get("minecraft:stored_enchantments")),
        repair_cost: c
            .get("minecraft:repair_cost")
            .and_then(int)
            .unwrap_or(0)
            .max(0) as u32,
        custom_name: c.get("minecraft:custom_name").and_then(read_text),
        potion,
        map_id: c.get("minecraft:map_id").and_then(int).map(|v| v as i32),
        banner_patterns: crate::banner::layers_from_nbt(c.get("minecraft:banner_patterns")),
        base_color: c
            .get("minecraft:base_color")
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        writable_pages: match c
            .get("minecraft:writable_book_content")
            .and_then(|b| b.get("pages"))
        {
            Some(Tag::List(pages)) => pages.iter().filter_map(filterable_text).collect(),
            _ => Vec::new(),
        },
        written: c
            .get("minecraft:written_book_content")
            .map(|b| WrittenBook {
                title: b.get("title").and_then(filterable_text).unwrap_or_default(),
                author: b.get("author").and_then(string).unwrap_or_default(),
                generation: b.get("generation").and_then(int).unwrap_or(0).clamp(0, 3) as u8,
                pages: match b.get("pages") {
                    Some(Tag::List(pages)) => pages.iter().filter_map(filterable_text).collect(),
                    _ => Vec::new(),
                },
            }),
        fireworks: c.get("minecraft:fireworks").map(|f| {
            let flight = f
                .get("flight_duration")
                .and_then(int)
                .unwrap_or(0)
                .clamp(0, 255) as u8;
            let ints = |t: Option<&Tag>| match t {
                Some(Tag::IntArray(v)) => v.iter().map(|c| *c as u32).collect(),
                _ => Vec::new(),
            };
            let explosions = match f.get("explosions") {
                Some(Tag::List(list)) => list
                    .iter()
                    .map(|e| crate::particles::FireworkExplosion {
                        shape: crate::particles::FireworkShape::from_name(
                            e.get("shape")
                                .and_then(string)
                                .as_deref()
                                .unwrap_or("small_ball"),
                        ),
                        colors: ints(e.get("colors")),
                        fade: ints(e.get("fade_colors")),
                        trail: e.get("has_trail").and_then(int).unwrap_or(0) != 0,
                        twinkle: e.get("has_twinkle").and_then(int).unwrap_or(0) != 0,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            (flight, explosions)
        }),
        charged: match c.get("minecraft:charged_projectiles") {
            Some(Tag::List(list)) => list.iter().filter_map(crate::save::item_from_nbt).collect(),
            _ => Vec::new(),
        },
        container: match c.get("minecraft:container") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    Some((
                        e.get("slot").and_then(int)?.clamp(0, 255) as u8,
                        crate::save::item_from_nbt(e.get("item")?)?,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        },
        ominous_banner: matches!(
            c.get("minecraft:banner_patterns"),
            Some(Tag::List(layers)) if layers.first().is_some_and(|l|
                l.get("pattern").and_then(string).as_deref() == Some("minecraft:rhombus"))
        ),
        ominous_amplifier: c
            .get("minecraft:ominous_bottle_amplifier")
            .and_then(int)
            .map(|v| v.clamp(0, 4) as u8),
        trim: c.get("minecraft:trim").and_then(|t| {
            Some((
                Arc::from(t.get("material").and_then(string)?.as_str()),
                Arc::from(t.get("pattern").and_then(string)?.as_str()),
            ))
        }),
    };
    (damage, (!data.is_empty()).then(|| Box::new(data)))
}

/// The `[id=value,...]` component list of an item argument (`ItemParser`)
/// as a `components` compound; `!id` removals are skipped.
pub fn command_components(text: &str) -> Result<Tag, String> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or("expected '['")?;
    let mut entries = horizon_world::nbt::Compound::new();
    let mut rest = inner.trim_start();
    while !rest.is_empty() {
        let end = rest.find(['=', ',']).unwrap_or(rest.len());
        let key = rest[..end].trim();
        rest = &rest[end..];
        if let Some(value) = rest.strip_prefix('=') {
            let (tag, used) = horizon_world::nbt::parse_snbt(value)?;
            let id = if key.contains(':') {
                key.to_owned()
            } else {
                format!("minecraft:{key}")
            };
            entries.insert(JavaString::from(id.as_str()), tag);
            rest = &value[used..];
        }
        rest = rest.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
    Ok(Tag::Compound(entries))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_round_trip() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.damage = 12;
        stack.enchant("minecraft:sharpness", 3);
        stack.enchant("minecraft:unbreaking", 2);
        stack.data_mut().repair_cost = 3;
        stack.data_mut().custom_name = Some("Blade".into());
        let tag = encode_components(&stack).expect("components");
        let (damage, extra) = decode_components(Some(&tag));
        assert_eq!(damage, 12);
        let back = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:diamond_sword", 1)
        };
        assert_eq!(back.enchantment_level("minecraft:sharpness"), 3);
        assert_eq!(back.enchantment_level("minecraft:unbreaking"), 2);
        assert_eq!(back.repair_cost(), 3);
        assert_eq!(back.custom_name(), Some("Blade"));
    }

    #[test]
    fn banner_layers_and_shield_base_round_trip() {
        let mut stack = ItemStack::new("minecraft:shield", 1);
        stack.data_mut().base_color = Some(Arc::from("red"));
        stack.data_mut().banner_patterns = vec![
            crate::banner::BannerLayer::new("minecraft:flower", "black"),
            crate::banner::BannerLayer::new("minecraft:border", "white"),
        ];
        let tag = encode_components(&stack).expect("components");
        let (_, extra) = decode_components(Some(&tag));
        let extra = extra.expect("data");
        assert_eq!(extra.base_color.as_deref(), Some("red"));
        assert_eq!(extra.banner_patterns, stack.data().unwrap().banner_patterns);
    }

    #[test]
    fn ominous_items_round_trip() {
        let mut banner = ItemStack::new("minecraft:white_banner", 1);
        banner.data_mut().ominous_banner = true;
        let tag = encode_components(&banner).unwrap();
        assert!(tag.get("minecraft:banner_patterns").is_some());
        let (_, back) = decode_components(Some(&tag));
        assert!(back.unwrap().ominous_banner);
        let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
        bottle.data_mut().ominous_amplifier = Some(3);
        let (_, back) = decode_components(encode_components(&bottle).as_ref());
        assert_eq!(back.unwrap().ominous_amplifier, Some(3));
    }
    #[test]
    fn plain_stacks_have_no_components() {
        let stack = ItemStack::new("minecraft:stone", 64);
        assert!(encode_components(&stack).is_none());
        let mut named = stack.clone();
        named.data_mut();
        named.tidy();
        assert_eq!(named, stack);
    }

    #[test]
    fn books_store_enchantments() {
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        book.enchant("minecraft:mending", 1);
        assert!(book.enchantments().is_empty());
        assert_eq!(book.stored_enchantments().len(), 1);
        upgrade(
            &mut book.data_mut().stored_enchantments,
            "minecraft:mending",
            1,
        );
        assert_eq!(book.crafting_enchantments()[0].1, 1);
        let book = ItemStack::new("minecraft:book", 1).transmute("minecraft:enchanted_book", 1);
        assert!(book.extra.is_none());
    }
    #[test]
    fn command_components_decode_fireworks() {
        let tag = command_components(
            "[fireworks={flight_duration:2,explosions:[{shape:large_ball,colors:[I;16711680],has_twinkle:true}]},damage=3]",
        )
        .unwrap();
        let (damage, data) = decode_components(Some(&tag));
        assert_eq!(damage, 3);
        let (flight, explosions) = data.unwrap().fireworks.unwrap();
        assert_eq!(flight, 2);
        assert_eq!(
            explosions[0].shape,
            crate::particles::FireworkShape::LargeBall
        );
        assert_eq!(explosions[0].colors, vec![0xFF0000]);
        assert!(explosions[0].twinkle && !explosions[0].trail);
    }
}
