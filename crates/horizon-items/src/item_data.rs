//! Item stack components beyond count and damage: enchantments, stored
//! (book) enchantments, `repair_cost`, `custom_name`, `potion_contents` and
//! `trim`, with their NBT form in vanilla's `components` compound.
#[path = "item_data/brewing_fuel.rs"]
pub mod brewing_fuel;
#[path = "item_data/component_values.rs"]
pub mod component_values;
#[path = "item_data/cooking_fuel.rs"]
pub mod cooking_fuel;
#[path = "item_data/custom_model_data.rs"]
pub mod custom_model_data;
#[path = "item_data/default_names.rs"]
pub mod default_names;
#[path = "item_data/ingredient_components.rs"]
pub mod ingredient_components;
use crate::items::{ItemInfo, ItemStack};
use horizon_core::nbt::{JavaString, Tag};

/// One item attribute modifier, as `ItemAttributeModifiers.Entry` holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModifierRow {
    /// The attribute's id (`type` in the component): `minecraft:armor`.
    pub attribute: String,
    /// The modifier's id.
    pub id: String,
    pub amount: f64,
    /// `add_value`, `add_multiplied_base` or `add_multiplied_total`.
    pub operation: String,
    /// The `EquipmentSlotGroup` it applies in: `mainhand`, `armor`, `any`...
    pub slot: String,
}
impl ModifierRow {
    pub fn new(attribute: &str, id: &str, amount: f64, operation: &str, slot: &str) -> Self {
        Self {
            attribute: attribute.to_owned(),
            id: id.to_owned(),
            amount,
            operation: operation.to_owned(),
            slot: slot.to_owned(),
        }
    }
}

/// `ItemAttributeModifiers.CODEC`: the entries of an `attribute_modifiers`
/// list (`type`, `id`, `amount`, `operation`, `slot` defaulting to `any`).
/// An entry missing a field is skipped.
pub fn modifier_rows(tag: &Tag) -> Vec<ModifierRow> {
    use horizon_client_core::nbt_value::{float, text};
    let Tag::List(entries) = tag else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|e| {
            Some(ModifierRow {
                attribute: horizon_world::datafix::dynamic::namespaced(&text(e.get("type"))?),
                id: horizon_world::datafix::dynamic::namespaced(&text(e.get("id"))?),
                amount: float(e.get("amount"))?,
                operation: text(e.get("operation"))?,
                slot: text(e.get("slot")).unwrap_or_else(|| "any".to_owned()),
            })
        })
        .collect()
}
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

/// Enchantment id and level, in insertion order (kept for saving and sending; stacks compare
/// them as a map, see [`same_levels`]).
pub type Levels = Vec<(Arc<str>, u32)>;

/// Whether `levels` names an id twice (say, saved as both `minecraft:sharpness` and
/// `sharpness`): the lookup reads the first entry, saving keeps the last.
fn repeats_an_id(levels: &Levels) -> bool {
    levels
        .iter()
        .enumerate()
        .any(|(i, (id, _))| levels[..i].iter().any(|(seen, _)| seen == id))
}

/// `ItemEnchantments.equals`: vanilla keeps the levels in a map, so two lists with the same
/// enchantments at the same levels are equal in any order. A list repeating an id has no single
/// map (the lookup and the saved form disagree), so it only equals the very same list.
fn same_levels(a: &Levels, b: &Levels) -> bool {
    if repeats_an_id(a) || repeats_an_id(b) {
        return a == b;
    }
    a.len() == b.len() && a.iter().all(|entry| b.contains(entry))
}

/// A hash agreeing with [`same_levels`]: in no particular order for a list of distinct ids,
/// as the list is for one repeating an id.
fn hash_levels<H: Hasher>(levels: &Levels, state: &mut H) {
    if repeats_an_id(levels) {
        levels.hash(state);
        return;
    }
    let mut sorted: Vec<_> = levels.iter().collect();
    sorted.sort_unstable();
    sorted.hash(state);
}
pub mod trim;
pub use trim::Trim;

/// A stack's modelled and preserved components. Start with `Default` (or
/// `ItemStack::data_mut`) and use the fields and setters; component storage may
/// gain private decoded data alongside its saved form.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct StackData {
    /// An explicit zero damage component overriding a damaged prototype.
    damage_set: bool,
    /// `minecraft:enchantments`.
    pub enchantments: Levels,
    /// `minecraft:stored_enchantments` (enchanted books).
    pub stored_enchantments: Levels,
    /// `minecraft:repair_cost` (anvil prior-work penalty).
    pub repair_cost: u32,
    /// An explicit `minecraft:damage` of zero. The damage is outside
    /// `StackData`, and a non-damageable item's prototype has no such
    /// component, so its presence is part of the stack's components.
    pub explicit_zero_damage: bool,
    /// The original `minecraft:custom_name` component; plain text is derived
    /// when gameplay needs it, so styled names have one source of truth.
    pub custom_name: Option<Tag>,
    /// Full kinetic component with its typed rendering timing, decoded together.
    kinetic_weapon: Option<crate::kinetic_weapon::KineticComponent>,
    /// Saved consumable and its typed runtime value, replaced together.
    consumable: Option<Arc<ConsumableComponent>>,
    death_protection: Option<Arc<Vec<crate::items::ConsumeEffect>>>,
    /// `minecraft:potion_contents.potion`.
    pub potion: Option<Arc<str>>,
    /// The rest of `minecraft:potion_contents` (`custom_color`, `custom_effects`,
    /// `custom_name`), kept as saved.
    pub potion_extra: Vec<(String, Tag)>,
    /// `minecraft:trim`: (material, pattern).
    pub trim: Option<Trim>,
    /// `minecraft:container` (shulker boxes): the non-empty slots.
    pub container: Vec<(u8, ItemStack)>,
    /// `minecraft:charged_projectiles` (a loaded crossbow).
    pub charged: Vec<ItemStack>,
    /// `minecraft:fireworks`: flight duration and explosions (None: the
    /// item's default).
    pub fireworks: Option<(u8, Vec<crate::firework::FireworkExplosion>)>,
    /// `minecraft:map_id` (filled maps).
    pub map_id: Option<i32>,
    /// `minecraft:writable_book_content` pages.
    pub writable_pages: Vec<String>,
    /// `minecraft:written_book_content`.
    pub written: Option<WrittenBook>,
    /// `minecraft:banner_patterns` (banners and shields).
    pub banner_patterns: Vec<crate::banner_layer::BannerLayer>,
    /// `minecraft:base_color` (a shield's dye).
    pub base_color: Option<Arc<str>>,
    /// The raid captain's banner: `minecraft:banner_patterns` with the
    /// ominous layers, its item name, rarity and hidden tooltip.
    pub ominous_banner: bool,
    /// `minecraft:ominous_bottle_amplifier` (None: the item's default, 0).
    pub ominous_amplifier: Option<u8>,
    /// `minecraft:dyed_color` (leather armour, wolf armour, ... as RGB).
    pub dyed_color: Option<i32>,
    /// `minecraft:firework_explosion` (a firework star).
    pub firework_explosion: Option<crate::firework::FireworkExplosion>,
    /// `minecraft:pot_decorations`: back, left, right, front sherds.
    pub pot_decorations: Option<Box<[Option<ItemStack>; 4]>>,
    /// `minecraft:bundle_contents`.
    pub bundle_contents: Vec<ItemStack>,
    /// `BundleContents.selectedItem`: runtime only, never saved or sent.
    pub bundle_selected: BundleSelected,
    /// `minecraft:suspicious_stew_effects`: (effect id, duration).
    pub stew_effects: Vec<(Arc<str>, i32)>,
    /// `minecraft:map_post_processing`: 0 lock, 1 scale.
    pub map_post_processing: Option<u8>,
    /// Components Horizon does not model (lore, custom data, item name,
    /// max stack size...), kept as their saved values in key order.
    pub other: Vec<(String, Tag)>,
    /// What the server sent for this stack, for sending it back unchanged.
    pub wire: Wire,
}
/// Field by field, except that the enchantment lists compare as maps ([`same_levels`]).
impl PartialEq for StackData {
    fn eq(&self, rhs: &Self) -> bool {
        let Self {
            damage_set,
            enchantments,
            stored_enchantments,
            repair_cost,
            explicit_zero_damage,
            custom_name,
            kinetic_weapon,
            consumable,
            death_protection,
            potion,
            potion_extra,
            trim,
            container,
            charged,
            fireworks,
            map_id,
            writable_pages,
            written,
            banner_patterns,
            base_color,
            ominous_banner,
            ominous_amplifier,
            dyed_color,
            firework_explosion,
            pot_decorations,
            bundle_contents,
            bundle_selected,
            stew_effects,
            map_post_processing,
            other,
            wire,
        } = self;
        same_levels(enchantments, &rhs.enchantments)
            && same_levels(stored_enchantments, &rhs.stored_enchantments)
            && *damage_set == rhs.damage_set
            && *repair_cost == rhs.repair_cost
            && *explicit_zero_damage == rhs.explicit_zero_damage
            && *custom_name == rhs.custom_name
            && *kinetic_weapon == rhs.kinetic_weapon
            && *consumable == rhs.consumable
            && *death_protection == rhs.death_protection
            && *potion == rhs.potion
            && *potion_extra == rhs.potion_extra
            && *trim == rhs.trim
            && *container == rhs.container
            && *charged == rhs.charged
            && *fireworks == rhs.fireworks
            && *map_id == rhs.map_id
            && *writable_pages == rhs.writable_pages
            && *written == rhs.written
            && *banner_patterns == rhs.banner_patterns
            && *base_color == rhs.base_color
            && *ominous_banner == rhs.ominous_banner
            && *ominous_amplifier == rhs.ominous_amplifier
            && *dyed_color == rhs.dyed_color
            && *firework_explosion == rhs.firework_explosion
            && *pot_decorations == rhs.pot_decorations
            && *bundle_contents == rhs.bundle_contents
            && *bundle_selected == rhs.bundle_selected
            && *stew_effects == rhs.stew_effects
            && *map_post_processing == rhs.map_post_processing
            && *other == rhs.other
            && *wire == rhs.wire
    }
}
impl Hash for StackData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let Self {
            damage_set,
            enchantments,
            stored_enchantments,
            repair_cost,
            explicit_zero_damage,
            custom_name,
            kinetic_weapon,
            consumable,
            death_protection: _,
            potion,
            potion_extra,
            trim,
            container,
            charged,
            fireworks,
            map_id,
            writable_pages,
            written,
            banner_patterns,
            base_color,
            ominous_banner,
            ominous_amplifier,
            dyed_color,
            firework_explosion,
            pot_decorations,
            bundle_contents,
            bundle_selected: _,
            stew_effects,
            map_post_processing,
            other,
            wire: _,
        } = self;
        damage_set.hash(state);
        hash_levels(enchantments, state);
        hash_levels(stored_enchantments, state);
        repair_cost.hash(state);
        explicit_zero_damage.hash(state);
        custom_name.hash(state);
        kinetic_weapon
            .as_ref()
            .map(|component| component.raw())
            .hash(state);
        consumable
            .as_ref()
            .map(|component| &component.raw)
            .hash(state);
        potion.hash(state);
        potion_extra.hash(state);
        trim.hash(state);
        container.hash(state);
        charged.hash(state);
        fireworks.hash(state);
        map_id.hash(state);
        writable_pages.hash(state);
        written.hash(state);
        banner_patterns.hash(state);
        base_color.hash(state);
        ominous_banner.hash(state);
        ominous_amplifier.hash(state);
        dyed_color.hash(state);
        firework_explosion.hash(state);
        pot_decorations.hash(state);
        bundle_contents.hash(state);
        stew_effects.hash(state);
        map_post_processing.hash(state);
        // Includes the saved death_protection component. The decoded cache
        // may contain floats and is only used by exact equality.
        other.hash(state);
    }
}

/// A saved consumable decoded once for use animation, countdown and completion.
#[derive(Clone, Debug, PartialEq)]
struct ConsumableComponent {
    raw: Tag,
    value: crate::items::ItemConsumable,
}
impl ConsumableComponent {
    fn new(raw: Tag) -> Option<Arc<Self>> {
        let mut value =
            crate::items::parse_consumable(&horizon_client_core::scoreboard::tag_to_json(&raw))
                .ok()?;
        value.seconds = raw
            .get("consume_seconds")
            .and_then(crate::properties::component_float)
            .unwrap_or(1.6);
        value.particles = raw
            .get("has_consume_particles")
            .and_then(component_bool)
            .unwrap_or(true);
        Some(Arc::new(Self { raw, value }))
    }
}
/// The component patch a server sent for a stack, as the creative slot packet
/// writes it, and the stack Horizon decoded from it, to tell whether anything
/// changed since. Like vanilla's `BundleContents.selectedItem` it is left out
/// when stacks are compared.
#[derive(Clone, Debug, Default)]
pub struct Wire(Option<Arc<(ItemStack, Arc<[u8]>)>>);
impl Wire {
    pub fn new(decoded: ItemStack, patch: Arc<[u8]>) -> Self {
        Self(Some(Arc::new((decoded, patch))))
    }
    /// The server's patch, whether or not the stack changed since.
    pub fn patch(&self) -> Option<&[u8]> {
        self.0.as_deref().map(|(_, patch)| &**patch)
    }
}
impl PartialEq for Wire {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
/// A bundle's selected item index (-1: none). Like vanilla's
/// `BundleContents.equals` it is left out when stacks are compared.
#[derive(Clone, Copy, Debug)]
pub struct BundleSelected(pub i32);
impl Default for BundleSelected {
    fn default() -> Self {
        Self(-1)
    }
}
impl PartialEq for BundleSelected {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
/// The components `components_tag` writes from modelled fields.
const MODELLED: &[&str] = &[
    "minecraft:banner_patterns",
    "minecraft:base_color",
    "minecraft:bundle_contents",
    "minecraft:charged_projectiles",
    "minecraft:container",
    "minecraft:consumable",
    "minecraft:custom_name",
    "minecraft:damage",
    "minecraft:dyed_color",
    "minecraft:enchantments",
    "minecraft:firework_explosion",
    "minecraft:fireworks",
    "minecraft:kinetic_weapon",
    "minecraft:map_id",
    "minecraft:map_post_processing",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:pot_decorations",
    "minecraft:potion_contents",
    "minecraft:repair_cost",
    "minecraft:stored_enchantments",
    "minecraft:suspicious_stew_effects",
    "minecraft:trim",
    "minecraft:writable_book_content",
    "minecraft:written_book_content",
];
impl StackData {
    pub fn custom_name_plain(&self) -> Option<String> {
        self.custom_name.as_ref().and_then(read_text)
    }

    /// The saved kinetic component and its decoded rendering timing.
    pub fn kinetic_weapon(&self) -> Option<&crate::kinetic_weapon::KineticComponent> {
        self.kinetic_weapon.as_ref()
    }
    /// A preserved component's saved value.
    pub fn other(&self, id: &str) -> Option<&Tag> {
        if id == "minecraft:custom_name" {
            return self.custom_name.as_ref();
        }
        if id == "minecraft:kinetic_weapon" {
            return self.kinetic_weapon.as_ref().map(|k| k.raw());
        }
        if id == "minecraft:consumable" {
            return self.consumable.as_ref().map(|c| &c.raw);
        }
        self.other.iter().find(|(k, _)| k == id).map(|(_, v)| v)
    }
    /// Whether the stack's own data sets component `id`, read from the live fields: the same
    /// components `components_tag` saves, plus the transient ones it leaves out
    /// (`map_post_processing`), without encoding anything.
    pub fn sets_component(&self, id: &str) -> bool {
        if self.other.iter().any(|(key, _)| key == id) {
            return true;
        }
        match id {
            "minecraft:damage" => self.damage_set,
            "minecraft:enchantments" => !self.enchantments.is_empty(),
            "minecraft:stored_enchantments" => !self.stored_enchantments.is_empty(),
            "minecraft:repair_cost" => self.repair_cost > 0,
            "minecraft:custom_name" => self.custom_name.is_some(),
            "minecraft:potion_contents" => self.potion.is_some() || !self.potion_extra.is_empty(),
            "minecraft:trim" => self.trim.is_some(),
            "minecraft:map_id" => self.map_id.is_some(),
            "minecraft:banner_patterns" => self.ominous_banner || !self.banner_patterns.is_empty(),
            "minecraft:base_color" => self.base_color.is_some(),
            "minecraft:writable_book_content" => !self.writable_pages.is_empty(),
            "minecraft:written_book_content" => self.written.is_some(),
            "minecraft:fireworks" => self.fireworks.is_some(),
            "minecraft:firework_explosion" => self.firework_explosion.is_some(),
            "minecraft:dyed_color" => self.dyed_color.is_some(),
            "minecraft:pot_decorations" => self.pot_decorations.is_some(),
            "minecraft:bundle_contents" => !self.bundle_contents.is_empty(),
            "minecraft:suspicious_stew_effects" => !self.stew_effects.is_empty(),
            "minecraft:charged_projectiles" => !self.charged.is_empty(),
            "minecraft:ominous_bottle_amplifier" => self.ominous_amplifier.is_some(),
            "minecraft:container" => !self.container.is_empty(),
            "minecraft:map_post_processing" => self.map_post_processing.is_some(),
            // An ominous banner's implied components (`components_tag`).
            "minecraft:item_name" | "minecraft:rarity" | "minecraft:tooltip_display"
                if self.ominous_banner =>
            {
                true
            }
            _ => self.other(id).is_some(),
        }
    }
    /// `SetNameFunction.run`: retain a loot name as its original component.
    pub fn set_loot_name(&mut self, id: &str, text: &serde_json::Value) {
        let tag = horizon_client_core::scoreboard::json_to_tag(text);
        self.set_other(id, Some(tag));
    }
    /// Apply an ordered loot component patch without changing its NBT type.
    /// None removes the component. Trim retains its modelled representation.
    pub fn set_loot_component(&mut self, id: &str, value: &Option<Tag>) {
        let tag = value.clone();
        match id {
            // `set_banner_pattern` (and `set_components`) write the layers a
            // banner or shield wears; the renderer and the tooltip read them
            // from `banner_patterns`, not from a tag in `other`.
            "minecraft:banner_patterns" => {
                self.banner_patterns = crate::banner_layer::layers_from_nbt(tag.as_ref());
            }
            // `set_book_cover` (and `set_components`) write the book's title,
            // author, generation and pages as one component: the fields the
            // tooltip and the reading screen use come back out of it.
            "minecraft:written_book_content" => {
                self.written = tag.as_ref().map(written_book_from_tag);
            }
            "minecraft:trim" => {
                self.trim = tag.as_ref().and_then(trim::from_nbt);
            }
            // The scalar lives on ItemStack; retain explicit zero presence here
            // so a non-durable prototype can carry its own durability.
            "minecraft:damage" => {
                self.explicit_zero_damage = tag.as_ref().is_some_and(|value| {
                    horizon_world::datafix::dynamic::as_int(Some(value), 0) == 0
                });
                self.set_other(id, tag);
            }
            "minecraft:custom_name" => self.set_other(id, tag),
            _ if MODELLED.contains(&id) => {
                let mut saved = match components_tag(0, Some(self)) {
                    Some(Tag::Compound(c)) => c,
                    _ => Default::default(),
                };
                saved.remove(&JavaString::from(id));
                if let Some(tag) = tag {
                    saved.insert(JavaString::from(id), tag);
                }
                // A damage set before stays where it was: reading would take it.
                let damage = self.other("minecraft:damage").cloned();
                let (_, read) = decode_components(Some(&Tag::Compound(saved)));
                *self = read.map_or_else(Default::default, |data| *data);
                self.set_other("minecraft:damage", damage);
            }
            _ => self.set_other(id, tag),
        }
    }
    /// Set (or with None remove) a preserved component.
    /// This setter deliberately remains infallible. Invalid opaque effect payloads
    /// refuse replacement without changing either the previous raw value or cache.
    pub fn set_other(&mut self, id: &str, value: Option<Tag>) {
        if id == "minecraft:death_protection" {
            let replacement = match value.as_ref() {
                Some(raw) => match crate::items::parse_death_effects(
                    &horizon_client_core::scoreboard::tag_to_json(raw),
                ) {
                    Ok(effects) => Some(Arc::new(effects)),
                    Err(_) => return,
                },
                None => None,
            };
            self.death_protection = replacement;
        }
        if id == "minecraft:consumable" {
            let replacement = match value {
                Some(raw) => match ConsumableComponent::new(raw) {
                    Some(component) => Some(component),
                    None => return,
                },
                None => None,
            };
            self.other.retain(|(k, _)| k != id);
            self.consumable = replacement;
            return;
        }
        self.other.retain(|(k, _)| k != id);
        if id == "minecraft:kinetic_weapon" {
            self.kinetic_weapon = value.map(crate::kinetic_weapon::KineticComponent::new);
            return;
        }
        if id == "minecraft:custom_name" {
            self.custom_name = value
                .filter(|tag| read_text(tag).is_some())
                .map(collapsed_text);
            return;
        }
        if let Some(v) = value {
            self.other.push((id.to_owned(), v));
            self.other.sort_by(|a, b| a.0.cmp(&b.0));
        }
    }
    /// `DebugStickState.properties().get(block)`: the property selected for
    /// `block` in `minecraft:debug_stick_state`.
    pub fn debug_stick_property(&self, block: &str) -> Option<String> {
        let Tag::Compound(map) = self.other("minecraft:debug_stick_state")? else {
            return None;
        };
        match map.get(&JavaString::from(block))? {
            Tag::String(property) => Some(property.to_string_lossy()),
            _ => None,
        }
    }
    /// `DebugStickState.withProperty(block, property)`.
    pub fn set_debug_stick_property(&mut self, block: &str, property: &str) {
        let mut map = match self.other("minecraft:debug_stick_state") {
            Some(Tag::Compound(map)) => map.clone(),
            _ => Default::default(),
        };
        map.insert(
            JavaString::from(block),
            Tag::String(JavaString::from(property)),
        );
        self.set_other("minecraft:debug_stick_state", Some(Tag::Compound(map)));
    }
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

/// The raid captain banner's default tooltip setting, stored by vanilla as a
/// component but represented by `StackData::ominous_banner` in Horizon.
fn ominous_tooltip_display() -> Tag {
    Tag::compound([(
        "hidden_components",
        Tag::List(vec![Tag::String("minecraft:banner_patterns".into())]),
    )])
}

/// `WrittenBookContent` with the pages as plain text.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub struct WrittenBook {
    pub title: String,
    pub author: String,
    pub generation: u8,
    pub pages: Vec<String>,
    /// Each page's component when it is not a plain string, so that its
    /// click events survive; empty or one per page.
    pub page_tags: Vec<Tag>,
    /// `WrittenBookContent.resolved`: a book whose selectors and scores have
    /// been resolved is never resolved again.
    pub resolved: bool,
}
impl StackData {
    pub fn is_empty(&self) -> bool {
        self.wire.0.is_none() && *self == Self::default()
    }
}

/// `BlockItem.onDestroyed` (`ItemStack.onDestroyed` for a container item): the
/// stack's `container` is emptied and the stacks it held come back in slot
/// order — `ItemUtils.onContainerDestroyed` drops each where the entity lay.
pub fn destroyed_container(stack: &mut ItemStack) -> Vec<ItemStack> {
    let Some(data) = stack.extra.as_deref_mut() else {
        return Vec::new();
    };
    if data.container.is_empty() {
        return Vec::new();
    }
    // `set(CONTAINER, ItemContainerContents.EMPTY)`: no slot left filled.
    let mut held = std::mem::take(&mut data.container);
    stack.tidy();
    // `nonEmptyItemCopyStream` walks the 256 slots in order.
    held.sort_by_key(|(slot, _)| *slot);
    held.into_iter().map(|(_, stack)| stack).collect()
}

/// `NbtOps.getBooleanValue`, used by boolean component codecs: every numeric tag
/// is true unless its double value is zero. In particular, fractions stay true.
pub fn component_bool(value: &Tag) -> Option<bool> {
    horizon_client_core::component_codec::nbt_bool(value)
}

/// `FoodProperties.DIRECT_CODEC`: required numeric nutrition/saturation and an
/// optional NbtOps boolean. Unknown fields are ignored; floats are unrestricted.
/// Reads borrowed NBT into the existing copyable value without allocating.
pub fn food_properties(
    value: &Tag,
) -> Result<horizon_world::food_properties::FoodProperties, &'static str> {
    if !matches!(value, Tag::Compound(_)) {
        return Err("food must be a map");
    }
    let nutrition = value
        .get("nutrition")
        .and_then(crate::properties::component_int)
        .ok_or("nutrition must be a number")?;
    if nutrition < 0 {
        return Err("nutrition must be non-negative");
    }
    let saturation = value
        .get("saturation")
        .and_then(crate::properties::component_float)
        .ok_or("saturation must be a number")?;
    let can_always_eat = value
        .get("can_always_eat")
        .map(component_bool)
        .unwrap_or(Some(false))
        .ok_or("can_always_eat must be a number")?;
    Ok(horizon_world::food_properties::FoodProperties {
        nutrition,
        saturation,
        can_always_eat,
    })
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
    /// The stack `ItemStack.CODEC` reads from `tag` (`id`, `count`,
    /// `components`), as `item_nbt::item_from_nbt` does; None for air or an
    /// id that is no valid identifier. The id is not looked up in a registry.
    pub fn from_nbt(tag: &horizon_core::nbt::Compound) -> Option<Self> {
        crate::item_nbt::item_from_nbt(&Tag::Compound(tag.clone()))
    }
    pub fn data(&self) -> Option<&StackData> {
        self.extra.as_deref()
    }
    /// Whether the stack carries component `id`: its own data sets it, or its item has it by
    /// default and the stack's changes don't remove it (a `!id` entry). Reads the live
    /// components, so nothing is encoded.
    pub fn carries_component(&self, info: Option<&ItemInfo>, id: &str) -> bool {
        let data = self.data();
        let own = if id == "minecraft:damage" {
            self.damage > 0 || data.is_some_and(|d| d.sets_component(id))
        } else {
            data.is_some_and(|d| d.sets_component(id))
        };
        if own {
            return true;
        }
        if data.is_some_and(|d| d.other.iter().any(|(k, _)| k.strip_prefix('!') == Some(id))) {
            return false;
        }
        info.is_some_and(|info| info.components.iter().any(|c| c == id))
    }
    /// Death protection effects retained at patch or prototype load.
    pub fn death_protection<'a>(
        &'a self,
        info: Option<&'a ItemInfo>,
    ) -> Option<&'a [crate::items::ConsumeEffect]> {
        if let Some(data) = self.data() {
            if let Some(value) = &data.death_protection {
                return Some(value);
            }
            if data.other("!minecraft:death_protection").is_some()
                || data.other("minecraft:death_protection").is_some()
            {
                return None;
            }
        }
        info.and_then(|i| i.death_protection.as_deref())
    }
    /// Effective consumable, decoded when the patch/prototype is loaded, with no tick allocation.
    pub fn consumable<'a>(
        &'a self,
        info: Option<&'a ItemInfo>,
    ) -> Option<&'a crate::items::ItemConsumable> {
        if let Some(data) = self.data() {
            if data.other("!minecraft:consumable").is_some() {
                return None;
            }
            if let Some(consumable) = &data.consumable {
                return Some(&consumable.value);
            }
        }
        info.and_then(|item| item.consumable.as_ref())
    }
    /// `ItemStack.getMaxDamage`: the stack override, removal, or item default.
    pub fn max_damage(&self, info: Option<&ItemInfo>) -> i32 {
        if !self.carries_component(info, "minecraft:max_damage") {
            return 0;
        }
        self.data()
            .and_then(|data| data.other("minecraft:max_damage"))
            .map(|value| crate::properties::component_int(value).unwrap_or(0))
            .unwrap_or_else(|| info.map_or(0, |item| item.max_damage as i32))
    }
    /// `ItemStack.getDamageValue`: a live patch, removal, or cached prototype damage.
    /// Preserve an explicit zero patch when the prototype starts damaged.
    pub fn effective_damage(&self, info: Option<&ItemInfo>) -> i32 {
        if self.damage > 0 || self.data().is_some_and(|data| data.damage_set) {
            return self.damage as i32;
        }
        if let Some(data) = self.data() {
            if let Some(value) = data.other("minecraft:damage") {
                return crate::properties::component_int(value).unwrap_or(0).max(0);
            }
            if data.other("!minecraft:damage").is_some() {
                return 0;
            }
        }
        info.map_or(0, |item| item.damage)
    }
    /// The stack's own `max_stack_size` component: `Some(1)` when the stack
    /// removes it (`!minecraft:max_stack_size`, vanilla's `getOrDefault`
    /// default of 1), `None` when it leaves the item's.
    pub fn own_max_stack_size(&self) -> Option<u32> {
        let data = self.data();
        if data.is_some_and(|d| d.other("!minecraft:max_stack_size").is_some()) {
            return Some(1);
        }
        data.and_then(|d| d.other("minecraft:max_stack_size"))
            .and_then(crate::properties::component_int)
            .map(|size| size.clamp(1, 99) as u32)
    }
    /// `ItemInstance.getMaxStackSize`: the stack's own `max_stack_size`, else
    /// the item's. A removed component is vanilla's `getOrDefault` default of
    /// 1, not the item's size.
    pub fn max_stack_size(&self, items: &crate::items::Items) -> u32 {
        self.own_max_stack_size()
            .unwrap_or_else(|| items.get(&self.item).map_or(64, |i| i.max_stack))
    }
    /// The patch the server sent for this stack, while the stack is still what
    /// it decoded to (its count aside); None once anything else changed.
    pub fn wire_patch(&self) -> Option<Arc<[u8]>> {
        let (decoded, patch) = &**self.data()?.wire.0.as_ref()?;
        let none = StackData::default();
        (decoded.item == self.item
            && decoded.damage == self.damage
            && decoded.data().unwrap_or(&none) == self.data().unwrap_or(&none))
        .then(|| patch.clone())
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
    /// `IItemStackExtension.getAllEnchantments`: the enchantments gameplay
    /// sees on the stack, its `minecraft:enchantments` as the mods'
    /// `get_enchantment_level` leaves them. Borrowed when no mod changes
    /// them. Tooltips, the anvil, the grindstone, saves and the network read
    /// the component itself ([`Self::enchantments`]).
    pub fn all_enchantments(&self) -> std::borrow::Cow<'_, [(Arc<str>, u32)]> {
        let own = self.enchantments();
        match crate::native_enchantment_hooks::get_enchantment_level(self, None, own) {
            Some(levels) => std::borrow::Cow::Owned(levels),
            None => std::borrow::Cow::Borrowed(own),
        }
    }
    /// `EnchantmentHelper.getItemEnchantmentLevel` as NeoForge has it
    /// (`IItemStackExtension.getEnchantmentLevel`): the level gameplay sees,
    /// after the mods' `get_enchantment_level` (0 when absent).
    pub fn enchantment_level(&self, id: &str) -> u32 {
        let own = self.enchantments();
        match crate::native_enchantment_hooks::get_enchantment_level(self, Some(id), own) {
            Some(levels) => level_in(&levels, id),
            None => level_in(own, id),
        }
    }
    /// `EnchantmentHelper.getTagEnchantmentLevel`: the level in the stack's
    /// own `minecraft:enchantments`, without the mods (0 when absent).
    pub fn tag_enchantment_level(&self, id: &str) -> u32 {
        level_in(self.enchantments(), id)
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
    /// `ItemStack.enchant`: upgrade to the higher level. Vanilla's
    /// `EnchantmentHelper.updateEnchantments` returns EMPTY when the component
    /// is absent, so a removal patch makes this a no-op.
    pub fn enchant(&mut self, id: &str, level: u32) {
        let book = &*self.item == "minecraft:enchanted_book";
        let removal = if book {
            "!minecraft:stored_enchantments"
        } else {
            "!minecraft:enchantments"
        };
        if self
            .data()
            .is_some_and(|data| data.other(removal).is_some())
        {
            return;
        }
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
    /// `ItemStack.hasFoil`: an explicit `minecraft:enchantment_glint_override`
    /// component wins, then the item's default override (`item_glint`), then a
    /// `Item.isFoil` (enchantments, or a compass carrying a lodestone tracker).
    /// Stored book enchantments do not count.
    pub fn has_foil(&self, item_glint: Option<bool>) -> bool {
        let own = self
            .data()
            .and_then(|data| data.other("minecraft:enchantment_glint_override"))
            .and_then(component_bool);
        let removed = self.data().is_some_and(|data| {
            data.other("!minecraft:enchantment_glint_override")
                .is_some()
        });
        let override_glint = own.or(if removed { None } else { item_glint });
        let item_foil = crate::properties::item_foil(
            &self.item,
            self.data()
                .is_some_and(|data| data.other("minecraft:lodestone_tracker").is_some()),
            !self.enchantments().is_empty(),
        );
        crate::properties::foil(override_glint, item_foil)
    }
    pub fn repair_cost(&self) -> u32 {
        self.data().map_or(0, |d| d.repair_cost)
    }
    pub fn custom_name(&self) -> Option<String> {
        self.data()?.custom_name_plain()
    }
    /// Non-empty slots of a `minecraft:container` component.
    pub fn container(&self) -> &[(u8, ItemStack)] {
        self.data().map_or(&[], |d| &d.container)
    }
    /// The `minecraft:custom_data` compound if present.
    pub fn custom_data(&self) -> Option<&horizon_core::nbt::Compound> {
        match self.data()?.other("minecraft:custom_data")? {
            Tag::Compound(map) => Some(map),
            _ => None,
        }
    }
    /// `ResolvableProfile.name`: a bare profile string or its `name` field.
    pub fn profile_name(&self) -> Option<String> {
        let profile = self.data()?.other("minecraft:profile")?;
        string(profile).or_else(|| profile.get("name").and_then(string))
    }
    /// `minecraft:item_name` in the current language (`getItemName`'s
    /// component). It stays in the patch as loaded, so a translatable name
    /// saves and reaches clients unchanged; the ominous banner's is modelled
    /// by `ominous_banner` instead.
    pub fn item_name(&self) -> Option<String> {
        self.item_name_text().map(|name| name.plain())
    }
    /// `getItemName`'s component as text: the stack's own `minecraft:item_name`, else the
    /// item's default where it is not its description id's ([`default_names`]).
    pub fn item_name_text(&self) -> Option<horizon_client_core::text::Text> {
        match self.data().and_then(|d| d.other("minecraft:item_name")) {
            Some(name) => Some(horizon_client_core::component::name_text(name)),
            None => default_names::of(&self.item),
        }
    }
    pub fn potion(&self) -> Option<&str> {
        self.data().and_then(|d| d.potion.as_deref())
    }
    /// `PotionContents.customEffects` as saved: (id, duration, amplifier). Both
    /// default to 0 like `MobEffectInstance.CODEC`.
    pub fn custom_potion_effects(&self) -> Vec<(String, i32, i32)> {
        let Some((_, Tag::List(list))) = self
            .data()
            .and_then(|d| d.potion_extra.iter().find(|(k, _)| k == "custom_effects"))
        else {
            return Vec::new();
        };
        list.iter()
            .filter_map(|e| {
                Some((
                    string(e.get("id")?)?,
                    e.get("duration").and_then(int).unwrap_or(0) as i32,
                    e.get("amplifier").and_then(int).unwrap_or(0).clamp(0, 255) as i32,
                ))
            })
            .collect()
    }
    /// `getOrDefault(DataComponents.POTION_DURATION_SCALE, 1.0F)` with the
    /// item's prototype default: 0.125 for a tipped arrow, 0.25 for a
    /// lingering potion, else 1. A removal patch leaves the lookup empty and
    /// the call's own 1.0 default applies.
    pub fn potion_duration_scale(&self) -> f32 {
        if self
            .data()
            .is_some_and(|d| d.other("!minecraft:potion_duration_scale").is_some())
        {
            return 1.0;
        }
        match self
            .data()
            .and_then(|d| d.other("minecraft:potion_duration_scale"))
        {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            Some(Tag::Double(bits)) => f64::from_bits(*bits) as f32,
            Some(t) if int(t).is_some() => int(t).unwrap_or(1) as f32,
            _ => match &*self.item {
                "minecraft:tipped_arrow" => 0.125,
                "minecraft:lingering_potion" => 0.25,
                _ => 1.0,
            },
        }
    }
    pub fn with_potion(mut self, potion: &str) -> Self {
        self.data_mut().potion = Some(Arc::from(potion));
        self
    }
    /// `ItemStack.isSameItemSameComponents`.
    pub fn same_item_same_components(&self, other: &Self) -> bool {
        self.item == other.item && self.damage == other.damage && self.data() == other.data()
    }
    /// Hash candidates for `same_item_same_components`. Confirm a match with
    /// that comparison, since fingerprints can collide.
    pub fn hash_item_and_components(&self, state: &mut impl Hasher) {
        self.item.hash(state);
        self.damage.hash(state);
        self.extra.hash(state);
    }

    /// `ItemStack.matches`: the same item, count and components.
    pub fn matches(&self, other: &Self) -> bool {
        self.count == other.count && self.same_item_same_components(other)
    }
    /// Check that a reloaded stack retains this stack's persistent data.
    pub fn matches_saved(&self, saved: &Self) -> bool {
        if self.count != saved.count || self.damage != saved.damage {
            return false;
        }
        if self.same_item_same_components(saved) {
            return true;
        }
        let mut planned = self.clone();
        let mut saved = saved.clone();
        normalize_saved_stack(&mut planned);
        normalize_saved_stack(&mut saved);
        planned.same_item_same_components(&saved)
    }

    /// `getOrDefault(DataComponents.ATTRIBUTE_MODIFIERS)`: the stack's own
    /// `attribute_modifiers` component when it has one (none at all when it
    /// removed the component), else its item type's. Borrowed unless the stack
    /// has its own list.
    pub fn default_attribute_modifiers<'a>(
        &self,
        items: &'a crate::items::Items,
    ) -> std::borrow::Cow<'a, [ModifierRow]> {
        self.default_attribute_modifiers_or(items.get(&self.item).map_or(&[], |i| &i.modifiers))
    }
    /// `default_attribute_modifiers` with the item type's list given: `default`.
    pub fn default_attribute_modifiers_or<'a>(
        &self,
        default: &'a [ModifierRow],
    ) -> std::borrow::Cow<'a, [ModifierRow]> {
        use std::borrow::Cow;
        if let Some(data) = self.data() {
            if data.other("!minecraft:attribute_modifiers").is_some() {
                return Cow::Borrowed(&[]);
            }
            if let Some(tag) = data.other("minecraft:attribute_modifiers") {
                return Cow::Owned(modifier_rows(tag));
            }
        }
        Cow::Borrowed(default)
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
/// `ItemEnchantments.getLevel`: `id`'s level in `list` (0 when absent).
fn level_in(list: &[(Arc<str>, u32)], id: &str) -> u32 {
    list.iter().find(|(e, _)| &**e == id).map_or(0, |(_, l)| *l)
}
/// `ItemEnchantments.Mutable.upgrade`.
pub fn upgrade(list: &mut Levels, id: &str, level: u32) {
    // ItemEnchantments.Mutable.upgrade: Math.min(level, 255).
    let level = level.min(255);
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
    // ItemEnchantments.Mutable.set: Math.min(level, 255).
    let level = level.min(255);
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
/// The persistent `ItemEnchantments` codec uses integer levels in 1..=255.
/// `TagValueInput.read` keeps the partial map when an entry fails decoding.
fn read_levels(t: Option<&Tag>) -> Levels {
    let Some(Tag::Compound(map)) = t else {
        return Vec::new();
    };
    // Pre-1.21.5 worlds wrap the map in {levels: {...}}.
    let map = match map.get(&JavaString::from("levels")) {
        Some(Tag::Compound(inner)) => inner,
        _ => map,
    };
    // The keys are enchantment Identifiers: `knockback` is `minecraft:knockback`.
    map.iter()
        .filter_map(|(k, v)| {
            let level = horizon_world::datafix::dynamic::as_number(v)?.int_value();
            if !(1..=255).contains(&level) {
                return None;
            }
            let id = horizon_world::datafix::dynamic::namespaced(&k.to_string_lossy());
            Some((Arc::from(id), level as u32))
        })
        .collect()
}
/// A text component as stored by vanilla: a plain string or `{"text": ...}`.
/// A `Filterable<String/Component>`: the value itself or `{raw: value}`.
/// `ComponentSerialization.CODEC`'s short form: a plain text component with
/// no style and no siblings is written as its string.
fn plain_component(t: &Tag) -> Tag {
    if let Tag::Compound(c) = t
        && let Some(text @ Tag::String(_)) = c.get(&"text".into())
        && c.iter().all(|(k, v)| {
            k.to_string_lossy() == "text"
                || (k.to_string_lossy() == "type"
                    && matches!(v, Tag::String(s) if s.to_string_lossy() == "text"))
        })
    {
        return text.clone();
    }
    t.clone()
}
/// `WrittenBookContent.CODEC`: the title and author as `Filterable` values,
/// the pages (each `{raw: component}`), the generation when it is not 0 and
/// the resolved flag when it is set.
fn written_book_tag(w: &WrittenBook) -> Tag {
    // Filterable values are saved as {raw: ..}.
    let raw = |t: Tag| Tag::compound([("raw", t)]);
    let pages = if w.page_tags.len() == w.pages.len() {
        w.page_tags.iter().cloned().map(raw).collect()
    } else {
        w.pages
            .iter()
            .map(|p| raw(Tag::String(p.as_str().into())))
            .collect()
    };
    let mut fields = vec![
        ("title", raw(Tag::String(w.title.as_str().into()))),
        ("author", Tag::String(w.author.as_str().into())),
    ];
    // `optionalFieldOf("pages", List.of())`: no pages, no field.
    if !w.pages.is_empty() {
        fields.push(("pages", Tag::List(pages)));
    }
    if w.generation != 0 {
        fields.push(("generation", Tag::Int(i32::from(w.generation))));
    }
    if w.resolved {
        fields.push(("resolved", Tag::Byte(1)));
    }
    Tag::compound(fields)
}

/// `WrittenBookContent` from its component: the title and author (a
/// `Filterable`'s raw text), the generation and the pages, whose component
/// tags are kept apart from their plain text for click events.
fn written_book_from_tag(b: &Tag) -> WrittenBook {
    WrittenBook {
        title: b.get("title").and_then(filterable_text).unwrap_or_default(),
        author: b.get("author").and_then(string).unwrap_or_default(),
        generation: b.get("generation").and_then(int).unwrap_or(0).clamp(0, 3) as u8,
        pages: match b.get("pages") {
            Some(Tag::List(pages)) => pages.iter().filter_map(filterable_text).collect(),
            _ => Vec::new(),
        },
        page_tags: match b.get("pages") {
            Some(Tag::List(pages)) => pages
                .iter()
                .map(|p| plain_component(p.get("raw").unwrap_or(p)))
                .collect(),
            _ => Vec::new(),
        },
        resolved: b.get("resolved").and_then(component_bool).unwrap_or(false),
    }
}

/// `WrittenBookContent.resolvePage`: the page's component with its selectors
/// and scores replaced by the text they name, recursing into `extra` and
/// `with`.
fn resolve_page_tag(tag: &Tag, text: &mut impl FnMut(&Tag) -> Option<String>) -> Tag {
    let Tag::Compound(c) = tag else {
        return tag.clone();
    };
    if let Some(resolved) = text(tag) {
        return Tag::compound([("text", Tag::String(resolved.as_str().into()))]);
    }
    let mut out = c.clone();
    for key in ["extra", "with"] {
        if let Some(Tag::List(list)) = out.get(&key.into()) {
            let list = list.iter().map(|t| resolve_page_tag(t, text)).collect();
            out.insert(key.into(), Tag::List(list));
        }
    }
    Tag::Compound(out)
}

impl WrittenBook {
    /// `WrittenBookContent.resolve` and `markResolved`: every page's
    /// selectors and scores become their text, and the book is marked
    /// resolved. A page whose resolved text passes `WrittenBookContent`'s
    /// 32767-character limit (`resolvePage`) cancels the resolution, and the
    /// book is only marked resolved. True when the pages were rewritten.
    pub fn resolve(&mut self, mut text: impl FnMut(&Tag) -> Option<String>) -> bool {
        if self.resolved {
            return false;
        }
        self.resolved = true;
        if self.page_tags.len() != self.pages.len() {
            return false;
        }
        let mut pages = Vec::with_capacity(self.pages.len());
        let mut tags = Vec::with_capacity(self.page_tags.len());
        for tag in &self.page_tags {
            let resolved = resolve_page_tag(tag, &mut text);
            let plain = horizon_client_core::component::name_text(&resolved).plain();
            if plain.chars().count() > 32767 {
                return false;
            }
            pages.push(plain);
            tags.push(resolved);
        }
        self.pages = pages;
        self.page_tags = tags;
        true
    }
}

fn filterable_text(t: &Tag) -> Option<String> {
    match t.get("raw") {
        Some(raw) => read_text(raw),
        None => read_text(t),
    }
}
fn read_text(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        // A component's root `text` may be empty with the visible words in
        // `extra` (every legacy display name ViaVersion produces looks like
        // this), so flatten the whole component instead of reading one key.
        // ComponentSerialization.CODEC also takes a list (its first element
        // styling the rest), as servers behind ViaVersion send book pages.
        Tag::Compound(_) | Tag::List(_) => {
            Some(horizon_client_core::component::name_text(t).plain())
        }
        _ => None,
    }
}

/// `Component.tryCollapseToString`: `ComponentSerialization.CODEC` writes
/// plain text with no style and no siblings as a bare string.
fn collapsed_text(tag: Tag) -> Tag {
    if let Tag::Compound(map) = &tag
        && let Some(Tag::String(text)) = tag.get("text")
        && map
            .iter()
            .all(|(key, value)| match key.to_string_lossy().as_str() {
                "text" => true,
                "type" => matches!(value, Tag::String(s) if s.to_string_lossy() == "text"),
                "extra" => matches!(value, Tag::List(l) if l.is_empty()),
                _ => false,
            })
    {
        return Tag::String(text.clone());
    }
    tag
}

/// A `FireworkExplosion` as vanilla's codec writes it (defaults omitted).
fn explosion_tag(e: &crate::firework::FireworkExplosion) -> Tag {
    let colors = |c: &[u32]| Tag::List(c.iter().map(|c| Tag::Int(*c as i32)).collect());
    let mut fields = vec![("shape", Tag::String(e.shape.name().into()))];
    if !e.colors.is_empty() {
        fields.push(("colors", colors(&e.colors)));
    }
    if !e.fade.is_empty() {
        fields.push(("fade_colors", colors(&e.fade)));
    }
    if e.trail {
        fields.push(("has_trail", Tag::Byte(1)));
    }
    if e.twinkle {
        fields.push(("has_twinkle", Tag::Byte(1)));
    }
    Tag::compound(fields)
}
fn explosion_from(e: &Tag) -> crate::firework::FireworkExplosion {
    let ints = |t: Option<&Tag>| match t {
        Some(Tag::IntArray(v)) => v.iter().map(|c| *c as u32).collect(),
        Some(Tag::ByteArray(v)) => v.iter().map(|&c| i32::from(c as i8) as u32).collect(),
        Some(Tag::LongArray(v)) => v.iter().map(|&c| c as i32 as u32).collect(),
        Some(Tag::List(v)) => v
            .iter()
            .filter_map(crate::properties::component_int)
            .map(|c| c as u32)
            .collect(),
        _ => Vec::new(),
    };
    crate::firework::FireworkExplosion {
        shape: crate::firework::FireworkShape::from_name(
            e.get("shape")
                .and_then(string)
                .as_deref()
                .map_or("small_ball", |s| s.strip_prefix("minecraft:").unwrap_or(s)),
        ),
        colors: ints(e.get("colors")),
        fade: ints(e.get("fade_colors")),
        trail: e.get("has_trail").and_then(component_bool).unwrap_or(false),
        twinkle: e
            .get("has_twinkle")
            .and_then(component_bool)
            .unwrap_or(false),
    }
}
/// An `ItemStackTemplate`: `{id, count?, components?}`.
fn template_tag(stack: &ItemStack) -> Tag {
    let mut entries = vec![("id", Tag::String((&*stack.item).into()))];
    if stack.count != 1 {
        entries.push(("count", Tag::Int(stack.count as i32)));
    }
    if let Some(components) = encode_components(stack) {
        entries.push(("components", components));
    }
    Tag::compound(entries)
}
/// The pot prototype's decorations: a brick on every side.
pub fn all_bricks(sides: &[Option<ItemStack>; 4]) -> bool {
    sides.iter().all(|s| {
        s.as_ref()
            .is_some_and(|s| &*s.item == "minecraft:brick" && s.count == 1 && s.extra.is_none())
    })
}
fn template_from(tag: &Tag) -> Option<ItemStack> {
    use horizon_world::datafix::dynamic::namespaced;

    match tag {
        Tag::String(id) => Some(ItemStack::new(&namespaced(&id.to_string_lossy()), 1)),
        _ => {
            let id = namespaced(&tag.get("id").and_then(string)?);
            // `ItemStackTemplate.CODEC`: `count` is `ExtraCodecs.intRange(1, 99)`, so a
            // template outside it does not decode.
            let count = tag.get("count").and_then(int).unwrap_or(1);
            if !(1..=99).contains(&count) {
                return None;
            }
            let (damage, extra) = decode_components_for_item(&id, tag.get("components"));
            Some(ItemStack {
                item: Arc::from(id.as_str()),
                count: count as u32,
                damage,
                extra,
            })
        }
    }
}
/// The stack a `show_item` hover event holds (`HoverEvent.ShowItem`): its
/// `{id, count?, components?}` object.
pub fn stack_from_hover(event: &serde_json::Value) -> Option<ItemStack> {
    template_from(&horizon_client_core::scoreboard::json_to_tag(event))
}
/// Sorted-key, type-annotated text of a tag (the differential harnesses
/// print the same form from the vanilla side).
pub fn canon_tag(tag: &Tag) -> String {
    fn list(out: &mut String, prefix: &str, items: impl Iterator<Item = String>) {
        out.push_str(prefix);
        out.push_str(&items.collect::<Vec<_>>().join(","));
        out.push(']');
    }
    fn go(tag: &Tag, out: &mut String) {
        use std::fmt::Write as _;
        match tag {
            Tag::End => {}
            Tag::Byte(v) => write!(out, "{v}b").unwrap(),
            Tag::Short(v) => write!(out, "{v}s").unwrap(),
            Tag::Int(v) => write!(out, "{v}").unwrap(),
            Tag::Long(v) => write!(out, "{v}L").unwrap(),
            Tag::Float(v) => write!(out, "{}f", f32::from_bits(*v)).unwrap(),
            Tag::Double(v) => write!(out, "{}d", f64::from_bits(*v)).unwrap(),
            Tag::String(s) => write!(
                out,
                "\"{}\"",
                s.to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
            )
            .unwrap(),
            Tag::ByteArray(v) => list(out, "[B;", v.iter().map(|b| (*b as i8).to_string())),
            Tag::IntArray(v) => list(out, "[I;", v.iter().map(i32::to_string)),
            Tag::LongArray(v) => list(out, "[L;", v.iter().map(i64::to_string)),
            Tag::List(items) => {
                out.push('[');
                for (i, t) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    go(t, out);
                }
                out.push(']');
            }
            Tag::Compound(map) => {
                let mut keys: Vec<(String, &Tag)> =
                    map.iter().map(|(k, v)| (k.to_string_lossy(), v)).collect();
                keys.sort_by(|a, b| a.0.cmp(&b.0));
                out.push('{');
                for (i, (k, v)) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&k);
                    out.push(':');
                    go(v, out);
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    go(tag, &mut out);
    out
}
/// `canon_tag` of a stack as `ItemStack.CODEC` writes it; "-" for none.
pub fn canon_stack(stack: Option<&ItemStack>) -> String {
    stack.map_or_else(
        || "-".to_owned(),
        |s| canon_tag(&crate::item_nbt::item_to_nbt(s)),
    )
}
/// The stack of an SNBT `ItemStack.CODEC` compound; None for "-".
pub fn parse_stack(snbt: &str) -> Result<Option<ItemStack>, String> {
    if snbt == "-" {
        return Ok(None);
    }
    let (tag, _) = horizon_core::nbt::parse_snbt(snbt)?;
    Ok(crate::item_nbt::item_from_nbt(&tag))
}
/// `parse_stack` against the loaded item prototypes, as `ItemStack.CODEC` builds it.
pub fn parse_stack_with_items(
    snbt: &str,
    items: &crate::items::Items,
) -> Result<Option<ItemStack>, String> {
    if snbt == "-" {
        return Ok(None);
    }
    let (tag, _) = horizon_core::nbt::parse_snbt(snbt)?;
    Ok(crate::item_nbt::item_from_nbt_with_items(&tag, items))
}
/// `MapDecorations.Entry.CODEC`: a bare type ID uses the default resource namespace.
/// Keep preserved entry keys and numeric tags intact; this is not registry validation.
fn canonical_other_component(id: &str, tag: &Tag) -> Tag {
    let mut value = tag.clone();
    if id == "minecraft:map_decorations"
        && let Tag::Compound(entries) = &mut value
    {
        for entry in entries.values_mut() {
            if let Some(Tag::String(kind)) = entry.get_mut("type") {
                let text = kind.to_string_lossy();
                if !text.is_empty()
                    && !text.contains(':')
                    && let Ok(id) = horizon_core::identifier::Identifier::parse(&text)
                {
                    *kind = id.to_string().as_str().into();
                }
            }
        }
    }
    value
}
/// The saved `DataComponentPatch.CODEC` compound, or None when default.
/// A filled map's empty decorations equal its prototype (`Items.mapProperties`).
pub fn encode_components(stack: &ItemStack) -> Option<Tag> {
    encode_stack_components(stack, true)
}

/// Encode the extra patch for a snapshot whose typed scalar already carries damage.
/// This uses the same component codec and canonicalization as persistent stacks.
pub fn encode_extra_components(stack: &ItemStack) -> Option<Tag> {
    encode_stack_components(stack, false)
}

fn encode_stack_components(stack: &ItemStack, include_damage: bool) -> Option<Tag> {
    let mut components = components_tag_inner(stack.damage, stack.data(), include_damage)?;
    if let Some(Tag::List(effects)) = components
        .get_mut("minecraft:potion_contents")
        .and_then(|potion| potion.get_mut("custom_effects"))
    {
        canonical_effect_icons(effects);
    }
    if let Some(tracker) = components.get_mut("minecraft:lodestone_tracker") {
        canonical_lodestone_tracker(tracker);
    }
    if let Some(key) = elided_prototype_key(&stack.item)
        && components
            .get(key)
            .is_some_and(|value| is_prototype_patch(key, value))
        && let Tag::Compound(entries) = &mut components
    {
        entries.remove(&JavaString::from(key));
        if entries.is_empty() {
            return None;
        }
    }
    Some(components)
}

/// The icon follows show_particles when omitted. Its default is true.
fn canonical_effect_icons(effects: &mut [Tag]) {
    for effect in effects {
        let shown = effect.get("show_particles").and_then(int).unwrap_or(1) != 0;
        if let Tag::Compound(fields) = effect {
            fields
                .entry(JavaString::from("show_icon"))
                .or_insert(Tag::Byte(i8::from(shown)));
        }
    }
}

/// LodestoneTracker.CODEC uses true when tracked is omitted.
fn canonical_lodestone_tracker(tracker: &mut Tag) {
    if let Tag::Compound(fields) = tracker {
        let tracked = JavaString::from("tracked");
        if fields.get(&tracked) == Some(&Tag::Byte(1)) {
            fields.remove(&tracked);
        }
    }
}

// These saved patches are redundant for their vanilla item prototypes.
fn elided_prototype_key(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:filled_map" => Some("minecraft:map_decorations"),
        "minecraft:enchanted_book" => Some("minecraft:stored_enchantments"),
        "minecraft:writable_book" => Some("minecraft:writable_book_content"),
        "minecraft:beehive" | "minecraft:bee_nest" => Some("minecraft:block_state"),
        _ if has_empty_banner_patterns_prototype(item) => Some("minecraft:banner_patterns"),
        _ => None,
    }
}

/// `Items.BANNER` and `Items.SHIELD` both have `BannerPatternLayers.EMPTY`.
fn has_empty_banner_patterns_prototype(item: &str) -> bool {
    item == "minecraft:shield"
        || item
            .strip_prefix("minecraft:")
            .and_then(|id| id.strip_suffix("_banner"))
            .is_some_and(|color| horizon_mod_package::content::DYE_COLORS.contains(&color))
}

fn is_prototype_patch(key: &str, value: &Tag) -> bool {
    if key == "minecraft:banner_patterns" {
        return matches!(value, Tag::List(layers) if layers.is_empty());
    }
    let Tag::Compound(entries) = value else {
        return false;
    };
    // Items.WRITABLE_BOOK starts with WritableBookContent.EMPTY. Its codec
    // defaults omitted pages to an empty list.
    if key == "minecraft:writable_book_content" {
        return entries.is_empty()
            || (entries.len() == 1
                && matches!(value.get("pages"), Some(Tag::List(pages)) if pages.is_empty()));
    }
    if key == "minecraft:block_state" {
        entries.len() == 1
            && matches!(value.get("honey_level"), Some(Tag::String(level)) if level.to_string_lossy() == "0")
    } else {
        entries.is_empty()
    }
}

/// Normalize equivalent representations directly so a lossy codec cannot hide a change.
fn normalize_saved_stack(stack: &mut ItemStack) {
    let canonical_id = |id: &mut Arc<str>| {
        if let Ok(parsed) = horizon_core::identifier::Identifier::parse(id) {
            *id = parsed.to_string().into();
        }
    };
    canonical_id(&mut stack.item);
    let Some(data) = stack.extra.as_deref_mut() else {
        return;
    };
    data.wire = Wire::default();
    data.map_post_processing = None;
    data.custom_name = data.custom_name.take().map(collapsed_text);
    for (id, _) in data
        .enchantments
        .iter_mut()
        .chain(&mut data.stored_enchantments)
    {
        canonical_id(id);
    }
    for (key, value) in &mut data.potion_extra {
        if key == "custom_effects"
            && let Tag::List(effects) = value
        {
            canonical_effect_icons(effects);
        }
    }
    data.potion_extra.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(book) = &mut data.written {
        if book.page_tags.is_empty() {
            book.page_tags = book
                .pages
                .iter()
                .map(|page| Tag::String(page.as_str().into()))
                .collect();
        } else {
            book.page_tags = book.page_tags.iter().map(plain_component).collect();
        }
    }
    elide_prototype_fireworks(&stack.item, data);
    if data.pot_decorations.as_deref().is_some_and(all_bricks) {
        data.pot_decorations = None;
    }
    for nested in data
        .container
        .iter_mut()
        .map(|(_, stack)| stack)
        .chain(&mut data.charged)
        .chain(&mut data.bundle_contents)
        .chain(
            data.pot_decorations
                .iter_mut()
                .flat_map(|sides| sides.iter_mut().flatten()),
        )
    {
        normalize_saved_stack(nested);
    }
    if let Some(key) = elided_prototype_key(&stack.item) {
        data.other
            .retain(|(id, value)| id != key || !is_prototype_patch(key, value));
    }
    for (key, value) in &mut data.other {
        match key.as_str() {
            "minecraft:map_decorations" => *value = canonical_other_component(key, value),
            "minecraft:lodestone_tracker" => canonical_lodestone_tracker(value),
            _ => crate::item_nbt::canonicalize_component_for_comparison(key, value),
        }
    }
    data.other.sort_by(|a, b| a.0.cmp(&b.0));
    stack.tidy();
}

/// Saved patch with the loaded item's prototype available.
/// Context-free callers and the live `components_tag` view stay unchanged.
pub fn encode_components_with_items(stack: &ItemStack, items: &crate::items::Items) -> Option<Tag> {
    let mut components = encode_components(stack)?;
    let Some(prototype) = items.get(&stack.item) else {
        return Some(components);
    };
    // PatchedDataComponentMap.set removes values equal to the prototype.
    let same_stack_size = prototype
        .components
        .iter()
        .any(|id| id == "minecraft:max_stack_size")
        && matches!(components.get("minecraft:max_stack_size"), Some(Tag::Int(size))
            if *size > 0 && *size as u32 == prototype.max_stack);
    // A durable prototype carries DAMAGE 0 (`Item.Properties.durability`), so the
    // explicit zero `components_tag` keeps for a stack-declared MAX_DAMAGE equals it.
    let same_damage =
        prototype.max_damage > 0 && matches!(components.get("minecraft:damage"), Some(Tag::Int(0)));
    if let Tag::Compound(entries) = &mut components {
        if same_stack_size {
            entries.remove(&JavaString::from("minecraft:max_stack_size"));
        }
        if same_damage {
            entries.remove(&JavaString::from("minecraft:damage"));
        }
        if entries.is_empty() {
            return None;
        }
    }
    Some(components)
}
/// `encode_components` from the damage value and extra components.
pub fn components_tag(damage: u32, data: Option<&StackData>) -> Option<Tag> {
    components_tag_inner(damage, data, true)
}

fn components_tag_inner(
    damage: u32,
    data: Option<&StackData>,
    include_damage: bool,
) -> Option<Tag> {
    let mut entries: Vec<(&str, Tag)> = Vec::new();
    // Explicit damage patches survive even at zero; a custom max damage can
    // also make its saved zero meaningful.
    if include_damage
        && (damage > 0
            || data.is_some_and(|data| {
                data.damage_set
                    || (data.explicit_zero_damage && data.other("minecraft:max_damage").is_some())
            }))
    {
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
            entries.push(("minecraft:custom_name", name.clone()));
        }
        if let Some(consumable) = &d.consumable {
            entries.push(("minecraft:consumable", consumable.raw.clone()));
        }
        if let Some(kinetic) = &d.kinetic_weapon {
            entries.push(("minecraft:kinetic_weapon", kinetic.raw().clone()));
        }
        if d.potion.is_some() || !d.potion_extra.is_empty() {
            let potion = d
                .potion
                .as_ref()
                .map(|p| ("potion", Tag::String((&**p).into())));
            let extra = d.potion_extra.iter().map(|(k, v)| (k.as_str(), v.clone()));
            entries.push((
                "minecraft:potion_contents",
                Tag::compound(potion.into_iter().chain(extra)),
            ));
        }
        if let Some(trim) = &d.trim {
            entries.push(("minecraft:trim", trim::to_nbt(trim)));
        }
        if let Some(id) = d.map_id {
            entries.push(("minecraft:map_id", Tag::Int(id)));
        }
        if !d.banner_patterns.is_empty() {
            entries.push((
                "minecraft:banner_patterns",
                crate::banner_layer::layers_to_nbt(&d.banner_patterns),
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
            entries.push(("minecraft:written_book_content", written_book_tag(w)));
        }
        if let Some((flight, explosions)) = &d.fireworks {
            let mut fields = Vec::new();
            if *flight != 0 {
                fields.push(("flight_duration", Tag::Byte(*flight as i8)));
            }
            if !explosions.is_empty() {
                fields.push((
                    "explosions",
                    Tag::List(explosions.iter().map(explosion_tag).collect()),
                ));
            }
            entries.push(("minecraft:fireworks", Tag::compound(fields)));
        }
        if let Some(e) = &d.firework_explosion {
            entries.push(("minecraft:firework_explosion", explosion_tag(e)));
        }
        if let Some(color) = d.dyed_color {
            entries.push(("minecraft:dyed_color", Tag::Int(color)));
        }
        if let Some(sides) = &d.pot_decorations {
            let mut sherds = Vec::new();
            for (name, side) in ["back", "left", "right", "front"].iter().zip(sides.iter()) {
                if let Some(stack) = side {
                    sherds.push((*name, template_tag(stack)));
                }
            }
            entries.push(("minecraft:pot_decorations", Tag::compound(sherds)));
        }
        if !d.bundle_contents.is_empty() {
            entries.push((
                "minecraft:bundle_contents",
                Tag::List(d.bundle_contents.iter().map(template_tag).collect()),
            ));
        }
        if !d.stew_effects.is_empty() {
            entries.push((
                "minecraft:suspicious_stew_effects",
                Tag::List(
                    d.stew_effects
                        .iter()
                        .map(|(id, duration)| {
                            // The codec leaves out the default duration (160).
                            let mut e = vec![("id", Tag::String((&**id).into()))];
                            if *duration != 160 {
                                e.push(("duration", Tag::Int(*duration)));
                            }
                            Tag::compound(e)
                        })
                        .collect(),
                ),
            ));
        }
        // minecraft:map_post_processing is transient: ItemStack.CODEC never
        // saves it (only the network carries it).
        if !d.charged.is_empty() {
            entries.push((
                "minecraft:charged_projectiles",
                Tag::List(d.charged.iter().map(template_tag).collect()),
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
            if d.other("minecraft:tooltip_display").is_none() {
                entries.push(("minecraft:tooltip_display", ominous_tooltip_display()));
            }
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
                        ("item", template_tag(stack)),
                    ])
                })
                .collect();
            entries.push(("minecraft:container", Tag::List(slots)));
        }
    }
    let mut out: horizon_core::nbt::Compound = entries
        .into_iter()
        .map(|(k, v)| (JavaString::from(k), v))
        .collect();
    if let Some(d) = data {
        for (k, v) in &d.other {
            out.entry(JavaString::from(k.as_str()))
                .or_insert_with(|| canonical_other_component(k, v));
        }
    }
    (!out.is_empty()).then_some(Tag::Compound(out))
}
/// The ASCII bytes of an NBT key, in `buf`. Every component ID is ASCII; a key
/// with any other unit cannot be one of them and needs no comparison.
fn ascii_key<'a>(key: &JavaString, buf: &'a mut [u8; 64]) -> Option<&'a str> {
    let units = &key.0;
    if units.len() > buf.len() {
        return None;
    }
    for (slot, &unit) in buf.iter_mut().zip(units) {
        if unit > 0x7f {
            return None;
        }
        *slot = unit as u8;
    }
    std::str::from_utf8(&buf[..units.len()]).ok()
}
/// The saved tags of the component IDs `decode_components` models, found in one
/// pass over the compound. Reading them one at a time built a key and walked the
/// tree for every ID, present or not, and most items carry one or two.
#[derive(Default)]
struct Seen<'a> {
    damage: Option<&'a Tag>,
    potion_contents: Option<&'a Tag>,
    enchantments: Option<&'a Tag>,
    stored_enchantments: Option<&'a Tag>,
    repair_cost: Option<&'a Tag>,
    custom_name: Option<&'a Tag>,
    map_id: Option<&'a Tag>,
    banner_patterns: Option<&'a Tag>,
    base_color: Option<&'a Tag>,
    writable_book_content: Option<&'a Tag>,
    written_book_content: Option<&'a Tag>,
    fireworks: Option<&'a Tag>,
    firework_explosion: Option<&'a Tag>,
    dyed_color: Option<&'a Tag>,
    pot_decorations: Option<&'a Tag>,
    bundle_contents: Option<&'a Tag>,
    suspicious_stew: Option<&'a Tag>,
    map_post_processing: Option<&'a Tag>,
    charged_projectiles: Option<&'a Tag>,
    container: Option<&'a Tag>,
    item_name: Option<&'a Tag>,
    amplifier: Option<&'a Tag>,
    trim: Option<&'a Tag>,
}
impl<'a> Seen<'a> {
    fn read(c: &'a Tag) -> Self {
        let Tag::Compound(map) = c else {
            return Self::default();
        };
        let mut seen = Self::default();
        let mut buf = [0u8; 64];
        for (key, value) in map {
            match ascii_key(key, &mut buf) {
                Some("minecraft:damage") => seen.damage = Some(value),
                Some("minecraft:potion_contents") => seen.potion_contents = Some(value),
                Some("minecraft:enchantments") => seen.enchantments = Some(value),
                Some("minecraft:stored_enchantments") => seen.stored_enchantments = Some(value),
                Some("minecraft:repair_cost") => seen.repair_cost = Some(value),
                Some("minecraft:custom_name") => seen.custom_name = Some(value),
                Some("minecraft:map_id") => seen.map_id = Some(value),
                Some("minecraft:banner_patterns") => seen.banner_patterns = Some(value),
                Some("minecraft:base_color") => seen.base_color = Some(value),
                Some("minecraft:writable_book_content") => seen.writable_book_content = Some(value),
                Some("minecraft:written_book_content") => seen.written_book_content = Some(value),
                Some("minecraft:fireworks") => seen.fireworks = Some(value),
                Some("minecraft:firework_explosion") => seen.firework_explosion = Some(value),
                Some("minecraft:dyed_color") => seen.dyed_color = Some(value),
                Some("minecraft:pot_decorations") => seen.pot_decorations = Some(value),
                Some("minecraft:bundle_contents") => seen.bundle_contents = Some(value),
                Some("minecraft:suspicious_stew_effects") => seen.suspicious_stew = Some(value),
                Some("minecraft:map_post_processing") => seen.map_post_processing = Some(value),
                Some("minecraft:charged_projectiles") => seen.charged_projectiles = Some(value),
                Some("minecraft:container") => seen.container = Some(value),
                Some("minecraft:item_name") => seen.item_name = Some(value),
                Some("minecraft:ominous_bottle_amplifier") => seen.amplifier = Some(value),
                Some("minecraft:trim") => seen.trim = Some(value),
                _ => {}
            }
        }
        seen
    }
}
/// Damage and extra components from a stack's `components` compound.
pub fn decode_components(components: Option<&Tag>) -> (u32, Option<Box<StackData>>) {
    decode_component_patch(components, None)
}

fn decode_component_patch(
    components: Option<&Tag>,
    item: Option<&str>,
) -> (u32, Option<Box<StackData>>) {
    let Some(c) = components else {
        return (0, None);
    };
    let seen = Seen::read(c);
    // NbtOps.getNumberValue supplies a boxed number; Codec.INT truncates its value.
    let damage = horizon_world::datafix::dynamic::as_int(seen.damage, 0).max(0) as u32;
    let potion = match seen.potion_contents {
        Some(Tag::String(s)) => Some(Arc::from(s.to_string_lossy().as_str())),
        Some(t) => t
            .get("potion")
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        None => None,
    };
    let potion_extra = match seen.potion_contents {
        Some(Tag::Compound(map)) => map
            .iter()
            .filter(|(k, _)| k.to_string_lossy() != "potion")
            .map(|(k, v)| (k.to_string_lossy(), v.clone()))
            .collect(),
        _ => Vec::new(),
    };
    let mut data = StackData {
        damage_set: damage == 0 && c.get("minecraft:damage").is_some(),
        enchantments: read_levels(c.get("minecraft:enchantments")),
        stored_enchantments: read_levels(c.get("minecraft:stored_enchantments")),
        repair_cost: c
            .get("minecraft:repair_cost")
            .and_then(int)
            .unwrap_or(0)
            .max(0) as u32,
        explicit_zero_damage: damage == 0 && c.get("minecraft:damage").is_some(),
        custom_name: c
            .get("minecraft:custom_name")
            .filter(|tag| read_text(tag).is_some())
            .cloned()
            .map(collapsed_text),
        potion,
        potion_extra,
        death_protection: c
            .get("minecraft:death_protection")
            .and_then(|raw| {
                crate::items::parse_death_effects(&horizon_client_core::scoreboard::tag_to_json(
                    raw,
                ))
                .ok()
            })
            .map(Arc::new),
        consumable: c
            .get("minecraft:consumable")
            .cloned()
            .and_then(ConsumableComponent::new),
        kinetic_weapon: c
            .get("minecraft:kinetic_weapon")
            .cloned()
            .map(crate::kinetic_weapon::KineticComponent::new),
        map_id: seen.map_id.and_then(crate::properties::component_int),
        banner_patterns: crate::banner_layer::layers_from_nbt(c.get("minecraft:banner_patterns")),
        base_color: c
            .get("minecraft:base_color")
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        writable_pages: match seen.writable_book_content.and_then(|b| b.get("pages")) {
            Some(Tag::List(pages)) => pages.iter().filter_map(filterable_text).collect(),
            _ => Vec::new(),
        },
        written: seen.written_book_content.map(written_book_from_tag),
        fireworks: seen.fireworks.map(|f| {
            let flight = f
                .get("flight_duration")
                .and_then(int)
                .unwrap_or(0)
                .clamp(0, 255) as u8;
            let explosions = match f.get("explosions") {
                Some(Tag::List(list)) => list.iter().map(explosion_from).collect(),
                _ => Vec::new(),
            };
            (flight, explosions)
        }),
        firework_explosion: seen.firework_explosion.map(explosion_from),
        dyed_color: seen
            .dyed_color
            .and_then(horizon_core::color::rgb_color_from_tag),
        pot_decorations: seen
            .pot_decorations
            .map(|p| {
                let side = |name: &str| p.get(name).and_then(template_from);
                Box::new([side("back"), side("left"), side("right"), side("front")])
            })
            .filter(|sides| !all_bricks(sides)),
        bundle_contents: match seen.bundle_contents {
            Some(Tag::List(list)) => list.iter().filter_map(template_from).collect(),
            _ => Vec::new(),
        },
        stew_effects: match seen.suspicious_stew {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    Some((
                        Arc::from(e.get("id").and_then(string)?.as_str()),
                        e.get("duration").and_then(int).unwrap_or(160) as i32,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        },
        map_post_processing: match seen.map_post_processing {
            Some(Tag::String(s)) => Some(u8::from(s.to_string_lossy() != "lock")),
            Some(t) => int(t).map(|v| v.clamp(0, 1) as u8),
            None => None,
        },
        charged: match seen.charged_projectiles {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(crate::item_nbt::item_from_nbt)
                .collect(),
            _ => Vec::new(),
        },
        container: match seen.container {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    Some((
                        e.get("slot").and_then(int)?.clamp(0, 255) as u8,
                        template_from(e.get("item")?)?,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        },
        ominous_banner: seen
            .item_name
            .and_then(|n| n.get("translate"))
            .and_then(string)
            .as_deref()
            == Some("block.minecraft.ominous_banner")
            && matches!(
                seen.banner_patterns,
                Some(Tag::List(layers)) if layers.len() == OMINOUS_LAYERS.len()),
        ominous_amplifier: seen
            .amplifier
            .and_then(crate::properties::component_int)
            .filter(|v| (0..=4).contains(v))
            .map(|v| v as u8),
        trim: seen.trim.and_then(trim::from_nbt),
        bundle_selected: BundleSelected::default(),
        other: Vec::new(),
        wire: Wire::default(),
    };
    if let Tag::Compound(map) = c {
        for (k, v) in map {
            let key = k.to_string_lossy();
            // These modelled components can use a raw marker for an explicit
            // empty value. Do not keep one for the item's own prototype (or
            // confuse it with a `!` removal).
            if matches!(
                key.as_str(),
                "minecraft:banner_patterns" | "minecraft:writable_book_content"
            ) && item.and_then(elided_prototype_key) == Some(key.as_str())
                && is_prototype_patch(&key, v)
            {
                continue;
            }
            if MODELLED.contains(&key.as_str())
                && key != "minecraft:damage"
                && !data.sets_component(&key)
            {
                data.other
                    .push((key.clone(), canonical_other_component(&key, v)));
            }
            let ominous_part = data.ominous_banner
                && (matches!(key.as_str(), "minecraft:item_name" | "minecraft:rarity")
                    || (key == "minecraft:tooltip_display" && v == &ominous_tooltip_display()));
            // An explicitly empty stored_enchantments still counts as present
            // (`ItemStack.has`); the list is modelled, so keep a marker.
            if key == "minecraft:stored_enchantments"
                && data.stored_enchantments.is_empty()
                && matches!(v, Tag::Compound(_))
            {
                data.other
                    .push((key, Tag::compound(Vec::<(&str, Tag)>::new())));
            } else if !MODELLED.contains(&key.as_str()) && !ominous_part {
                let value = canonical_other_component(&key, v);
                data.other.push((key, value));
            }
        }
        data.other.sort_by(|a, b| a.0.cmp(&b.0));
    }
    (damage, (!data.is_empty()).then(|| Box::new(data)))
}

/// `decode_components` with the item id known, so a component value equal to
/// the item's prototype is left out the way `PatchedDataComponentMap.applyPatch`
/// leaves it out of the patch.
pub fn decode_components_for_item(
    item: &str,
    components: Option<&Tag>,
) -> (u32, Option<Box<StackData>>) {
    let (damage, mut extra) = decode_component_patch(components, Some(item));
    if let Some(data) = extra.as_deref_mut() {
        elide_prototype_fireworks(item, data);
        if data.is_empty() {
            extra = None;
        }
    }
    (damage, extra)
}

/// `PatchedDataComponentMap.applyPatch` and `set`: a component whose value
/// equals the item's `prototype` (`Items::component_defaults`) is no part of
/// the stack's patch, nor is the removal of a component it lacks. Differing
/// values and removals of its own components stay. Call this where a
/// stack is built with the item registry at hand. The context-free codecs
/// have no prototype to compare against.
pub fn drop_prototype_values(stack: &mut ItemStack, prototype: &horizon_core::nbt::Compound) {
    let Some(Tag::Compound(mut patch)) = components_tag(stack.damage, stack.data()) else {
        return;
    };
    let before = patch.len();
    patch.retain(|key, value| keep_patch_entry(key, value, Some(prototype)));
    if patch.len() == before {
        return;
    }
    let old = stack.extra.take();
    let patch = (!patch.is_empty()).then_some(Tag::Compound(patch));
    (stack.damage, stack.extra) = decode_components_for_item(&stack.item, patch.as_ref());
    // Runtime-only state is not part of the saved patch.
    if let Some(old) = old
        && (old.map_post_processing.is_some()
            || old.bundle_selected.0 != BundleSelected::default().0)
    {
        let data = stack.data_mut();
        data.map_post_processing = old.map_post_processing;
        data.bundle_selected = old.bundle_selected;
    }
}

fn keep_patch_entry(
    key: &JavaString,
    value: &Tag,
    prototype: Option<&horizon_core::nbt::Compound>,
) -> bool {
    let id = key.to_string_lossy();
    if let Some(removed) = id.strip_prefix('!') {
        return prototype.is_some_and(|p| p.contains_key(&JavaString::from(removed)));
    }
    prototype
        .and_then(|p| p.get(key))
        .is_none_or(|default| !same_component_value(&id, value, default))
}

/// `ItemStack.applyComponentsAndValidate`: commit a patch only when the
/// resulting components and the original count pass `validateStrict`.
pub fn apply_components_and_validate(
    stack: &mut ItemStack,
    patch: &horizon_core::nbt::Compound,
    items: &crate::items::Items,
) {
    let Ok(patch) = component_values::decode_live_patch(patch) else {
        return;
    };
    let prototype = items.component_defaults.get(&*stack.item);
    let mut merged = match components_tag(stack.damage, stack.data()) {
        Some(Tag::Compound(components)) => components,
        _ => Default::default(),
    };
    let replaces_bundle = patch.contains_key(&JavaString::from("minecraft:bundle_contents"))
        || patch.contains_key(&JavaString::from("!minecraft:bundle_contents"));
    let replaces_post_processing = patch
        .contains_key(&JavaString::from("minecraft:map_post_processing"))
        || patch.contains_key(&JavaString::from("!minecraft:map_post_processing"));
    for (key, value) in patch {
        let id = key.to_string_lossy();
        let opposite = match id.strip_prefix('!') {
            Some(id) => JavaString::from(id),
            None => JavaString::from(format!("!{id}").as_str()),
        };
        merged.remove(&opposite);
        if keep_patch_entry(&key, &value, prototype) {
            merged.insert(key, value);
        } else {
            merged.remove(&key);
        }
    }
    let tag = Tag::Compound(merged);
    let (damage, extra) = decode_components_for_item(&stack.item, Some(&tag));
    let mut candidate = ItemStack {
        item: stack.item.clone(),
        count: stack.count,
        damage,
        extra,
    };
    if let Some(old) = stack.data() {
        if let Some(post_processing) = old.map_post_processing
            && !replaces_post_processing
        {
            candidate.data_mut().map_post_processing = Some(post_processing);
        }
        if old.bundle_selected.0 != BundleSelected::default().0 && !replaces_bundle {
            candidate.data_mut().bundle_selected = old.bundle_selected;
        }
    }
    if validate_strict(&candidate, items) {
        *stack = candidate;
    }
}

/// `ItemStack.validateStrict`: check the effective size, durability and
/// contained stacks before accepting an edited stack.
fn validate_strict(stack: &ItemStack, items: &crate::items::Items) -> bool {
    let max = stack.max_stack_size(items);
    stack.count <= max
        && (max <= 1 || !stack.carries_component(items.get(&stack.item), "minecraft:max_damage"))
        && stack.data().is_none_or(|data| {
            // Vanilla checks the counts of the immediate templates here.
            // ItemStackTemplate.create validates children when the template becomes a stack.
            data.container
                .iter()
                .map(|(_, stack)| stack)
                .chain(&data.bundle_contents)
                .chain(&data.charged)
                .all(|stack| stack.count <= stack.max_stack_size(items))
                && crate::bundle::valid_contents_weight(items, &data.bundle_contents)
        })
}

/// Whether two values of component `key` are equal as its codec reads them.
/// The scalar number components decode with `Codec.INT` / `Codec.FLOAT`
/// (`Number.intValue`, `floatValue`): a loaded prototype keeps JSON's
/// narrowest tag (`250` is a short) and `250.5f` decodes to 250. The break
/// sound's registry reference accepts the default namespace and `use_effects`
/// compares decoded fields. Other components compare NBT exactly.
pub fn same_component_value(key: &str, a: &Tag, b: &Tag) -> bool {
    let int = |tag: &Tag| match tag {
        Tag::Byte(v) => Some(i32::from(*v)),
        Tag::Short(v) => Some(i32::from(*v)),
        Tag::Int(v) => Some(*v),
        // Java's `long.intValue` wraps; `(int)` of a float or double saturates.
        Tag::Long(v) => Some(*v as i32),
        Tag::Float(bits) => Some(f32::from_bits(*bits) as i32),
        Tag::Double(bits) => Some(f64::from_bits(*bits) as i32),
        _ => None,
    };
    let float = |tag: &Tag| match tag {
        Tag::Float(bits) => Some(f32::from_bits(*bits)),
        Tag::Double(bits) => Some(f64::from_bits(*bits) as f32),
        Tag::Long(v) => Some(*v as f32),
        _ => int(tag).map(|v| v as f32),
    };
    match key {
        "minecraft:damage"
        | "minecraft:max_damage"
        | "minecraft:max_stack_size"
        | "minecraft:repair_cost"
        | "minecraft:map_id"
        | "minecraft:ominous_bottle_amplifier" => {
            matches!((int(a), int(b)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:minimum_attack_charge" => {
            matches!((float(a), float(b)), (Some(a), Some(b)) if a == b)
        }
        "minecraft:break_sound" => a == b || component_values::identifier_equal(Some(a), Some(b)),
        "minecraft:use_effects" => matches!(
            (ingredient_components::use_effects(a), ingredient_components::use_effects(b)),
            (Ok(a), Ok(b)) if a == b
        ),
        _ => a == b,
    }
}

/// The stack's effective components (`ItemStack.getComponents`): the
/// prototype with the stack's patch over it, its removals taken out.
pub fn effective_components(
    stack: &ItemStack,
    prototype: Option<&horizon_core::nbt::Compound>,
) -> horizon_core::nbt::Compound {
    let mut components = prototype.cloned().unwrap_or_default();
    if let Some(Tag::Compound(patch)) = encode_components(stack) {
        for (key, value) in patch {
            match key.to_string_lossy().strip_prefix('!') {
                Some(removed) => {
                    components.remove(&JavaString::from(removed));
                }
                None => {
                    components.insert(key, value);
                }
            }
        }
    }
    components
}

/// `PatchedDataComponentMap.applyPatch` leaves a component out of the patch
/// when it equals the item prototype. `Items.FIREWORK_ROCKET` carries
/// `Fireworks(1, [])`, so that exact value is the prototype's, not the
/// stack's; the same value on any other item is the stack's own component.
pub(crate) fn elide_prototype_fireworks(item: &str, data: &mut StackData) {
    if item == "minecraft:firework_rocket" && data.fireworks == Some((1, Vec::new())) {
        data.fireworks = None;
    }
}

/// The `[id=value,...]` component list of an item argument (`ItemParser`)
/// as a `components` compound, retaining `!id` removal patches.
pub fn command_components(text: &str) -> Result<Tag, String> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or("expected '['")?;
    let mut entries = horizon_core::nbt::Compound::new();
    let mut rest = inner.trim_start();
    while !rest.is_empty() {
        let end = rest.find(['=', ',']).unwrap_or(rest.len());
        let key = rest[..end].trim();
        rest = &rest[end..];
        if let Some(value) = rest.strip_prefix('=') {
            let (tag, used) = horizon_core::nbt::parse_snbt(value)?;
            let id = if key.contains(':') {
                key.to_owned()
            } else {
                format!("minecraft:{key}")
            };
            entries.insert(JavaString::from(id.as_str()), tag);
            rest = &value[used..];
        } else if let Some(key) = key.strip_prefix('!') {
            let id = if key.contains(':') {
                format!("!{key}")
            } else {
                format!("!minecraft:{key}")
            };
            entries.insert(
                JavaString::from(id.as_str()),
                Tag::compound([] as [(&str, Tag); 0]),
            );
        } else {
            return Err("expected '='".to_string());
        }
        rest = rest.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
    Ok(Tag::Compound(entries))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::banner_layer::BannerLayer;

    #[test]
    fn set_components_applies_prototype_rules_before_accepting_the_patch() {
        let stairs = "minecraft:mud_brick_stairs";
        let bush = "minecraft:bush";
        let compass = "minecraft:compass";
        let mut items = crate::items::Items::empty();
        items.component_defaults = Arc::new(std::collections::HashMap::from([
            (
                stairs.to_owned(),
                patch(r#"{"minecraft:break_sound":"minecraft:entity.item.break"}"#),
            ),
            (bush.to_owned(), patch(r#"{"minecraft:use_effects":{}}"#)),
        ]));
        for (item, count, before, change, after) in [
            (
                stairs,
                1,
                r#"{"minecraft:break_sound":"minecraft:block.stone.break","minecraft:custom_data":{kept:1}}"#,
                r#"{"minecraft:break_sound":"entity.item.break"}"#,
                r#"{"minecraft:custom_data":{kept:1}}"#,
            ),
            (
                bush,
                1,
                r#"{"minecraft:custom_data":{temporary:1}}"#,
                r#"{"!minecraft:use_effects":{},"!minecraft:custom_data":{}}"#,
                r#"{"!minecraft:use_effects":{}}"#,
            ),
            (
                bush,
                1,
                r#"{"minecraft:custom_data":{kept:1},"minecraft:use_effects":{can_sprint:true}}"#,
                r#"{"minecraft:use_effects":{can_sprint:false}}"#,
                r#"{"minecraft:custom_data":{kept:1}}"#,
            ),
            (
                compass,
                8,
                "{}",
                r#"{"minecraft:max_stack_size":16b}"#,
                r#"{"minecraft:max_stack_size":16}"#,
            ),
            (
                compass,
                8,
                r#"{"minecraft:max_stack_size":16}"#,
                r#"{"minecraft:max_stack_size":100}"#,
                r#"{"minecraft:max_stack_size":16}"#,
            ),
            (
                compass,
                64,
                r#"{"minecraft:custom_data":{kept:1}}"#,
                r#"{"minecraft:max_stack_size":16,"minecraft:rarity":"rare"}"#,
                r#"{"minecraft:custom_data":{kept:1}}"#,
            ),
        ] {
            let mut stack = ItemStack::new(item, count);
            (stack.damage, stack.extra) = decode_components(Some(&Tag::Compound(patch(before))));
            apply_components_and_validate(&mut stack, &patch(change), &items);
            assert_eq!(stack.count, count, "{item}");
            assert_eq!(
                encode_components(&stack),
                Some(Tag::Compound(patch(after))),
                "{item}"
            );
        }
    }

    fn patch(snbt: &str) -> horizon_core::nbt::Compound {
        let (Tag::Compound(patch), _) = horizon_core::nbt::parse_snbt(snbt).unwrap() else {
            panic!("expected a component patch");
        };
        patch
    }

    #[test]
    fn set_components_keeps_or_replaces_live_map_post_processing() {
        let items = crate::items::Items::empty();
        let mut map = ItemStack::new("minecraft:filled_map", 1);
        map.data_mut().map_post_processing = Some(1);
        for (change, expected) in [
            (r#"{custom_data:{kept:1}}"#, Some(1)),
            (r#"{map_post_processing:"lock"}"#, Some(0)),
            (r#"{"!map_post_processing":{}}"#, None),
        ] {
            apply_components_and_validate(&mut map, &patch(change), &items);
            assert_eq!(map.data().and_then(|d| d.map_post_processing), expected);
        }
    }

    #[test]
    fn set_components_rejects_invalid_durability_or_contained_counts() {
        let items = crate::items::Items::empty();
        let mut stack = ItemStack::new("minecraft:compass", 1);
        let nested =
            r#"{id:"minecraft:compass",count:2,components:{"minecraft:max_stack_size":1}}"#;
        for invalid in [
            r#""minecraft:max_damage":1"#.to_owned(),
            format!(r#""minecraft:container":[{{slot:0,item:{nested}}}]"#),
            format!(r#""minecraft:charged_projectiles":[{nested}]"#),
            format!(r#""minecraft:bundle_contents":[{nested}]"#),
        ] {
            let snbt = format!(r#"{{"minecraft:rarity":"rare",{invalid}}}"#);
            apply_components_and_validate(&mut stack, &patch(&snbt), &items);
            assert_eq!(encode_components(&stack), None, "{invalid}");
        }
    }

    #[test]
    fn set_components_keeps_overstacked_grandchild_templates() {
        let items = crate::items::Items::empty();
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        let change = patch(
            r#"{bundle_contents:[{id:"chest",components:{container:[{slot:0,item:{
            id:"compass",count:2,components:{max_stack_size:1}
        }}]}}]}"#,
        );
        apply_components_and_validate(&mut bundle, &change, &items);
        let chest = &bundle.data().unwrap().bundle_contents[0];
        let compass = &chest.data().unwrap().container[0].1;
        assert_eq!(&*compass.item, "minecraft:compass");
        assert_eq!(compass.count, 2);
        assert_eq!(compass.own_max_stack_size(), Some(1));
    }

    #[test]
    fn set_components_validates_unqualified_item_templates() {
        let root =
            std::env::temp_dir().join(format!("h20490-item-templates-{}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(
            root.join("items.json"),
            r#"{"minecraft:ender_pearl":{"minecraft:max_stack_size":16}}"#,
        )
        .unwrap();
        let items = crate::items::Items::catalog(
            &root,
            &horizon_world::loot::PackOverlay::NONE,
            &horizon_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        for (item, invalid) in [
            (
                "minecraft:chest",
                r#"{"minecraft:container":[{slot:0,item:{id:"ender_pearl",count:17}}]}"#,
            ),
            (
                "minecraft:bundle",
                r#"{"minecraft:bundle_contents":[{id:"ender_pearl",count:17}]}"#,
            ),
            (
                "minecraft:bundle",
                r#"{bundle_contents:[{id:"compass",count:2,components:{max_stack_size:1}}]}"#,
            ),
            (
                "minecraft:bundle",
                r#"{bundle_contents:[{id:"compass",components:{max_stack_size:100}}]}"#,
            ),
            (
                "minecraft:chest",
                r#"{container:[{slot:0,item:{id:"compass",count:2,components:{max_stack_size:1}}}]}"#,
            ),
            (
                "minecraft:crossbow",
                r#"{charged_projectiles:[{id:"arrow",count:2,components:{max_stack_size:1}}]}"#,
            ),
        ] {
            let mut stack = ItemStack::new(item, 1);
            apply_components_and_validate(&mut stack, &patch(invalid), &items);
            assert_eq!(encode_components(&stack), None, "{invalid}");
        }
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        apply_components_and_validate(
            &mut bundle,
            &patch(r#"{"minecraft:bundle_contents":[{id:"ender_pearl",count:3},"ender_pearl"]}"#),
            &items,
        );
        let pearls = &bundle.data().unwrap().bundle_contents;
        assert_eq!(pearls.len(), 2);
        assert!(
            pearls
                .iter()
                .all(|pearl| &*pearl.item == "minecraft:ender_pearl")
        );
        let contents = crate::bundle::Contents::from_items(&items, pearls.clone());
        assert_eq!(contents.weight_fraction(), 0.25);

        apply_components_and_validate(
            &mut bundle,
            &patch(
                r#"{bundle_contents:[{id:"chest",components:{container:[{slot:0,item:{
                id:"compass",components:{max_stack_size:1b,custom_data:{max_stack_size:100}}
            }}]}}]}"#,
            ),
            &items,
        );
        let chest = &bundle.data().unwrap().bundle_contents[0];
        let compass = &chest.data().unwrap().container[0].1;
        assert_eq!(compass.own_max_stack_size(), Some(1));
        assert_eq!(
            encode_components(compass).unwrap(),
            Tag::Compound(patch(
                r#"{"minecraft:max_stack_size":1,"minecraft:custom_data":{max_stack_size:100}}"#
            )),
        );
    }

    /// `PatchedDataComponentMap.applyPatch`: values equal to the loaded prototype (whose
    /// JSON numbers load as their narrowest tag) leave the patch; differing values,
    /// removals of its components and components it lacks stay; a removal of one it lacks goes.
    #[test]
    fn values_equal_to_the_prototype_leave_the_patch() {
        let prototype = |json: serde_json::Value| {
            horizon_core::json_nbt::json_to_compound(json.as_object().unwrap()).unwrap()
        };
        let pickaxe = prototype(serde_json::json!({
            "minecraft:damage": 0, "minecraft:max_damage": 250,
            "minecraft:max_stack_size": 1, "minecraft:repair_cost": 0,
            "minecraft:enchantments": {}
        }));
        let built = |item: &str, snbt: &str, prototype: &horizon_core::nbt::Compound| {
            let patch = horizon_core::nbt::parse_snbt(snbt).unwrap().0;
            let (damage, extra) = decode_components_for_item(item, Some(&patch));
            let mut stack = ItemStack {
                item: item.into(),
                count: 1,
                damage,
                extra,
            };
            drop_prototype_values(&mut stack, prototype);
            encode_components(&stack)
        };
        let snbt = |text: &str| Some(horizon_core::nbt::parse_snbt(text).unwrap().0);
        let iron = "minecraft:iron_pickaxe";
        for equal in [
            r#"{"minecraft:damage":0}"#,
            r#"{"minecraft:damage":0,"minecraft:max_damage":250}"#,
            r#"{"minecraft:repair_cost":0,"minecraft:max_stack_size":1}"#,
            r#"{"minecraft:enchantments":{}}"#,
        ] {
            assert_eq!(built(iron, equal, &pickaxe), None, "{equal}");
        }
        for kept in [
            r#"{"minecraft:damage":32}"#,
            r#"{"minecraft:max_damage":100}"#,
            r#"{"!minecraft:damage":{}}"#,
            r#"{"minecraft:custom_data":{keep:87}}"#,
        ] {
            assert_eq!(built(iron, kept, &pickaxe), snbt(kept), "{kept}");
        }
        assert_eq!(
            built(
                iron,
                r#"{"!minecraft:custom_name":{},"minecraft:custom_data":{keep:87}}"#,
                &pickaxe,
            ),
            snbt(r#"{"minecraft:custom_data":{keep:87}}"#)
        );
        assert_eq!(
            built(
                iron,
                r#"{"minecraft:damage":32,"minecraft:max_damage":250,"minecraft:custom_data":{keep:87}}"#,
                &pickaxe,
            ),
            snbt(r#"{"minecraft:damage":32,"minecraft:custom_data":{keep:87}}"#)
        );
        // Raw NBT keeps its tag types, also inside lists and compounds; a typed
        // float default that loads from JSON as an integer still equals the float.
        let raw = prototype(serde_json::json!({
            "minecraft:custom_data": {"k": 1, "l": [1]},
            "minecraft:minimum_attack_charge": 1
        }));
        let stick = "minecraft:stick";
        for kept in [
            r#"{"minecraft:custom_data":{k:1,l:[1b]}}"#,
            r#"{"minecraft:custom_data":{k:1s,l:[1b]}}"#,
            r#"{"minecraft:minimum_attack_charge":0.5f}"#,
        ] {
            assert_eq!(built(stick, kept, &raw), snbt(kept), "{kept}");
        }
        for equal in [
            r#"{"minecraft:custom_data":{k:1b,l:[1b]}}"#,
            r#"{"minecraft:minimum_attack_charge":1.0f}"#,
        ] {
            assert_eq!(built(stick, equal, &raw), None, "{equal}");
        }
        assert!(same_component_value(
            "minecraft:max_damage",
            &Tag::Int(250),
            &Tag::Short(250)
        ));
        assert!(!same_component_value(
            "minecraft:custom_data",
            &Tag::compound([("k", Tag::Int(1))]),
            &Tag::compound([("k", Tag::Byte(1))])
        ));
        // A nested stack's raw NBT stays exact inside an item-carrying component.
        let bundle = |k: Tag| {
            Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(1)),
                (
                    "components",
                    Tag::compound([("minecraft:custom_data", Tag::compound([("k", k)]))]),
                ),
            ])])
        };
        assert!(!same_component_value(
            "minecraft:bundle_contents",
            &bundle(Tag::Byte(1)),
            &bundle(Tag::Int(1))
        ));
        // `Codec.INT` reads 250.5f as 250: the prototype's value, so it leaves the patch.
        assert!(same_component_value(
            "minecraft:max_damage",
            &Tag::Float(250.5f32.to_bits()),
            &Tag::Short(250)
        ));
        assert_eq!(
            built(iron, r#"{"minecraft:max_damage":250.5f}"#, &pickaxe),
            None
        );
        assert_eq!(
            built(iron, r#"{"minecraft:max_damage":251.5f}"#, &pickaxe),
            snbt(r#"{"minecraft:max_damage":251.5f}"#)
        );

        // The effective components: the prototype under the patch, removals out.
        let mut removed = ItemStack::new(iron, 1);
        removed
            .data_mut()
            .set_other("!minecraft:damage", Some(Tag::Compound(Default::default())));
        removed
            .data_mut()
            .set_other("minecraft:max_damage", Some(Tag::Int(100)));
        let effective = Tag::Compound(effective_components(&removed, Some(&pickaxe)));
        assert_eq!(effective.get("minecraft:damage"), None);
        assert_eq!(effective.get("minecraft:max_damage"), Some(&Tag::Int(100)));
        assert_eq!(
            effective.get("minecraft:max_stack_size"),
            Some(&Tag::Byte(1))
        );

        // A stick has no durability of its own: its patch's zero and maximum both stay.
        let stick = prototype(serde_json::json!({"minecraft:max_stack_size": 64}));
        let durable = r#"{"minecraft:damage":0,"minecraft:max_damage":20}"#;
        assert_eq!(built("minecraft:stick", durable, &stick), snbt(durable));

        // Runtime-only state survives the rebuild of the patch.
        let mut map = ItemStack::new("minecraft:filled_map", 1);
        map.data_mut()
            .set_other("minecraft:max_stack_size", Some(Tag::Int(64)));
        map.data_mut().map_post_processing = Some(1);
        let filled = prototype(serde_json::json!({"minecraft:max_stack_size": 64}));
        drop_prototype_values(&mut map, &filled);
        assert_eq!(encode_components(&map), None);
        assert_eq!(map.data().and_then(|d| d.map_post_processing), Some(1));
    }

    /// `ItemEnchantments.equals` compares a level map: the same enchantments in
    /// another order are the same component, so the stacks stack, merge and match
    /// (and hash alike); another level is still a different component.
    #[test]
    fn enchantments_compare_as_a_map_whatever_their_order() {
        let hash = |stack: &ItemStack| {
            let mut state = std::collections::hash_map::DefaultHasher::new();
            stack.hash_item_and_components(&mut state);
            state.finish()
        };
        for item in ["minecraft:diamond_sword", "minecraft:enchanted_book"] {
            let enchanted = |levels: &[(&str, u32)]| {
                let mut stack = ItemStack::new(item, 1);
                for (id, level) in levels {
                    stack.enchant(id, *level);
                }
                stack
            };
            let a = enchanted(&[("minecraft:sharpness", 5), ("minecraft:unbreaking", 3)]);
            let b = enchanted(&[("minecraft:unbreaking", 3), ("minecraft:sharpness", 5)]);
            // Each keeps the order it was given in, for saving and sending.
            assert_ne!(
                a.crafting_enchantments(),
                b.crafting_enchantments(),
                "{item}"
            );
            assert!(a.same_item_same_components(&b), "{item}");
            assert!(a.matches(&b), "{item}");
            assert_eq!(hash(&a), hash(&b), "{item}");
            let c = enchanted(&[("minecraft:unbreaking", 2), ("minecraft:sharpness", 5)]);
            assert!(!a.same_item_same_components(&c), "{item}");
            let d = enchanted(&[("minecraft:sharpness", 5)]);
            assert!(!a.same_item_same_components(&d), "{item}");
        }
    }

    /// A list may repeat an id (the vectors are public; two spellings of one id
    /// decode to one key). The lookup reads the first entry but the saved map
    /// keeps the last, so such a list only equals the very same list, in order.
    #[test]
    fn a_list_repeating_an_id_only_equals_the_same_list() {
        let hash = |stack: &ItemStack| {
            let mut state = std::collections::hash_map::DefaultHasher::new();
            stack.hash_item_and_components(&mut state);
            state.finish()
        };
        let sword = |levels: &[(&str, u32)]| {
            let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
            stack.data_mut().enchantments =
                levels.iter().map(|(id, l)| (Arc::from(*id), *l)).collect();
            stack
        };
        let one_five = sword(&[("minecraft:sharpness", 1), ("minecraft:sharpness", 5)]);
        let five_one = sword(&[("minecraft:sharpness", 5), ("minecraft:sharpness", 1)]);
        let one = sword(&[("minecraft:sharpness", 1)]);
        assert!(!one_five.same_item_same_components(&five_one));
        assert!(!one_five.same_item_same_components(&one));
        assert!(!one.same_item_same_components(&one_five));
        let again = sword(&[("minecraft:sharpness", 1), ("minecraft:sharpness", 5)]);
        assert!(one_five.same_item_same_components(&again));
        assert_eq!(hash(&one_five), hash(&again));
        // Distinct ids in another order are still equal, and hash alike.
        let a = sword(&[("minecraft:sharpness", 1), ("minecraft:unbreaking", 3)]);
        let b = sword(&[("minecraft:unbreaking", 3), ("minecraft:sharpness", 1)]);
        assert!(a.same_item_same_components(&b));
        assert_eq!(hash(&a), hash(&b));
    }

    /// `set_banner_pattern` (and `set_components`) hand a banner's layers over
    /// as a component; the renderer and the tooltip read `banner_patterns`, so
    /// they have to come out of it.
    #[test]
    fn a_loot_banner_patterns_component_dresses_the_banner() {
        let mut stack = ItemStack::new("minecraft:white_banner", 1);
        stack.data_mut().set_loot_component(
            "minecraft:banner_patterns",
            &Some(
                horizon_core::json_nbt::json_to_tag(&serde_json::json!([
                    {"pattern": "minecraft:stripe_bottom", "color": "red"},
                    {"pattern": "minecraft:border", "color": "blue"},
                ]))
                .unwrap(),
            ),
        );
        let layers = &stack.data().expect("a component box").banner_patterns;
        assert_eq!(
            layers,
            &[
                BannerLayer::new("minecraft:stripe_bottom", "red"),
                BannerLayer::new("minecraft:border", "blue"),
            ]
        );
        // A null component takes them off again.
        stack
            .data_mut()
            .set_loot_component("minecraft:banner_patterns", &None);
        assert!(stack.data().is_none_or(|d| d.banner_patterns.is_empty()));
    }

    /// `set_book_cover` (and `set_components`) hand a written book's content
    /// over as a component; the fields the tooltip and the reading screen read
    /// have to come out of it, not just sit in `other` unread.
    #[test]
    fn a_loot_written_book_component_fills_the_book() {
        let mut stack = ItemStack::new("minecraft:written_book", 1);
        stack.data_mut().set_loot_component(
            "minecraft:written_book_content",
            &Some(
                horizon_core::json_nbt::json_to_tag(&serde_json::json!({
                    "title": {"raw": "A Guide"},
                    "author": "Cartographer",
                    "generation": 2,
                    "pages": [{"raw": "page one"}, {"raw": "page two"}],
                }))
                .unwrap(),
            ),
        );
        let book = stack
            .data()
            .and_then(|d| d.written.as_ref())
            .expect("content");
        assert_eq!(book.title, "A Guide");
        assert_eq!(book.author, "Cartographer");
        assert_eq!(book.generation, 2);
        assert_eq!(book.pages, ["page one", "page two"]);
        // A null component is no component.
        stack
            .data_mut()
            .set_loot_component("minecraft:written_book_content", &None);
        assert!(stack.data().is_none_or(|d| d.written.is_none()));
    }

    /// A stack's own `attribute_modifiers` replace its type's whole; a removed
    /// component leaves none (`getOrDefault`).
    #[test]
    fn a_stacks_own_attribute_modifiers_replace_the_types() {
        let entry = |attribute: &str, id: &str, amount: f64, slot: Option<&str>| {
            let mut e = vec![
                ("type", Tag::String(attribute.into())),
                ("id", Tag::String(id.into())),
                ("amount", Tag::Double(amount.to_bits())),
                ("operation", Tag::String("add_value".into())),
            ];
            if let Some(slot) = slot {
                e.push(("slot", Tag::String(slot.into())));
            }
            Tag::compound(e)
        };
        let types = vec![ModifierRow::new(
            "minecraft:attack_damage",
            "minecraft:base_attack_damage",
            6.0,
            "add_value",
            "mainhand",
        )];
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        assert_eq!(&*stack.default_attribute_modifiers_or(&types), &types[..]);
        stack.data_mut().set_other(
            "minecraft:attribute_modifiers",
            Some(Tag::List(vec![
                entry("minecraft:armor", "test:plate", 2.0, Some("chest")),
                entry("minecraft:luck", "test:charm", 1.0, None),
                // No amount: not an entry.
                Tag::compound([("type", Tag::String("minecraft:luck".into()))]),
            ])),
        );
        let own = stack.default_attribute_modifiers_or(&types);
        assert_eq!(
            &*own,
            &[
                ModifierRow::new("minecraft:armor", "test:plate", 2.0, "add_value", "chest"),
                ModifierRow::new("minecraft:luck", "test:charm", 1.0, "add_value", "any"),
            ][..]
        );
        let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
        removed.data_mut().set_other(
            "!minecraft:attribute_modifiers",
            Some(Tag::compound(Vec::<(&str, Tag)>::new())),
        );
        assert!(removed.default_attribute_modifiers_or(&types).is_empty());
        // `matches` also compares the count.
        let one = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(one.matches(&ItemStack::new("minecraft:diamond_sword", 1)));
        assert!(!one.matches(&ItemStack::new("minecraft:diamond_sword", 2)));
        assert!(!one.matches(&stack));
    }

    #[test]
    fn an_empty_enchanted_book_patch_is_elided_only_when_saved() {
        let stack = parse_stack(r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:stored_enchantments":{}}}"#)
            .unwrap().unwrap();
        let live = components_tag(stack.damage, stack.data());
        assert!(
            live.as_ref()
                .unwrap()
                .get("minecraft:stored_enchantments")
                .is_some()
        );
        assert_eq!(
            crate::item_nbt::item_to_nbt(&stack),
            horizon_core::nbt::parse_snbt(r#"{id:"minecraft:enchanted_book",count:1}"#)
                .unwrap()
                .0,
        );
        assert_eq!(
            components_tag(stack.damage, stack.data()),
            live,
            "saving must not remove live presence"
        );
    }

    #[test]
    fn nondefault_stored_enchantment_patches_are_preserved() {
        for (item, patch) in [
            ("minecraft:book", r#"{"minecraft:stored_enchantments":{}}"#),
            ("minecraft:stone", r#"{"minecraft:stored_enchantments":{}}"#),
            (
                "minecraft:enchanted_book",
                r#"{"minecraft:stored_enchantments":{"minecraft:sharpness":1}}"#,
            ),
            (
                "minecraft:enchanted_book",
                r#"{"!minecraft:stored_enchantments":{}}"#,
            ),
        ] {
            let expected = horizon_core::nbt::parse_snbt(patch).unwrap().0;
            let stack = parse_stack(&format!("{{id:'{item}',count:1,components:{patch}}}"))
                .unwrap()
                .unwrap();
            assert_eq!(encode_components(&stack), Some(expected), "{item}: {patch}");
        }
    }

    #[test]
    fn an_empty_book_patch_keeps_unrelated_components() {
        let stack = parse_stack(r#"{id:"minecraft:enchanted_book",count:1,components:{"minecraft:stored_enchantments":{},"minecraft:custom_data":{keep:7}}}"#).unwrap().unwrap();
        let expected = horizon_core::nbt::parse_snbt(r#"{"minecraft:custom_data":{keep:7}}"#)
            .unwrap()
            .0;
        assert_eq!(encode_components(&stack), Some(expected));
    }

    #[test]
    fn inline_trim_is_modeled_and_saved_without_raw_fallback() {
        let source = horizon_core::nbt::parse_snbt(r#"{id:"minecraft:diamond_chestplate",count:1,components:{"minecraft:custom_data":{keep:284},"minecraft:trim":{material:{palette_id:"minecraft:gold",description:"Inline material"},pattern:{asset_id:"minecraft:coast",description:"Inline pattern",decal:0b}}}}"#).unwrap().0;
        let decoded = crate::item_nbt::item_from_nbt(&source).unwrap();
        let data = decoded.data().unwrap();
        assert!(
            data.trim.is_some(),
            "inline trim must be adopted into the actual model"
        );
        assert!(data.other("minecraft:trim").is_none(), "not a raw fallback");
        let mut fresh = ItemStack::new("minecraft:diamond_chestplate", 1);
        fresh.data_mut().trim = data.trim.clone();
        fresh.data_mut().set_other(
            "minecraft:custom_data",
            Some(Tag::compound([("keep", Tag::Int(284))])),
        );
        for stack in [&decoded, &fresh] {
            assert_eq!(
                components_tag(0, stack.data()),
                source.get("components").cloned()
            );
            assert_eq!(crate::item_nbt::item_to_nbt(stack), source);
        }
    }

    fn check_lodestone_saved_default(decoded: bool) {
        const TRACKER: &str = "minecraft:lodestone_tracker";
        const CUSTOM: &str = "minecraft:custom_data";
        let target = Tag::compound([
            ("dimension", Tag::String("minecraft:overworld".into())),
            ("pos", Tag::IntArray(vec![12, 200, 12])),
        ]);
        let custom = Tag::compound([("keep", Tag::Int(17))]);
        assert_eq!(
            encode_components(&ItemStack::new("minecraft:compass", 1)),
            None
        );
        let mut only_custom = ItemStack::new("minecraft:compass", 1);
        only_custom
            .data_mut()
            .set_other(CUSTOM, Some(custom.clone()));
        assert_eq!(
            encode_components(&only_custom),
            Some(Tag::compound([(CUSTOM, custom.clone())]))
        );
        // The valid false and absent controls precede the old true-default failure.
        for (tracked, has_target) in [
            (Some(false), true),
            (None, true),
            (Some(true), true),
            (Some(true), false),
        ] {
            let mut fields = Vec::new();
            let mut expected_fields = Vec::new();
            if has_target {
                fields.push(("target", target.clone()));
                expected_fields.push(("target", target.clone()));
            }
            if let Some(value) = tracked {
                fields.push(("tracked", Tag::Byte(i8::from(value))));
                if !value {
                    expected_fields.push(("tracked", Tag::Byte(0)));
                }
            }
            let source = Tag::compound(fields);
            let input = Tag::compound([(TRACKER, source.clone()), (CUSTOM, custom.clone())]);
            let expected = Tag::compound([
                (TRACKER, Tag::compound(expected_fields)),
                (CUSTOM, custom.clone()),
            ]);
            let stack = if decoded {
                let (damage, extra) = decode_components(Some(&input));
                ItemStack {
                    damage,
                    extra,
                    ..ItemStack::new("minecraft:compass", 1)
                }
            } else {
                let mut fresh = ItemStack::new("minecraft:compass", 1);
                assert!(fresh.extra.is_none());
                fresh.data_mut().set_other(TRACKER, Some(source.clone()));
                fresh.data_mut().set_other(CUSTOM, Some(custom.clone()));
                fresh
            };
            {
                assert_eq!(stack.data().unwrap().other(TRACKER), Some(&source));
                assert_eq!(encode_components(&stack), Some(expected.clone()));
                assert_eq!(components_tag(0, stack.data()), Some(input.clone()));
                let saved = crate::item_nbt::item_to_nbt(&stack);
                assert_eq!(saved.get("components"), Some(&expected));
                assert_eq!(
                    saved.get("id"),
                    Some(&Tag::String("minecraft:compass".into()))
                );
                assert_eq!(saved.get("count"), Some(&Tag::Int(1)));
                assert_eq!(stack.data().unwrap().other(TRACKER), Some(&source));
            }
        }
        let mut targetless = ItemStack::new("minecraft:compass", 1);
        targetless
            .data_mut()
            .set_other(TRACKER, Some(Tag::compound([("tracked", Tag::Byte(1))])));
        assert_eq!(
            encode_components(&targetless),
            Some(Tag::compound([(
                TRACKER,
                Tag::Compound(Default::default())
            )]))
        );
    }

    #[test]
    fn lodestone_tracker_saved_default_decoded() {
        check_lodestone_saved_default(true);
    }

    #[test]
    fn lodestone_tracker_saved_default_fresh_model() {
        check_lodestone_saved_default(false);
    }

    #[test]
    fn numeric_damage_decodes_and_saves_as_a_modelled_int() {
        let keep = Tag::compound([("keep", Tag::Int(87))]);
        for (raw, expected) in [
            (None, 0),
            (Some(Tag::Byte(1)), 1),
            (Some(Tag::Short(1)), 1),
            (Some(Tag::Int(1)), 1),
            (Some(Tag::Long(4_294_967_297)), 1),
            (Some(Tag::Float(1.5_f32.to_bits())), 1),
            (Some(Tag::Double(2.75_f64.to_bits())), 2),
            (Some(Tag::Float((-0.5_f32).to_bits())), 0),
            (
                Some(Tag::Float(2_147_483_648_f32.to_bits())),
                i32::MAX as u32,
            ),
            (Some(Tag::Double(1.0e20_f64.to_bits())), i32::MAX as u32),
        ] {
            let mut entries = vec![("minecraft:custom_data", keep.clone())];
            if let Some(raw) = &raw {
                entries.push(("minecraft:damage", raw.clone()));
            }
            let (damage, extra) = decode_components(Some(&Tag::compound(entries)));
            let loaded = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:diamond_pickaxe", 1)
            };
            assert_eq!(loaded.damage, expected, "{raw:?}");
            assert!(loaded.wire_patch().is_none());
            assert!(loaded.data().unwrap().other("minecraft:damage").is_none());
            let saved = crate::item_nbt::item_to_nbt(&loaded);
            assert_eq!(
                saved
                    .get("components")
                    .and_then(|c| c.get("minecraft:damage")),
                raw.is_some().then_some(&Tag::Int(expected as i32)),
                "{raw:?}"
            );
            assert_eq!(
                saved
                    .get("components")
                    .unwrap()
                    .get("minecraft:custom_data"),
                Some(&keep)
            );
            // No retained source or wire patch: only modelled damage is saved.
            let mut fresh = ItemStack::new("minecraft:diamond_pickaxe", 1);
            fresh.damage = expected;
            fresh
                .data_mut()
                .set_other("minecraft:custom_data", Some(keep.clone()));
            fresh.data_mut().damage_set = expected == 0 && raw.is_some();
            assert_eq!(crate::item_nbt::item_to_nbt(&fresh), saved);
        }
    }

    #[test]
    fn potion_custom_effect_icons_save_the_codec_default() {
        let parse = |text: &str| horizon_core::nbt::parse_snbt(text).unwrap().0;
        for (flags, icon) in [
            ("", 1),
            (",show_icon:0b", 0),
            (",show_icon:1b", 1),
            (",show_particles:0b", 0),
            (",show_particles:0b,show_icon:0b", 0),
            (",show_particles:0b,show_icon:1b,ambient:1b", 1),
        ] {
            let contents = |suffix: &str| {
                format!(
                    r#"{{potion:"minecraft:water",custom_color:123456,custom_effects:[{{id:"minecraft:speed",duration:100,amplifier:2{suffix}}}]}}"#
                )
            };
            let input = parse(&format!(
                r#"{{"minecraft:potion_contents":{},"minecraft:custom_data":{{keep:7}}}}"#,
                contents(flags)
            ));
            let expected = parse(&format!(
                r#"{{"minecraft:potion_contents":{},"minecraft:custom_data":{{keep:7}}}}"#,
                contents(&format!("{flags},show_icon:{icon}b"))
            ));
            let (damage, extra) = decode_components(Some(&input));
            let decoded = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:potion", 1)
            };
            assert_eq!(
                encode_components(&decoded),
                Some(expected.clone()),
                "decoded {flags}"
            );
            // Build a model directly, without decoded input or a retained raw patch.
            let mut fresh = ItemStack::new("minecraft:potion", 1);
            fresh.data_mut().potion = Some(Arc::from("minecraft:water"));
            let potion = input.get("minecraft:potion_contents").unwrap();
            for name in ["custom_color", "custom_effects"] {
                fresh
                    .data_mut()
                    .potion_extra
                    .push((name.into(), potion.get(name).unwrap().clone()));
            }
            fresh.data_mut().set_other(
                "minecraft:custom_data",
                input.get("minecraft:custom_data").cloned(),
            );
            assert_eq!(encode_components(&fresh), Some(expected), "modeled {flags}");
        }
        assert!(encode_components(&ItemStack::new("minecraft:potion", 1)).is_none());
        let mut base = ItemStack::new("minecraft:potion", 1);
        base.data_mut().potion = Some(Arc::from("minecraft:water"));
        assert_eq!(
            encode_components(&base),
            Some(parse(
                r#"{"minecraft:potion_contents":{potion:"minecraft:water"}}"#
            ))
        );
    }

    #[test]
    fn map_decoration_types_round_trip_with_default_namespaces() {
        let parse = |text| horizon_core::nbt::parse_snbt(text).unwrap().0;
        let source = parse(
            r#"{"minecraft:map_decorations":{a:{type:"minecraft:mansion",x:10.5d,z:-20.0d,rotation:90.0f},"a+b":{type:"red_x",x:-1.25d,z:2.75d,rotation:-45.5f}},"minecraft:custom_data":{keep:7}}"#,
        );
        let expected = parse(
            r#"{"minecraft:map_decorations":{a:{type:"minecraft:mansion",x:10.5d,z:-20.0d,rotation:90.0f},"a+b":{type:"minecraft:red_x",x:-1.25d,z:2.75d,rotation:-45.5f}},"minecraft:custom_data":{keep:7}}"#,
        );
        assert!(decode_components(None).1.is_none());
        for input in [&source, &expected] {
            let (damage, extra) = decode_components(Some(input));
            assert_eq!(
                extra.as_ref().unwrap().other("minecraft:map_decorations"),
                expected.get("minecraft:map_decorations")
            );
            let loaded = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:filled_map", 1)
            };
            assert_eq!(encode_components(&loaded), Some(expected.clone()));
            // A fresh model has no original NBT or retained component source.
            let mut fresh = ItemStack::new("minecraft:filled_map", 1);
            assert!(fresh.extra.is_none());
            for id in ["minecraft:map_decorations", "minecraft:custom_data"] {
                fresh.data_mut().set_other(id, input.get(id).cloned());
            }
            assert_eq!(encode_components(&fresh), Some(expected.clone()));
            assert!(fresh.data().unwrap().map_id.is_none());
        }
    }

    #[test]
    fn empty_map_decoration_patch_uses_filled_map_prototype() {
        let parse = |text| horizon_core::nbt::parse_snbt(text).unwrap().0;
        let empty = parse(r#"{"minecraft:map_decorations":{}}"#);
        assert!(decode_components(None).1.is_none());
        assert!(encode_components(&ItemStack::new("minecraft:filled_map", 1)).is_none());
        let mut fresh = ItemStack::new("minecraft:filled_map", 1);
        fresh
            .data_mut()
            .other
            .push(("minecraft:map_decorations".into(), parse("{}")));
        // The value codec has no item prototype; only the saved patch elides it.
        assert_eq!(components_tag(0, fresh.data()), Some(empty.clone()));
        assert!(encode_components(&fresh).is_none());
        let (damage, extra) = decode_components(Some(&empty));
        let loaded = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:filled_map", 1)
        };
        let saved = crate::item_nbt::item_to_nbt(&loaded);
        assert_eq!(saved, parse(r#"{id:"minecraft:filled_map",count:1}"#));
        let back = crate::item_nbt::item_from_nbt(&saved).unwrap();
        assert!(encode_components(&back).is_none());
        let keep = parse(r#"{"minecraft:map_decorations":{},"minecraft:custom_data":{keep:7}}"#);
        let (damage, extra) = decode_components(Some(&keep));
        let stack = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:filled_map", 1)
        };
        assert_eq!(
            encode_components(&stack),
            Some(parse(r#"{"minecraft:custom_data":{keep:7}}"#))
        );
    }

    #[test]
    fn nondefault_map_decoration_patches_round_trip() {
        let parse = |text| horizon_core::nbt::parse_snbt(text).unwrap().0;
        let nonempty = parse(
            r#"{"minecraft:map_decorations":{"a+b":{type:"minecraft:red_x",x:-1.25d,z:2.75d,rotation:-45.5f}},"minecraft:custom_data":{keep:7}}"#,
        );
        let removed =
            parse(r#"{"!minecraft:map_decorations":{},"minecraft:custom_data":{keep:7}}"#);
        let empty = parse(r#"{"minecraft:map_decorations":{}}"#);
        for (item, components) in [
            ("minecraft:filled_map", &nonempty),
            ("minecraft:stone", &nonempty),
            ("minecraft:filled_map", &removed),
            ("minecraft:stone", &empty),
            ("minecraft:map", &empty),
        ] {
            let (damage, extra) = decode_components(Some(components));
            let stack = ItemStack {
                damage,
                extra,
                ..ItemStack::new(item, 1)
            };
            assert_eq!(
                encode_components(&stack),
                Some(components.clone()),
                "{item}"
            );
            let back =
                crate::item_nbt::item_from_nbt(&crate::item_nbt::item_to_nbt(&stack)).unwrap();
            assert_eq!(encode_components(&back), Some(components.clone()), "{item}");
        }
    }

    fn check_hive_saved_patch(decoded: bool) {
        let parse = |text| horizon_core::nbt::parse_snbt(text).unwrap().0;
        let zero = parse(r#"{"minecraft:block_state":{honey_level:"0"}}"#);
        let keep = parse(
            r#"{"minecraft:block_state":{honey_level:"0"},"minecraft:custom_data":{keep:34}}"#,
        );
        let custom = parse(r#"{"minecraft:custom_data":{keep:34}}"#);
        let one = parse(r#"{"minecraft:block_state":{honey_level:"1"}}"#);
        let empty = parse(r#"{"minecraft:block_state":{}}"#);
        let extra = parse(r#"{"minecraft:block_state":{honey_level:"0",facing:"south"}}"#);
        let removed = parse(r#"{"!minecraft:block_state":{},"minecraft:custom_data":{keep:34}}"#);
        for item in ["minecraft:beehive", "minecraft:bee_nest", "minecraft:stone"] {
            assert!(encode_components(&ItemStack::new(item, 1)).is_none());
            for source in [&zero, &keep, &one, &empty, &extra, &removed] {
                let mut stack = ItemStack::new(item, 1);
                if decoded {
                    (stack.damage, stack.extra) = decode_components(Some(source));
                } else {
                    // A fresh model, without retained source NBT.
                    assert!(stack.extra.is_none());
                    let Tag::Compound(entries) = source else {
                        unreachable!()
                    };
                    for (id, value) in entries {
                        stack
                            .data_mut()
                            .set_other(&id.to_string_lossy(), Some(value.clone()));
                    }
                }
                let before = stack.data().cloned();
                assert_eq!(
                    components_tag(stack.damage, stack.data()),
                    Some(source.clone())
                );
                let saved = encode_components(&stack);
                // Saved patch canonicalization must not change live/unsaved values.
                assert_eq!(stack.data(), before.as_ref());
                assert_eq!(
                    components_tag(stack.damage, stack.data()),
                    Some(source.clone())
                );
                let expected = if item != "minecraft:stone" && source == &zero {
                    None
                } else if item != "minecraft:stone" && source == &keep {
                    Some(custom.clone())
                } else {
                    Some(source.clone())
                };
                assert_eq!(saved, expected, "{item}, decoded={decoded}, {source:?}");
                let tag = crate::item_nbt::item_to_nbt(&stack);
                assert_eq!(tag.get("components"), expected.as_ref());
                let back = crate::item_nbt::item_from_nbt(&tag).unwrap();
                assert_eq!(encode_components(&back), expected);
            }
        }
    }

    fn check_max_stack_saved_prototype(decoded: bool) {
        use crate::items::Items;
        use horizon_world::loot::PackOverlay;
        let root =
            std::env::temp_dir().join(format!("h088-prototypes-{}-{decoded}", std::process::id()));
        std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
        std::fs::write(root.join("items.json"), r#"{"minecraft:iron_helmet":{"minecraft:max_stack_size":1},"minecraft:stone":{"minecraft:max_stack_size":64},"minecraft:ender_pearl":{"minecraft:max_stack_size":16},"audit:changed":{"minecraft:max_stack_size":7},"audit:missing":{}}"#).unwrap();
        let items = Items::catalog(
            &root,
            &PackOverlay::NONE,
            &horizon_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        let parse = |text| horizon_core::nbt::parse_snbt(text).unwrap().0;
        for (id, prototype, has_default) in [
            ("minecraft:iron_helmet", 1, true),
            ("minecraft:stone", 64, true),
            ("minecraft:ender_pearl", 16, true),
            ("audit:changed", 7, true),
            ("audit:missing", 64, false),
            ("audit:unknown", 7, false),
        ] {
            assert!(encode_components_with_items(&ItemStack::new(id, 1), &items).is_none());
            for (key, value) in [
                ("minecraft:max_stack_size", Tag::Int(prototype)),
                ("minecraft:max_stack_size", Tag::Int(3)),
                ("!minecraft:max_stack_size", parse("{}")),
            ] {
                let input = Tag::compound([
                    (key, value.clone()),
                    ("minecraft:custom_data", parse("{keep:88}")),
                ]);
                let mut stack = ItemStack::new(id, 1);
                if decoded {
                    (stack.damage, stack.extra) = decode_components(Some(&input));
                } else {
                    assert!(stack.extra.is_none());
                    stack.data_mut().set_other(key, Some(value.clone()));
                    stack
                        .data_mut()
                        .set_other("minecraft:custom_data", Some(parse("{keep:88}")));
                }
                let before = stack.clone();
                assert_eq!(
                    components_tag(stack.damage, stack.data()),
                    Some(input.clone())
                );
                assert_eq!(
                    encode_components(&stack),
                    Some(input.clone()),
                    "context-free API stays unchanged"
                );
                let default = key == "minecraft:max_stack_size"
                    && value == Tag::Int(prototype)
                    && has_default;
                let expected = if default {
                    Some(parse(r#"{"minecraft:custom_data":{keep:88}}"#))
                } else {
                    Some(input.clone())
                };
                assert_eq!(
                    encode_components_with_items(&stack, &items),
                    expected,
                    "{id}, decoded={decoded}"
                );
                assert_eq!(
                    crate::item_nbt::item_to_nbt_with_items(&stack, &items).get("components"),
                    expected.as_ref()
                );
                assert_eq!(stack, before, "no live stack mutation");
                assert_eq!(components_tag(stack.damage, stack.data()), Some(input));
            }
            // Removing a sole default must also omit the empty components compound.
            let mut only = ItemStack::new(id, 1);
            only.data_mut()
                .set_other("minecraft:max_stack_size", Some(Tag::Int(prototype)));
            assert_eq!(
                encode_components_with_items(&only, &items).is_none(),
                has_default
            );
        }
        // Reload the same known item's prototype through the real catalog loader.
        std::fs::write(
            root.join("items.json"),
            r#"{"minecraft:stone":{"minecraft:max_stack_size":7}}"#,
        )
        .unwrap();
        let changed = Items::catalog(
            &root,
            &PackOverlay::NONE,
            &horizon_world::item_defaults::read(&root).unwrap(),
        )
        .unwrap();
        assert_eq!(items.get("minecraft:stone").unwrap().max_stack, 64);
        assert_eq!(changed.get("minecraft:stone").unwrap().max_stack, 7);
        for size in [7, 64] {
            let mut stack = ItemStack::new("minecraft:stone", 1);
            stack
                .data_mut()
                .set_other("minecraft:max_stack_size", Some(Tag::Int(size)));
            assert_eq!(
                encode_components_with_items(&stack, &items).is_none(),
                size == 64
            );
            assert_eq!(
                encode_components_with_items(&stack, &changed).is_none(),
                size == 7
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn decoded_max_stack_saved_patch_uses_loaded_prototype() {
        check_max_stack_saved_prototype(true);
    }
    #[test]
    fn modeled_max_stack_saved_patch_uses_loaded_prototype() {
        check_max_stack_saved_prototype(false);
    }
    #[test]
    fn decoded_hive_saved_patch_uses_whole_prototype() {
        check_hive_saved_patch(true);
    }

    #[test]
    fn fresh_hive_saved_patch_uses_whole_prototype() {
        check_hive_saved_patch(false);
    }

    #[test]
    fn an_absent_enchantments_component_is_not_enchantable() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.data_mut().set_other(
            "!minecraft:enchantments",
            Some(horizon_core::nbt::Tag::compound(Vec::<(
                &str,
                horizon_core::nbt::Tag,
            )>::new())),
        );
        stack.enchant("minecraft:sharpness", 1);
        assert!(stack.enchantments().is_empty());
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        book.data_mut().set_other(
            "!minecraft:stored_enchantments",
            Some(horizon_core::nbt::Tag::compound(Vec::<(
                &str,
                horizon_core::nbt::Tag,
            )>::new())),
        );
        book.enchant("minecraft:sharpness", 1);
        assert!(book.stored_enchantments().is_empty());
    }

    #[test]
    fn components_round_trip() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.damage = 12;
        stack.enchant("minecraft:sharpness", 3);
        stack.enchant("minecraft:unbreaking", 2);
        stack.data_mut().repair_cost = 3;
        stack.data_mut().custom_name = Some(Tag::String("Blade".into()));
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
        assert_eq!(back.custom_name().as_deref(), Some("Blade"));
    }

    #[test]
    fn enchantment_ids_without_a_namespace_are_minecraft_ones() {
        let levels = Tag::compound([
            ("knockback", Tag::Int(2)),
            ("minecraft:sharpness", Tag::Int(1)),
        ]);
        let components = Tag::compound([("minecraft:enchantments", levels)]);
        let (_, extra) = decode_components(Some(&components));
        let stack = ItemStack {
            extra,
            ..ItemStack::new("minecraft:wooden_sword", 1)
        };
        assert_eq!(stack.enchantment_level("minecraft:knockback"), 2);
        assert_eq!(stack.enchantment_level("minecraft:sharpness"), 1);
    }

    #[test]
    fn invalid_enchantment_levels_leave_the_other_entries_intact() {
        for component in ["minecraft:enchantments", "minecraft:stored_enchantments"] {
            for level in [-1, 0, 256, i32::MAX] {
                let components = Tag::compound([
                    ("minecraft:damage", Tag::Int(12)),
                    (
                        component,
                        Tag::compound([("looting", Tag::Int(level)), ("sharpness", Tag::Int(3))]),
                    ),
                ]);
                let (damage, extra) = decode_components(Some(&components));
                let data = extra.unwrap();
                let levels = if component == "minecraft:enchantments" {
                    data.enchantments
                } else {
                    data.stored_enchantments
                };
                assert_eq!(damage, 12);
                assert_eq!(levels, vec![(Arc::from("minecraft:sharpness"), 3)]);
            }
        }
    }

    #[test]
    fn custom_name_reads_the_extra_spans() {
        let name = Tag::compound([
            (
                "extra",
                Tag::List(vec![
                    Tag::compound([("text", Tag::String("Games ".into()))]),
                    Tag::compound([("text", Tag::String("Right click".into()))]),
                ]),
            ),
            ("text", Tag::String("".into())),
        ]);
        let components = Tag::compound([("minecraft:custom_name", name)]);
        let (_, extra) = decode_components(Some(&components));
        assert_eq!(
            extra.and_then(|d| d.custom_name_plain()).as_deref(),
            Some("Games Right click")
        );
    }

    #[test]
    fn plain_custom_name_is_saved_as_a_string() {
        // Component.tryCollapseToString: `{text:"Cool"}` is written "Cool";
        // a style or a sibling keeps the compound.
        let decode = |name: Tag| {
            let components = Tag::compound([("minecraft:custom_name", name)]);
            decode_components(Some(&components)).1.unwrap().custom_name
        };
        let plain = Tag::compound([("text", Tag::String("Cool".into()))]);
        assert_eq!(decode(plain), Some(Tag::String("Cool".into())));
        let styled = Tag::compound([
            ("text", Tag::String("Cool".into())),
            ("color", Tag::String("red".into())),
        ]);
        assert_eq!(decode(styled.clone()), Some(styled));
        let mut data = StackData::default();
        data.set_other(
            "minecraft:custom_name",
            Some(Tag::compound([("text", Tag::String("Loot".into()))])),
        );
        assert_eq!(data.custom_name, Some(Tag::String("Loot".into())));
    }

    #[test]
    fn rich_custom_name_is_the_single_saved_value_after_rename() {
        let name = Tag::compound([
            ("text", Tag::String("".into())),
            (
                "extra",
                Tag::List(vec![
                    Tag::compound([
                        ("text", Tag::String("A".into())),
                        ("color", Tag::String("#ff0000".into())),
                    ]),
                    Tag::compound([
                        ("text", Tag::String("B".into())),
                        ("color", Tag::String("#0000ff".into())),
                    ]),
                ]),
            ),
        ]);
        let components = Tag::compound([("minecraft:custom_name", name.clone())]);
        let (_, extra) = decode_components(Some(&components));
        let mut stack = ItemStack {
            extra,
            ..ItemStack::new("minecraft:stick", 1)
        };
        assert_eq!(stack.custom_name().as_deref(), Some("AB"));
        assert_eq!(encode_components(&stack), Some(components));
        stack
            .data_mut()
            .set_other("minecraft:custom_name", Some(Tag::String("Changed".into())));
        assert_eq!(stack.custom_name().as_deref(), Some("Changed"));
        assert_eq!(
            encode_components(&stack)
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&Tag::String("Changed".into()))
        );
    }

    #[test]
    fn loot_name_preserves_plain_and_styled_components() {
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &serde_json::json!("Rock"));
        assert_eq!(
            encode_components(&stack)
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&Tag::String("Rock".into()))
        );
        let styled = serde_json::json!({"text": "Rock", "color": "red"});
        stack
            .data_mut()
            .set_loot_name("minecraft:custom_name", &styled);
        assert_eq!(
            encode_components(&stack)
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&horizon_client_core::scoreboard::json_to_tag(&styled))
        );
    }

    #[test]
    fn loot_component_keeps_a_styled_custom_name() {
        let name = serde_json::json!({"text":"Prize","color":"#ff8800"});
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().set_loot_component(
            "minecraft:custom_name",
            &Some(horizon_core::json_nbt::json_to_tag(&name).unwrap()),
        );
        assert_eq!(stack.custom_name().as_deref(), Some("Prize"));
        let expected = horizon_client_core::scoreboard::json_to_tag(&name);
        assert_eq!(
            encode_components(&stack)
                .unwrap()
                .get("minecraft:custom_name"),
            Some(&expected)
        );
        stack
            .data_mut()
            .set_loot_component("minecraft:custom_name", &None);
        assert!(stack.custom_name().is_none());
    }

    /// A chest's `set_components` patch: a potion's contents land where
    /// drinking and splashing read them, not only in its saved form, and the
    /// components Horizon keeps as saved stay so.
    #[test]
    fn a_loot_component_horizon_models_lands_in_its_field() {
        use serde_json::json;
        let mut swift = ItemStack::new("minecraft:splash_potion", 1);
        swift.data_mut().set_loot_component(
            "minecraft:potion_contents",
            &Some(
                horizon_core::json_nbt::json_to_tag(&json!({"potion": "minecraft:swiftness"}))
                    .unwrap(),
            ),
        );
        assert_eq!(swift.potion(), Some("minecraft:swiftness"));
        let mut custom = ItemStack::new("minecraft:potion", 1);
        let data = custom.data_mut();
        data.set_loot_component(
            "minecraft:custom_data",
            &Some(horizon_core::json_nbt::json_to_tag(&json!({"page": 3})).unwrap()),
        );
        data.set_loot_component(
            "minecraft:damage",
            &Some(horizon_core::json_nbt::json_to_tag(&json!(5)).unwrap()),
        );
        let effects = json!({"custom_effects": [{"id": "minecraft:resistance", "amplifier": 1, "duration": 1800}]});
        data.set_loot_component(
            "minecraft:potion_contents",
            &Some(horizon_core::json_nbt::json_to_tag(&effects).unwrap()),
        );
        assert_eq!(
            custom.custom_potion_effects(),
            [("minecraft:resistance".to_owned(), 1800, 1)]
        );
        let data = custom.data().unwrap();
        assert!(data.other("minecraft:custom_data").is_some());
        assert!(
            data.other("minecraft:damage").is_some(),
            "a damage set before stays"
        );
        // Saved as before: the patch's contents, with the codec's icon flag.
        let saved = encode_components(&custom).unwrap();
        assert_eq!(
            saved.get("minecraft:potion_contents").unwrap(),
            &horizon_core::json_nbt::json_to_tag(&json!({"custom_effects": [{"id": "minecraft:resistance", "amplifier": 1, "duration": 1800, "show_icon": true}]})).unwrap()
        );
        // A null value takes the potion away again.
        swift
            .data_mut()
            .set_loot_component("minecraft:potion_contents", &None);
        assert_eq!(swift.potion(), None);
    }

    #[test]
    fn written_book_pages_may_be_component_lists() {
        // A page as Hypixel sends it through ViaVersion: a list, not a compound.
        let page = Tag::List(vec![
            Tag::compound([("text", Tag::String("".into()))]),
            Tag::compound([("text", Tag::String("Please read ".into()))]),
            Tag::compound([("text", Tag::String("the rules".into()))]),
        ]);
        let book = Tag::compound([
            (
                "title",
                Tag::compound([("raw", Tag::String("Rules".into()))]),
            ),
            ("author", Tag::String("Server".into())),
            (
                "pages",
                Tag::List(vec![
                    page,
                    Tag::compound([("raw", Tag::String("Two".into()))]),
                ]),
            ),
        ]);
        let components = Tag::compound([("minecraft:written_book_content", book)]);
        let (_, extra) = decode_components(Some(&components));
        let pages = extra.and_then(|d| d.written.clone()).map(|w| w.pages);
        assert_eq!(
            pages,
            Some(vec!["Please read the rules".to_owned(), "Two".to_owned()])
        );
    }

    #[test]
    fn banner_layers_and_shield_base_round_trip() {
        let mut stack = ItemStack::new("minecraft:shield", 1);
        stack.data_mut().base_color = Some(Arc::from("red"));
        stack.data_mut().banner_patterns = vec![
            crate::banner_layer::BannerLayer::new("minecraft:flower", "black"),
            crate::banner_layer::BannerLayer::new("minecraft:border", "white"),
        ];
        let tag = encode_components(&stack).expect("components");
        let (_, extra) = decode_components(Some(&tag));
        let extra = extra.expect("data");
        assert_eq!(extra.base_color.as_deref(), Some("red"));
        assert_eq!(extra.banner_patterns, stack.data().unwrap().banner_patterns);
    }

    #[test]
    fn banner_pattern_patch_elides_only_the_prototype_value() {
        let key = "minecraft:banner_patterns";
        let empty = Tag::List(Vec::new());
        let patch = Tag::compound([(key, empty.clone())]);
        let (_, shield) = decode_components_for_item("minecraft:shield", Some(&patch));
        assert!(shield.is_none());
        for id in ["minecraft:stone", "example:white_banner"] {
            let mut stack = ItemStack::new(id, 1);
            (stack.damage, stack.extra) = decode_components_for_item(id, Some(&patch));
            assert_eq!(stack.data().unwrap().other(key), Some(&empty));
            assert_eq!(encode_components(&stack), Some(patch.clone()));
        }

        let mut banner = ItemStack::new("minecraft:white_banner", 1);
        banner.data_mut().set_other(key, Some(empty));
        assert!(encode_components(&banner).is_none());

        let removed = "!minecraft:banner_patterns";
        let patch = Tag::compound([(removed, Tag::Compound(Default::default()))]);
        (banner.damage, banner.extra) = decode_components_for_item(&banner.item, Some(&patch));
        assert_eq!(banner.data().unwrap().other(removed), patch.get(removed));
        assert_eq!(encode_components(&banner), Some(patch));
    }

    #[test]
    fn ominous_items_round_trip() {
        let mut banner = ItemStack::new("minecraft:white_banner", 1);
        banner.data_mut().ominous_banner = true;
        let tag = encode_components(&banner).unwrap();
        assert!(tag.get("minecraft:banner_patterns").is_some());
        let (_, back) = decode_components(Some(&tag));
        let back = back.unwrap();
        assert!(back.ominous_banner);
        assert!(back.other("minecraft:tooltip_display").is_none());

        banner.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::Compound(Default::default())),
        );
        let (_, custom_display) = decode_components(encode_components(&banner).as_ref());
        assert_eq!(
            custom_display.unwrap().other("minecraft:tooltip_display"),
            Some(&Tag::Compound(Default::default()))
        );
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
            crate::firework::FireworkShape::LargeBall
        );
        assert_eq!(explosions[0].colors, vec![0xFF0000]);
        assert!(explosions[0].twinkle && !explosions[0].trail);
    }

    #[test]
    fn command_components_rejects_bare_component_without_equals() {
        assert_eq!(
            command_components("[custom_data]"),
            Err("expected '='".to_string())
        );
        assert_eq!(
            command_components("[!tool,custom_data]"),
            Err("expected '='".to_string())
        );
    }

    #[test]
    fn debug_stick_state_round_trips() {
        let mut stack = ItemStack::new("minecraft:debug_stick", 1);
        stack
            .data_mut()
            .set_debug_stick_property("minecraft:oak_stairs", "facing");
        stack
            .data_mut()
            .set_debug_stick_property("minecraft:oak_stairs", "half");
        stack
            .data_mut()
            .set_debug_stick_property("minecraft:stone", "axis");

        let tag = encode_components(&stack).expect("debug stick components");
        let (_, extra) = decode_components(Some(&tag));
        let data = extra.expect("debug stick state");
        assert_eq!(
            data.debug_stick_property("minecraft:oak_stairs").as_deref(),
            Some("half")
        );
        assert_eq!(
            data.debug_stick_property("minecraft:stone").as_deref(),
            Some("axis")
        );
        assert_eq!(data.debug_stick_property("minecraft:dirt"), None);
    }

    /// The modelled component IDs, as `decode_components` read them before the
    /// single pass: one `BTreeMap` lookup per ID, whether or not the item
    /// carries it.
    fn baseline_decode_components(components: Option<&Tag>) -> (u32, Option<Box<StackData>>) {
        let Some(c) = components else {
            return (0, None);
        };
        let damage =
            horizon_world::datafix::dynamic::as_int(c.get("minecraft:damage"), 0).max(0) as u32;
        let potion = match c.get("minecraft:potion_contents") {
            Some(Tag::String(s)) => Some(Arc::from(s.to_string_lossy().as_str())),
            Some(t) => t
                .get("potion")
                .and_then(string)
                .map(|s| Arc::from(s.as_str())),
            None => None,
        };
        let potion_extra = match c.get("minecraft:potion_contents") {
            Some(Tag::Compound(map)) => map
                .iter()
                .filter(|(k, _)| k.to_string_lossy() != "potion")
                .map(|(k, v)| (k.to_string_lossy(), v.clone()))
                .collect(),
            _ => Vec::new(),
        };
        let mut data = StackData {
            damage_set: damage == 0 && c.get("minecraft:damage").is_some(),
            death_protection: c
                .get("minecraft:death_protection")
                .and_then(|raw| {
                    crate::items::parse_death_effects(
                        &horizon_client_core::scoreboard::tag_to_json(raw),
                    )
                    .ok()
                })
                .map(Arc::new),
            consumable: c
                .get("minecraft:consumable")
                .cloned()
                .and_then(ConsumableComponent::new),
            kinetic_weapon: c
                .get("minecraft:kinetic_weapon")
                .cloned()
                .map(crate::kinetic_weapon::KineticComponent::new),

            enchantments: read_levels(c.get("minecraft:enchantments")),
            stored_enchantments: read_levels(c.get("minecraft:stored_enchantments")),
            repair_cost: c
                .get("minecraft:repair_cost")
                .and_then(int)
                .unwrap_or(0)
                .max(0) as u32,
            explicit_zero_damage: damage == 0 && c.get("minecraft:damage").is_some(),
            custom_name: c
                .get("minecraft:custom_name")
                .filter(|tag| read_text(tag).is_some())
                .cloned()
                .map(collapsed_text),
            potion,
            potion_extra,
            map_id: c.get("minecraft:map_id").and_then(int).map(|v| v as i32),
            banner_patterns: crate::banner_layer::layers_from_nbt(
                c.get("minecraft:banner_patterns"),
            ),
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
                .map(written_book_from_tag),
            fireworks: c.get("minecraft:fireworks").map(|f| {
                let flight = f
                    .get("flight_duration")
                    .and_then(int)
                    .unwrap_or(0)
                    .clamp(0, 255) as u8;
                let explosions = match f.get("explosions") {
                    Some(Tag::List(list)) => list.iter().map(explosion_from).collect(),
                    _ => Vec::new(),
                };
                (flight, explosions)
            }),
            firework_explosion: c.get("minecraft:firework_explosion").map(explosion_from),
            dyed_color: c
                .get("minecraft:dyed_color")
                .and_then(horizon_core::color::rgb_color_from_tag),
            pot_decorations: c
                .get("minecraft:pot_decorations")
                .map(|p| {
                    let side = |name: &str| p.get(name).and_then(template_from);
                    Box::new([side("back"), side("left"), side("right"), side("front")])
                })
                .filter(|sides| !all_bricks(sides)),
            bundle_contents: match c.get("minecraft:bundle_contents") {
                Some(Tag::List(list)) => list.iter().filter_map(template_from).collect(),
                _ => Vec::new(),
            },
            stew_effects: match c.get("minecraft:suspicious_stew_effects") {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|e| {
                        Some((
                            Arc::from(e.get("id").and_then(string)?.as_str()),
                            e.get("duration").and_then(int).unwrap_or(160) as i32,
                        ))
                    })
                    .collect(),
                _ => Vec::new(),
            },
            map_post_processing: match c.get("minecraft:map_post_processing") {
                Some(Tag::String(s)) => Some(u8::from(s.to_string_lossy() != "lock")),
                Some(t) => int(t).map(|v| v.clamp(0, 1) as u8),
                None => None,
            },
            charged: match c.get("minecraft:charged_projectiles") {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(crate::item_nbt::item_from_nbt)
                    .collect(),
                _ => Vec::new(),
            },
            container: match c.get("minecraft:container") {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|e| {
                        Some((
                            e.get("slot").and_then(int)?.clamp(0, 255) as u8,
                            template_from(e.get("item")?)?,
                        ))
                    })
                    .collect(),
                _ => Vec::new(),
            },
            ominous_banner: c
                .get("minecraft:item_name")
                .and_then(|n| n.get("translate"))
                .and_then(string)
                .as_deref()
                == Some("block.minecraft.ominous_banner")
                && matches!(
                    c.get("minecraft:banner_patterns"),
                    Some(Tag::List(layers)) if layers.len() == OMINOUS_LAYERS.len()),
            ominous_amplifier: c
                .get("minecraft:ominous_bottle_amplifier")
                .and_then(int)
                .map(|v| v.clamp(0, 4) as u8),
            trim: c.get("minecraft:trim").and_then(trim::from_nbt),
            bundle_selected: BundleSelected::default(),
            other: Vec::new(),
            wire: Wire::default(),
        };
        if let Tag::Compound(map) = c {
            for (k, v) in map {
                let key = k.to_string_lossy();
                let ominous_part = data.ominous_banner
                    && (matches!(key.as_str(), "minecraft:item_name" | "minecraft:rarity")
                        || (key == "minecraft:tooltip_display" && v == &ominous_tooltip_display()));
                if key == "minecraft:stored_enchantments"
                    && data.stored_enchantments.is_empty()
                    && matches!(v, Tag::Compound(_))
                {
                    data.other
                        .push((key, Tag::compound(Vec::<(&str, Tag)>::new())));
                } else if !MODELLED.contains(&key.as_str()) && !ominous_part {
                    let value = canonical_other_component(&key, v);
                    data.other.push((key, value));
                }
            }
            data.other.sort_by(|a, b| a.0.cmp(&b.0));
        }
        (damage, (!data.is_empty()).then(|| Box::new(data)))
    }
    /// Components as saved: none, one kept ID, every modelled ID, and IDs the
    /// model keeps in `other`, one of them with a non-ASCII unit.
    fn component_cases() -> Vec<(&'static str, Tag)> {
        let parse = |text: &str| horizon_core::nbt::parse_snbt(text).unwrap().0;
        vec![
            ("not a compound", Tag::String("minecraft:damage".into())),
            ("empty", Tag::compound(Vec::<(&str, Tag)>::new())),
            (
                "one kept id",
                parse(r#"{"minecraft:custom_data":{fixture:"save batch"}}"#),
            ),
            (
                "every modelled id",
                parse(
                    r#"{"minecraft:damage": 7,
                        "minecraft:potion_contents": {potion:"minecraft:water",custom_color:255,custom_effects:[{id:"minecraft:speed",duration:100,amplifier:1}]},
                        "minecraft:enchantments": {levels:{"minecraft:sharpness":3}},
                        "minecraft:stored_enchantments": {levels:{"minecraft:mending":1}},
                        "minecraft:repair_cost": 4,
                        "minecraft:custom_name": {text:"Blade",color:"red"},
                        "minecraft:map_id": 12,
                        "minecraft:banner_patterns": [{pattern:"minecraft:border",color:"blue"}],
                        "minecraft:base_color": "red",
                        "minecraft:writable_book_content": {pages:[{raw:"one"},{raw:"two"}]},
                        "minecraft:written_book_content": {title:{raw:"T"},author:"A",generation:1,pages:[{raw:"p"}]},
                        "minecraft:fireworks": {flight_duration:2,explosions:[{shape:"large_ball",colors:[I;16711680],has_twinkle:true}]},
                        "minecraft:firework_explosion": {shape:"star",colors:[I;255]},
                        "minecraft:dyed_color": 16711680,
                        "minecraft:pot_decorations": {back:{id:"minecraft:brick"},front:{id:"minecraft:brick"}},
                        "minecraft:bundle_contents": [{id:"minecraft:stone",count:2}],
                        "minecraft:suspicious_stew_effects": [{id:"minecraft:poison",duration:100}],
                        "minecraft:map_post_processing": "lock",
                        "minecraft:charged_projectiles": [{id:"minecraft:arrow",count:1}],
                        "minecraft:container": [{slot:0,item:{id:"minecraft:stone",count:3}}],
                        "minecraft:ominous_bottle_amplifier": 2,
                        "minecraft:trim": {material:"minecraft:gold",pattern:"minecraft:coast"},
                        "minecraft:item_name": {translate:"block.minecraft.ominous_banner"},
                        "minecraft:rarity": "uncommon",
                        "minecraft:tooltip_display": {hide_tooltip:true}
                    }"#,
                ),
            ),
            (
                "unknown and non-ASCII ids",
                parse(
                    r#"{"minecraft:custom_data":{keep:1},"minecraft:future_component":"x","minecraft:damage\u00e9":3}"#,
                ),
            ),
        ]
    }
    /// One pass over the compound decodes every shape the per-ID lookups did,
    /// including IDs kept in `other` and keys that are not ASCII.
    #[test]
    fn one_pass_decode_matches_the_per_id_lookups() {
        for (name, tag) in component_cases() {
            assert_eq!(
                decode_components(Some(&tag)),
                baseline_decode_components(Some(&tag)),
                "{name}"
            );
        }
        assert_eq!(decode_components(None), baseline_decode_components(None));
    }
    /// `decode_components` against the per-ID lookups, over the cases above. 25
    /// measured rounds alternate order and report the median microseconds per
    /// call.
    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn benchmark_decode_components() {
        use std::{hint::black_box, time::Instant};
        for (name, tag) in component_cases() {
            let reps = 10_000;
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..25 {
                for optimized in [round % 2 == 0, round % 2 != 0] {
                    let started = Instant::now();
                    for _ in 0..reps {
                        if optimized {
                            black_box(decode_components(black_box(Some(&tag))));
                        } else {
                            black_box(baseline_decode_components(black_box(Some(&tag))));
                        }
                    }
                    if round > 0 {
                        times[usize::from(optimized)]
                            .push(started.elapsed().as_secs_f64() * 1e6 / f64::from(reps));
                    }
                }
            }
            for t in &mut times {
                t.sort_by(f64::total_cmp);
            }
            let (base, new) = (times[0][times[0].len() / 2], times[1][times[1].len() / 2]);
            println!(
                "decode {name}: baseline {base:.3} us, new {new:.3} us, {:.2}x",
                base / new
            );
        }
    }
}

#[cfg(test)]
mod carried_component_tests {
    use super::*;

    #[test]
    fn invalid_payload_replacements_leave_existing_component_unchanged() {
        for (id, field) in [
            ("minecraft:consumable", "on_consume_effects"),
            ("minecraft:death_protection", "death_effects"),
        ] {
            let good = Tag::compound([(
                field,
                Tag::List(vec![Tag::compound([
                    ("type", Tag::String("example:charge".into())),
                    ("amount", Tag::Int(42)),
                ])]),
            )]);
            let bad = Tag::compound([(
                field,
                Tag::List(vec![Tag::compound([
                    ("type", Tag::String("example:charge".into())),
                    ("amount", Tag::End),
                ])]),
            )]);
            let mut data = StackData::default();
            data.set_other(id, Some(good.clone()));
            let consumable = data.consumable.clone();
            let death = data.death_protection.clone();
            data.set_other(id, Some(bad));
            assert_eq!(data.other(id), Some(&good));
            if let Some(before) = consumable {
                assert!(Arc::ptr_eq(&before, data.consumable.as_ref().unwrap()));
            }
            if let Some(before) = death {
                assert!(Arc::ptr_eq(
                    &before,
                    data.death_protection.as_ref().unwrap()
                ));
            }
            data.set_other(id, None);
            assert!(data.other(id).is_none());
        }
    }

    #[test]
    fn death_payload_is_decoded_once_and_replaced_with_component() {
        use crate::items::ConsumeEffect;
        let component = Tag::compound([(
            "death_effects",
            Tag::List(vec![Tag::compound([
                ("type", Tag::String("example:charge".into())),
                ("amount", Tag::Int(42)),
            ])]),
        )]);
        let patch = Tag::compound([("minecraft:death_protection", component.clone())]);
        let (_, extra) = decode_components(Some(&patch));
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.extra = extra;
        let copied = stack.clone();
        let (ConsumeEffect::Native { data: a, .. }, ConsumeEffect::Native { data: b, .. }) = (
            &stack.death_protection(None).unwrap()[0],
            &copied.death_protection(None).unwrap()[0],
        ) else {
            panic!("native effect")
        };
        assert!(Arc::ptr_eq(a, b));
        assert_eq!(a.get("amount"), Some(&Tag::Byte(42)));
        assert_eq!(components_tag(0, stack.data()), Some(patch));
        stack
            .data_mut()
            .set_other("minecraft:death_protection", Some(component));
        assert!(matches!(
            &stack.death_protection(None).unwrap()[0],
            ConsumeEffect::Native { .. }
        ));
        stack
            .data_mut()
            .set_other("minecraft:death_protection", None);
        assert!(stack.death_protection(None).is_none());
    }

    #[test]
    fn damage_keeps_a_canonical_int_including_explicit_zero() {
        for damage in [0, 5] {
            let original = Tag::compound([("minecraft:damage", Tag::Int(damage))]);
            let (decoded, extra) = decode_components(Some(&original));
            assert_eq!(decoded, damage as u32);
            let encoded = components_tag(decoded, extra.as_deref());
            assert_eq!(encoded, Some(original));
        }
        let removed = Tag::compound([("!minecraft:damage", Tag::Compound(Default::default()))]);
        let (damage, extra) = decode_components(Some(&removed));
        assert_eq!(components_tag(damage, extra.as_deref()), Some(removed));
    }

    #[test]
    fn extra_snapshot_codec_omits_typed_damage_but_preserves_the_patch() {
        for damage in [0, 5] {
            let original = Tag::compound([("minecraft:damage", Tag::Int(damage))]);
            let (damage, extra) = decode_components(Some(&original));
            let stack = ItemStack {
                damage,
                extra,
                ..ItemStack::new("minecraft:iron_sword", 1)
            };
            assert_eq!(encode_extra_components(&stack), None);
            assert_eq!(encode_components(&stack), Some(original));
        }
        let patch = Tag::compound([
            ("minecraft:damage", Tag::Int(5)),
            (
                "minecraft:custom_name",
                Tag::String(JavaString::from("Snapshot")),
            ),
            ("!minecraft:consumable", Tag::Compound(Default::default())),
        ]);
        let (damage, extra) = decode_components(Some(&patch));
        let stack = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:iron_sword", 1)
        };
        let mut expected = patch;
        if let Tag::Compound(entries) = &mut expected {
            entries.remove(&JavaString::from("minecraft:damage"));
        }
        assert_eq!(encode_extra_components(&stack), Some(expected));
    }

    #[test]
    fn explicit_empty_and_preserved_values_are_carried() {
        let original = Tag::compound([(
            "minecraft:potion_contents",
            Tag::Compound(Default::default()),
        )]);
        let (_, extra) = decode_components(Some(&original));
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.extra = extra;
        assert!(stack.carries_component(None, "minecraft:potion_contents"));
        assert_eq!(components_tag(0, stack.data()), Some(original));
        stack
            .data_mut()
            .set_other("minecraft:potion_contents", None);
        assert!(!stack.carries_component(None, "minecraft:potion_contents"));
        stack
            .data_mut()
            .set_other("minecraft:repair_cost", Some(Tag::Int(5)));
        assert!(stack.carries_component(None, "minecraft:repair_cost"));
        assert_eq!(
            components_tag(0, stack.data())
                .unwrap()
                .get("minecraft:repair_cost"),
            Some(&Tag::Int(5))
        );
        stack
            .data_mut()
            .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(5)));
        assert!(stack.carries_component(None, "minecraft:repair_cost"));
    }

    #[test]
    #[ignore = "daily"]
    fn default_component_presence_and_removal() {
        let items = crate::items::prepared_items().expect("prepared item resources");
        let info = items.get("minecraft:stick").expect("stick");
        let component = info.components.first().expect("default component");
        let mut stack = ItemStack::new("minecraft:stick", 1);
        assert!(stack.carries_component(Some(info), component));
        stack.data_mut().set_other(
            &format!("!{component}"),
            Some(Tag::Compound(Default::default())),
        );
        assert!(!stack.carries_component(Some(info), component));
    }

    /// `map_post_processing` is transient: a stack carries it while `components_tag` saves
    /// nothing for it, so a presence check reads the live field.
    #[test]
    fn a_transient_component_is_carried_but_not_saved() {
        let mut map = ItemStack::new("minecraft:filled_map", 1);
        map.data_mut().map_post_processing = Some(1);
        assert!(map.carries_component(None, "minecraft:map_post_processing"));
        assert!(!map.carries_component(None, "minecraft:enchantments"));
        assert!(components_tag(map.damage, map.data()).is_none());
    }

    /// A removed default is not carried; a stack's own value is.
    #[test]
    fn a_removed_component_is_not_carried() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_other("!minecraft:lore", Some(Tag::String("removed".into())));
        assert!(!stack.carries_component(None, "minecraft:lore"));
        stack
            .data_mut()
            .set_other("minecraft:custom_data", Some(Tag::String("kept".into())));
        assert!(stack.carries_component(None, "minecraft:custom_data"));
    }
}

#[cfg(test)]
mod charged_projectile_template_tests {
    use super::*;

    #[test]
    fn charged_templates_omit_default_count_and_keep_components() {
        for count in [1, 2] {
            let mut projectile = ItemStack::new("minecraft:arrow", count);
            let marker = Tag::compound([("audit", Tag::Int(7))]);
            projectile
                .data_mut()
                .other
                .push(("minecraft:custom_data".into(), marker.clone()));
            let mut outer = ItemStack::new("minecraft:crossbow", 1);
            outer.data_mut().charged.push(projectile.clone());
            outer.data_mut().other.push((
                "minecraft:custom_data".into(),
                Tag::compound([("audit", Tag::Int(9))]),
            ));
            let encoded = encode_components(&outer).unwrap();
            let Tag::List(list) = encoded.get("minecraft:charged_projectiles").unwrap() else {
                panic!("projectile list")
            };
            assert_eq!(list.len(), 1);
            assert_eq!(
                list[0].get("count"),
                (count != 1).then_some(Tag::Int(count as i32)).as_ref()
            );
            assert_eq!(
                list[0]
                    .get("components")
                    .unwrap()
                    .get("minecraft:custom_data"),
                Some(&marker)
            );
            assert_eq!(
                encoded.get("minecraft:custom_data"),
                Some(&Tag::compound([("audit", Tag::Int(9))]))
            );
            // Generic ItemStack/inventory encoding retains its required count.
            assert_eq!(
                crate::item_nbt::item_to_nbt(&projectile).get("count"),
                Some(&Tag::Int(count as i32))
            );
            let (_, decoded) = decode_components(Some(&encoded));
            let restored = decoded.unwrap();
            assert_eq!(restored.charged[0].count, count);
            assert_eq!(
                restored.charged[0]
                    .data()
                    .unwrap()
                    .other("minecraft:custom_data"),
                Some(&marker)
            );
        }
    }
}

#[cfg(test)]
mod written_book_tests {
    use super::*;

    /// `WrittenBookContent.resolve`/`markResolved`: each page's selectors and
    /// scores become their text, the book is marked resolved, and a resolved
    /// book is left alone.
    #[test]
    fn a_written_books_pages_resolve_selectors_and_scores() {
        let selector = Tag::compound([("selector", Tag::String("@p".into()))]);
        let score = Tag::compound([(
            "score",
            Tag::compound([
                ("name", Tag::String("*".into())),
                ("objective", Tag::String("kills".into())),
            ]),
        )]);
        let mut book = WrittenBook {
            title: "t".into(),
            author: "a".into(),
            pages: vec!["@p".into(), "score".into(), "plain".into()],
            page_tags: vec![selector, score, Tag::String("plain".into())],
            resolved: false,
            ..Default::default()
        };
        let mut asked = 0;
        let changed = book.resolve(|tag| {
            asked += 1;
            if tag.get("selector").is_some() {
                return Some("Steve".into());
            }
            tag.get("score").map(|_| "7".into())
        });
        assert!(changed, "the pages were rewritten");
        assert!(book.resolved, "the book is marked resolved");
        assert_eq!(asked, 2, "a plain page is not asked");
        assert_eq!(book.pages, ["Steve", "7", "plain"]);
        assert_eq!(
            book.page_tags[0],
            Tag::compound([("text", Tag::String("Steve".into()))])
        );
        // A resolved book is never resolved again.
        assert!(!book.resolve(|_| Some("other".into())));
        assert_eq!(book.pages, ["Steve", "7", "plain"]);
    }

    /// `WrittenBookContent.resolvePage`: a page whose resolved text passes
    /// 32767 characters cancels the resolution, and the raw pages stay.
    #[test]
    fn an_over_long_resolved_page_leaves_the_pages_raw() {
        let mut book = WrittenBook {
            pages: vec!["x".into()],
            page_tags: vec![Tag::compound([("selector", Tag::String("@p".into()))])],
            ..Default::default()
        };
        let changed = book.resolve(|_| Some("y".repeat(32768)));
        assert!(!changed);
        assert!(book.resolved, "the book is still marked resolved");
        assert_eq!(book.pages, ["x"], "the raw pages stay");
    }

    /// `WrittenBookContent.CODEC` keeps the resolved flag through a save and a
    /// load.
    #[test]
    fn a_resolved_book_round_trips_its_flag() {
        let book = WrittenBook {
            title: "T".into(),
            author: "A".into(),
            pages: vec!["hi".into()],
            page_tags: vec![Tag::String("hi".into())],
            resolved: true,
            ..Default::default()
        };
        let tag = written_book_tag(&book);
        assert_eq!(tag.get("resolved"), Some(&Tag::Byte(1)));
        assert_eq!(written_book_from_tag(&tag), book);
        let unresolved = WrittenBook {
            resolved: false,
            ..book
        };
        assert_eq!(written_book_tag(&unresolved).get("resolved"), None);
    }

    /// `ItemStackTemplate.CODEC` reads `count` with `intRange(1, 99)`: outside it
    /// the template is rejected, absent it is 1.
    #[test]
    fn a_template_count_outside_one_to_ninety_nine_is_rejected() {
        let template = |count: Option<i32>| {
            let mut fields = vec![("id", Tag::String("minecraft:stone".into()))];
            if let Some(count) = count {
                fields.push(("count", Tag::Int(count)));
            }
            template_from(&Tag::compound(fields)).map(|s| s.count)
        };
        assert_eq!(template(None), Some(1));
        assert_eq!(template(Some(1)), Some(1));
        assert_eq!(template(Some(99)), Some(99));
        assert_eq!(template(Some(0)), None);
        assert_eq!(template(Some(100)), None);
        assert_eq!(template(Some(-1)), None);
    }
}
