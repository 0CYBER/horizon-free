//! Item tooltip formatting: equipment modifiers with a player's base values,
//! and lines without player context for the recipe book and creative search trees.
use crate::text::{GRAY, Text};
use crate::{
    bundle::TooltipLayout,
    enchant::Enchantments,
    hud::rgba,
    items::{ItemInfo, ItemStack, Items},
};
use horizon_core::nbt::Tag;
use horizon_world::attributes::{Display, Sentiment, Tooltip};
use mod_api::TooltipPlacement;
pub use mod_api::item::item_tooltip::TooltipDisplay;
/// Borrow the engine's modelled visibility without allocating an API stack.
pub fn tooltip_display(stack: &ItemStack) -> TooltipDisplay<'_> {
    TooltipDisplay::from_component(
        stack
            .data()
            .and_then(|data| data.other("minecraft:tooltip_display")),
        stack.data().is_some_and(|data| data.ominous_banner),
    )
}

/// `EquipmentSlotGroup` in declaration order, the order tooltips list them in.
const SLOT_GROUPS: [&str; 11] = [
    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
    "saddle",
];

/// `ItemAttributeModifiers.ATTRIBUTE_MODIFIER_FORMAT`, `DecimalFormat("#.##")`
/// of a positive amount (half-even on the exact value). Infinity reads as the
/// infinity sign, NaN as `NaN`, and a negative that rounds to zero keeps its sign.
fn format_amount(value: f64) -> String {
    if value.is_infinite() {
        return if value > 0.0 { "\u{221e}" } else { "-\u{221e}" }.to_owned();
    }
    let text = format!("{:.2}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `ItemAttributeModifiers.Display.Default.apply`: a modifier's line, none
/// for a zero amount on ordinary numeric attributes.
/// `operation` is the modifier's operation id (0: added value, 1 and 2:
/// multiplied); `display` is how the attribute reads its amounts (NeoForge's
/// `IAttributeExtension.toValueComponent`).
fn modifier_line(
    translate: &dyn Fn(&str) -> Option<String>,
    attribute: &str,
    amount: f64,
    operation: usize,
    total: bool,
    display: Display,
) -> Option<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let name = || crate::text::Text::literal(text(&format!("attribute.name.{attribute}")));
    if display == Display::Boolean {
        let state = if amount.is_nan() {
            "invalid"
        } else if total {
            if amount > 0.0 { "enabled" } else { "disabled" }
        } else if operation == 0 && amount > 0.0 {
            "enabled"
        } else if operation == 2 && amount.trunc() == -1.0 {
            "disabled"
        } else {
            "invalid"
        };
        let key = format!("horizon.attribute.modifier.{state}");
        return Some(crate::text::format_translation(&text(&key), &[name()]).plain());
    }
    // `PercentageAttribute`: an added value reads as a percentage too.
    let percent = display == Display::Percentage && operation == 0;
    let scale = match operation {
        0 if percent => 100.0,
        0 if attribute == "knockback_resistance" => 10.0,
        0 => 1.0,
        _ => 100.0,
    };
    let (kind, shown) = if total {
        ("equals", amount * scale)
    } else if amount > 0.0 {
        ("plus", amount * scale)
    } else if amount < 0.0 {
        ("take", -amount * scale)
    } else if amount.is_nan() {
        // A NaN modifier uses the decrease layout, whose key carries the minus.
        ("take", amount)
    } else {
        return None;
    };
    // A percentage reads through the multiplied operations' percent translation.
    let form = if percent { 1 } else { operation };
    // TranslatableContents: `%s` and numbered `%1$s` arguments, `%%` a percent sign.
    Some(
        crate::text::format_translation(
            &text(&format!("attribute.modifier.{kind}.{form}")),
            &[crate::text::Text::literal(format_amount(shown)), name()],
        )
        .plain(),
    )
}

/// `Attribute.getStyle`: the colour of a line for an attribute whose value
/// rises or falls. Gravity, scale and the waypoint ranges are neutral, burning
/// time and fall damage are negative, every other attribute is positive; a
/// mod's declared attribute answers its own sentiment, positive by default.
fn attribute_color(attribute: &str, rises: bool, sentiment: Sentiment) -> u32 {
    let (blue, red, grey) = (
        rgba(85, 85, 255, 255),
        rgba(255, 85, 85, 255),
        rgba(170, 170, 170, 255),
    );
    match attribute {
        "burning_time" | "fall_damage_multiplier" => {
            if rises {
                red
            } else {
                blue
            }
        }
        "gravity" | "scale" | "waypoint_transmit_range" | "waypoint_receive_range" => grey,
        _ => match sentiment {
            Sentiment::Positive => {
                if rises {
                    blue
                } else {
                    red
                }
            }
            Sentiment::Neutral => grey,
            Sentiment::Negative => {
                if rises {
                    red
                } else {
                    blue
                }
            }
        },
    }
}

/// What listeners hid from a stack's attribute lines (`GatherSkippedAttributeTooltipsEvent`):
/// every modifier, the modifiers with these ids, or those in these slot groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttributeFilter {
    /// Hide every attribute line.
    pub skip_all: bool,
    /// Modifier ids to hide; a bare name is `minecraft:`'s.
    pub ids: Vec<String>,
    /// `EquipmentSlotGroup` names to hide (`mainhand`, `armor`, `any`...).
    pub groups: Vec<String>,
}

impl AttributeFilter {
    /// Whether the modifier `id` in slot group `group` is hidden.
    pub fn skips(&self, id: &str, group: &str) -> bool {
        let qualified = |name: &str| {
            if name.contains(':') {
                name.to_owned()
            } else {
                format!("minecraft:{name}")
            }
        };
        self.skip_all
            || self.groups.iter().any(|g| g == group)
            || self.ids.iter().any(|i| qualified(i) == qualified(id))
    }
}

/// The context an attribute tooltip is built in (NeoForge's `AttributeTooltipContext`): the
/// player's attack damage and speed bases, none for recipe-search text, which has no player, and
/// the tooltip's `TooltipFlag`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttributeTooltipContext {
    /// The local player's base attack damage and attack speed; none without a player.
    pub player_base: Option<[f64; 2]>,
    /// F3+H advanced tooltips.
    pub advanced: bool,
    /// A creative tab's own slot.
    pub creative: bool,
}

/// What one attribute's lines take from the game: its declared tooltip style (colour, display,
/// base modifier), and the player's base value of it for the base modifier's total line, none
/// without a player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttributeStyle {
    pub tooltip: Tooltip,
    pub player_base: Option<f64>,
}

/// What mods change about a stack's attribute lines. The game answers it by firing the
/// `gather_skipped_attribute_tooltips` and `add_attribute_tooltips` hooks; a builder with no game
/// to ask (the creative search cache) uses [`NoAttributeListener`].
pub trait AttributeTooltipListener {
    /// The modifiers to leave out.
    fn skipped(&self, _stack: &ItemStack, _ctx: &AttributeTooltipContext) -> AttributeFilter {
        AttributeFilter::default()
    }
    /// The lines to add right after the attribute lines. `shown` is whether the stack's
    /// `minecraft:tooltip_display` shows the attribute modifiers at all.
    fn extra(
        &self,
        _stack: &ItemStack,
        _ctx: &AttributeTooltipContext,
        _shown: bool,
    ) -> Vec<(Text, u32)> {
        Vec::new()
    }
}

/// An [`AttributeTooltipListener`] that changes nothing.
pub struct NoAttributeListener;
impl AttributeTooltipListener for NoAttributeListener {}

/// The one attribute tooltip builder both routes use (the ordinary item tooltip and the recipe
/// book's search text): the filtered modifier lines when `shown`, then the lines listeners add,
/// even when the display hides the modifiers (`AddAttributeTooltipsEvent.shouldShow`). Returns the
/// modifier lines and the added ones, in order.
pub fn attribute_tooltip(
    stack: &ItemStack,
    modifiers: &[horizon_items::item_data::ModifierRow],
    ctx: &AttributeTooltipContext,
    shown: bool,
    listener: &dyn AttributeTooltipListener,
    translate: &dyn Fn(&str) -> Option<String>,
    style: &dyn Fn(&str) -> Option<AttributeStyle>,
) -> (Vec<(String, u32)>, Vec<(Text, u32)>) {
    let lines = if shown {
        let filter = listener.skipped(stack, ctx);
        attribute_lines_filtered(
            modifiers,
            ctx.player_base,
            &filter,
            ctx.advanced,
            translate,
            style,
        )
    } else {
        Vec::new()
    };
    (lines, listener.extra(stack, ctx, shown))
}

/// `ItemStack.addAttributeTooltips`: slot groups, colors and optional player
/// attack damage/speed bases. Recipe search has no player context.
pub fn attribute_lines(
    modifiers: &[horizon_items::item_data::ModifierRow],
    player_base: Option<[f64; 2]>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    attribute_lines_filtered(
        modifiers,
        player_base,
        &AttributeFilter::default(),
        false,
        translate,
        &|_| None,
    )
}

/// [`attribute_lines`] without the modifiers `filter` hides; a slot group with nothing left shows
/// no heading. Each attribute reads as its `style` says; `advanced` (F3+H) follows a mod
/// attribute's line with its modifier id (NeoForge's `IAttributeExtension.getDebugInfo`).
pub fn attribute_lines_filtered(
    modifiers: &[horizon_items::item_data::ModifierRow],
    player_base: Option<[f64; 2]>,
    filter: &AttributeFilter,
    advanced: bool,
    translate: &dyn Fn(&str) -> Option<String>,
    style: &dyn Fn(&str) -> Option<AttributeStyle>,
) -> Vec<(String, u32)> {
    let mut lines = Vec::new();
    for group in SLOT_GROUPS {
        let mut first = true;
        for horizon_items::item_data::ModifierRow {
            attribute,
            id: modifier,
            amount,
            operation,
            slot,
        } in modifiers
        {
            if slot != group || filter.skips(modifier, slot) || *amount == 0.0 {
                continue;
            }
            if std::mem::take(&mut first) {
                lines.push((String::new(), u32::MAX));
                lines.push((
                    translate(&format!("item.modifiers.{group}"))
                        .unwrap_or_else(|| format!("item.modifiers.{group}")),
                    rgba(170, 170, 170, 255),
                ));
            }
            let id = match operation.as_str() {
                "add_multiplied_base" => 1,
                "add_multiplied_total" => 2,
                _ => 0,
            };
            let path = attribute.strip_prefix("minecraft:").unwrap_or(attribute);
            let style = style(attribute).unwrap_or_default();
            let declared_base = style.tooltip.base_modifier.as_deref() == Some(modifier.as_str());
            let base = player_base
                .and_then(|[damage, speed]| match modifier.as_str() {
                    "minecraft:base_attack_damage" => Some(damage),
                    "minecraft:base_attack_speed" => Some(speed),
                    _ => None,
                })
                .or(player_base.and(style.player_base).filter(|_| declared_base));
            if let Some(line) = modifier_line(
                translate,
                path,
                amount + base.unwrap_or(0.0),
                id,
                base.is_some(),
                style.tooltip.display,
            ) {
                let (line, color) = if base.is_some() {
                    (format!(" {line}"), rgba(0, 170, 0, 255))
                } else {
                    (
                        line,
                        attribute_color(path, *amount > 0.0, style.tooltip.sentiment),
                    )
                };
                lines.push((line, color));
                if advanced && path.contains(':') {
                    lines.push((format!("  {modifier}"), rgba(85, 85, 85, 255)));
                }
            }
        }
    }
    lines
}

/// The attribute modifiers a mob effect adds per amplifier level
/// (`MobEffects`): attribute, amount, operation id.
fn effect_modifier(effect: &str) -> Option<(&'static str, f64, usize)> {
    Some(match effect {
        "speed" => ("movement_speed", 0.2, 2),
        "slowness" => ("movement_speed", -0.15, 2),
        "strength" => ("attack_damage", 3.0, 0),
        "weakness" => ("attack_damage", -4.0, 0),
        "jump_boost" => ("safe_fall_distance", 1.0, 0),
        "luck" => ("luck", 1.0, 0),
        "invisibility" => ("waypoint_transmit_range", -1.0, 2),
        _ => return None,
    })
}

fn is_potion_item(item: &str) -> bool {
    item.ends_with("potion") || item == "minecraft:tipped_arrow"
}

/// `PlayerHeadItem.getName`: a player head whose profile has a name is titled
/// after it.
pub struct PlayerHeadName(String);

impl PlayerHeadName {
    /// The title as `getName`'s `%s's Head` translation.
    pub fn title(&self, translate: &dyn Fn(&str) -> Option<String>) -> String {
        translate("block.minecraft.player_head.named")
            .unwrap_or_else(|| "%s's Head".to_owned())
            .replacen("%s", &self.0, 1)
    }
    /// The title as a component, for command feedback.
    pub fn text(&self) -> crate::text::Text {
        crate::text::Text::translate(
            "block.minecraft.player_head.named",
            &[crate::text::Text::literal(self.0.clone())],
        )
    }
}

/// The name in a player head's `minecraft:profile`, if it has one.
pub fn player_head_name(stack: &ItemStack) -> Option<PlayerHeadName> {
    if &*stack.item != "minecraft:player_head" {
        return None;
    }
    Some(PlayerHeadName(stack.profile_name()?))
}

/// The name line of `stack`, as the item's translation or, for potions and
/// smithing templates, its name component gives it.
pub fn name_line(stack: &ItemStack, translate: &dyn Fn(&str) -> Option<String>) -> String {
    let (namespace, path) = stack
        .item
        .split_once(':')
        .unwrap_or(("minecraft", &stack.item));
    if let Some(name) = stack.custom_name() {
        return name.to_owned();
    }
    // A potion's or tipped arrow's own name comes before its `item_name`.
    let contents = crate::brewing::contents_name_key(stack);
    if contents.is_none()
        && let Some(name) = stack.item_name()
    {
        return name;
    }
    if stack.data().is_some_and(|d| d.ominous_banner) {
        return translate("block.minecraft.ominous_banner")
            .unwrap_or_else(|| "Ominous Banner".to_owned());
    }
    if let Some(head) = player_head_name(stack) {
        return head.title(translate);
    }
    let key = if contents.is_some() {
        contents
    } else if let Some(pattern) = path.strip_suffix("_armor_trim_smithing_template") {
        Some(format!("trim_pattern.minecraft.{pattern}"))
    } else if path == "netherite_upgrade_smithing_template" {
        Some("upgrade.minecraft.netherite_upgrade".to_owned())
    } else if path.ends_with("_banner_pattern") {
        // The item name component of the pattern items that have one.
        Some(format!("item.minecraft.{path}.new"))
    } else if &*stack.item == "minecraft:shield"
        && let Some(color) = stack.data().and_then(|d| d.base_color.as_deref())
    {
        // `ShieldItem.getName`: the banner's base colour names the shield.
        Some(format!("item.minecraft.shield.{color}"))
    } else {
        None
    };
    // `Item.getDescriptionId`: `Util.makeDescriptionId("item", id)`, or the
    // block's for a block item, in the item's own namespace.
    let id = format!("{namespace}.{}", path.replace('/', "."));
    key.and_then(|k| translate(&k))
        .or_else(|| translate(&format!("item.{id}")))
        .or_else(|| translate(&format!("block.{id}")))
        .unwrap_or_else(|| path.replace('_', " "))
}

/// ItemStack.getRarity: the stack's `minecraft:rarity` (the item's default
/// unless a command or server set it; the ominous banner's uncommon), then
/// the enchantment upgrade.
pub fn rarity<'a>(stack: &ItemStack, default: &'a str) -> &'a str {
    let set = match stack.data().and_then(|d| d.other("minecraft:rarity")) {
        Some(Tag::String(s)) => match &*s.to_string_lossy() {
            "common" => Some("common"),
            "uncommon" => Some("uncommon"),
            "rare" => Some("rare"),
            "epic" => Some("epic"),
            _ => None,
        },
        _ => None,
    };
    let rarity = if let Some(set) = set {
        set
    } else if stack
        .data()
        .is_some_and(|data| data.other("!minecraft:rarity").is_some())
    {
        "common"
    } else if stack.data().is_some_and(|data| data.ominous_banner) {
        "uncommon"
    } else {
        default
    };
    horizon_items::properties::ItemRarity::of(rarity, !stack.enchantments().is_empty()).as_str()
}

/// `BundleItem.getTooltipImage`: the contents grid of a bundle, or None when
/// the stack is not one or `minecraft:tooltip_display` hides
/// `minecraft:bundle_contents` - the image is that component's tooltip
/// detail, so a display that hides the component hides the grid too.
pub fn bundle_tooltip(
    items: &Items,
    stack: &ItemStack,
    x: f32,
    y: f32,
    box_width: f32,
    description_lines: usize,
) -> Option<TooltipLayout> {
    tooltip_display(stack)
        .shows("minecraft:bundle_contents")
        .then(|| crate::bundle::tooltip_layout(items, stack, x, y, box_width, description_lines))
        .flatten()
}

fn is_beehive(path: &str) -> bool {
    matches!(path, "beehive" | "bee_nest")
}

/// `InstrumentComponent.addToTooltip`: the instrument's description. A goat
/// horn without the component has the item's default, the ponder horn.
pub fn instrument_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:instrument") {
        return None;
    }
    let id = match stack.data().and_then(|d| d.other("minecraft:instrument")) {
        Some(Tag::String(id)) => id.to_string_lossy(),
        // An inline instrument carries its own description.
        Some(inline) => {
            return inline
                .get("description")
                .map(|d| crate::entity_nbt::name_text(d).plain());
        }
        None if &*stack.item == "minecraft:goat_horn" => "minecraft:ponder_goat_horn".to_owned(),
        None => return None,
    };
    let key = format!("instrument.{}", id.replacen(':', ".", 1));
    Some(translate(&key).unwrap_or(key))
}

/// `TropicalFish.COMMON_VARIANTS`: pattern, base colour and pattern colour of
/// the 22 named fish (`entity.minecraft.tropical_fish.predefined.<index>`).
const COMMON_TROPICAL_FISH: [(&str, &str, &str); 22] = [
    ("stripey", "orange", "gray"),
    ("flopper", "gray", "gray"),
    ("flopper", "gray", "blue"),
    ("clayfish", "white", "gray"),
    ("sunstreak", "blue", "gray"),
    ("kob", "orange", "white"),
    ("spotty", "pink", "light_blue"),
    ("blockfish", "purple", "yellow"),
    ("clayfish", "white", "red"),
    ("spotty", "white", "yellow"),
    ("glitter", "white", "gray"),
    ("clayfish", "white", "orange"),
    ("dasher", "cyan", "pink"),
    ("brinely", "lime", "light_blue"),
    ("betty", "red", "white"),
    ("snooper", "gray", "red"),
    ("blockfish", "red", "white"),
    ("flopper", "white", "yellow"),
    ("kob", "red", "white"),
    ("sunstreak", "gray", "white"),
    ("dasher", "cyan", "yellow"),
    ("flopper", "yellow", "yellow"),
];

/// `TropicalFish.Pattern.addToTooltip`: a bucket of tropical fish names a
/// common variant, or else shows the pattern and then the base colour (with
/// the pattern colour when it differs), all in italic gray. Only a stack that
/// has the pattern component shows it.
pub fn tropical_fish_lines(stack: &ItemStack, display: &TooltipDisplay) -> Vec<Text> {
    if !display.shows("minecraft:tropical_fish/pattern") {
        return Vec::new();
    }
    let Some(data) = stack.data() else {
        return Vec::new();
    };
    let name = |key: &str, default: &'static str| match data.other(key) {
        Some(Tag::String(s)) => s
            .to_string_lossy()
            .trim_start_matches("minecraft:")
            .to_owned(),
        _ => default.to_owned(),
    };
    let Some(Tag::String(pattern)) = data.other("minecraft:tropical_fish/pattern") else {
        return Vec::new();
    };
    let pattern = pattern
        .to_string_lossy()
        .trim_start_matches("minecraft:")
        .to_owned();
    let base = name("minecraft:tropical_fish/base_color", "white");
    let color = name("minecraft:tropical_fish/pattern_color", "white");
    let styled = |t: Text| t.italic().with_color(GRAY);
    let common = COMMON_TROPICAL_FISH
        .iter()
        .position(|(p, b, c)| *p == pattern && *b == base && *c == color);
    if let Some(index) = common {
        let key = format!("entity.minecraft.tropical_fish.predefined.{index}");
        return vec![styled(Text::translate(&key, &[]))];
    }
    let mut colors = Text::translate(&format!("color.minecraft.{base}"), &[]);
    if base != color {
        colors = colors
            .append(", ")
            .append(Text::translate(&format!("color.minecraft.{color}"), &[]));
    }
    vec![
        styled(Text::translate(
            &format!("entity.minecraft.tropical_fish.type.{pattern}"),
            &[],
        )),
        styled(colors),
    ]
}

/// `MapId.addToTooltip`: a map whose data is not known (`map` is `None`, else
/// its `locked` flag and `scale`) reads "Unknown Map"; a known one gives its
/// id (unless it has a custom name or is to be locked or scaled), "Locked"
/// when it is or is being locked, and with advanced tooltips its scale (a
/// copy being scaled one up, at most 4) and level.
pub fn map_id_lines(
    stack: &ItemStack,
    display: &TooltipDisplay,
    advanced: bool,
    map: Option<(bool, u8)>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let Some(id) = stack.data().and_then(|d| d.map_id) else {
        return Vec::new();
    };
    if !display.shows("minecraft:map_id") {
        return Vec::new();
    }
    let text = |key: &str, fallback: &str| translate(key).unwrap_or_else(|| fallback.to_owned());
    let Some((locked, scale)) = map else {
        return vec![text("filled_map.unknown", "Unknown Map")];
    };
    // MapPostProcessing: 0 lock, 1 scale.
    let post = stack.data().and_then(|d| d.map_post_processing);
    let mut lines = Vec::new();
    if stack.custom_name().is_none() && post.is_none() {
        lines.push(text("filled_map.id", "ID #%s").replacen("%s", &id.to_string(), 1));
    }
    if locked || post == Some(0) {
        lines.push(text("filled_map.locked", "Locked"));
    }
    if advanced {
        let scale = (scale + u8::from(post == Some(1))).min(4);
        lines.push(text("filled_map.scale", "Scaling at 1:%s").replacen(
            "%s",
            &(1u32 << scale).to_string(),
            1,
        ));
        lines.push(
            text("filled_map.level", "(Level %s/%s)")
                .replacen("%s", &scale.to_string(), 1)
                .replacen("%s", "4", 1),
        );
    }
    lines
}

/// `Bees.addToTooltip`: how many bees a hive item holds (a hive item without
/// the component has the item's default, none).
pub fn bees_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:bees") {
        return None;
    }
    let bees = match stack.data().and_then(|d| d.other("minecraft:bees")) {
        Some(Tag::List(bees)) => bees.len(),
        Some(_) => return None,
        None if is_beehive(stack.item.trim_start_matches("minecraft:")) => 0,
        None => return None,
    };
    let text =
        translate("container.beehive.bees").unwrap_or_else(|| "container.beehive.bees".into());
    Some(
        text.replacen("%s", &bees.to_string(), 1)
            .replacen("%s", "3", 1),
    )
}

/// `TypedEntityData.addToTooltip` for a spawn egg: in Peaceful, the red
/// "Disabled in Peaceful" under an egg whose entity type is not allowed there
/// (`EntityType.isAllowedInPeaceful`). The type is the one in the egg's
/// `entity_data` component, else the egg's own (`zombie_spawn_egg`).
/// `allowed_in_peaceful` answers for an entity type id; one it does not know
/// is allowed.
pub fn spawn_egg_peaceful_line(
    stack: &ItemStack,
    peaceful: bool,
    display: &TooltipDisplay,
    allowed_in_peaceful: &dyn Fn(&str) -> bool,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !peaceful || !display.shows("minecraft:entity_data") {
        return None;
    }
    // `TypedEntityData.addToTooltip` hangs on the `entity_data` component, whatever the item;
    // a spawn egg without the component has its own mob as the default.
    let id = match stack
        .data()
        .and_then(|d| d.other("minecraft:entity_data"))
        .and_then(|data| data.get("id"))
    {
        Some(Tag::String(id)) => {
            let id = id.to_string_lossy();
            if id.contains(':') {
                id
            } else {
                format!("minecraft:{id}")
            }
        }
        _ if stack.item.ends_with("_spawn_egg") => format!(
            "minecraft:{}",
            stack
                .item
                .trim_start_matches("minecraft:")
                .trim_end_matches("_spawn_egg")
        ),
        _ => return None,
    };
    (!allowed_in_peaceful(&id)).then(|| {
        translate("item.spawn_egg.peaceful").unwrap_or_else(|| "item.spawn_egg.peaceful".into())
    })
}

/// `BlockItemStateProperties.addToTooltip`: a hive item's honey level (a hive
/// item without the component has the item's default, 0).
pub fn honey_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:block_state") {
        return None;
    }
    let level = match stack.data().and_then(|d| d.other("minecraft:block_state")) {
        Some(Tag::Compound(state)) => {
            match state.get(&horizon_core::nbt::JavaString::from("honey_level")) {
                Some(Tag::String(level)) => level.to_string_lossy().parse::<u32>().ok()?,
                _ => return None,
            }
        }
        Some(_) => return None,
        None if is_beehive(stack.item.trim_start_matches("minecraft:")) => 0,
        None => return None,
    };
    let text =
        translate("container.beehive.honey").unwrap_or_else(|| "container.beehive.honey".into());
    Some(
        text.replacen("%s", &level.to_string(), 1)
            .replacen("%s", "5", 1),
    )
}

/// `WrittenBookContent.addToTooltip`: "by <author>" and the generation.
pub fn written_book_lines(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let Some(book) = stack
        .data()
        .and_then(|d| d.written.as_ref())
        .filter(|_| display.shows("minecraft:written_book_content"))
    else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if !book.author.trim().is_empty() {
        lines.push(
            crate::text::format_translation(
                &text("book.byAuthor"),
                &[crate::text::Text::literal(&book.author)],
            )
            .plain(),
        );
    }
    lines.push(text(&format!("book.generation.{}", book.generation)));
    lines
}

/// `JukeboxPlayable.addToTooltip`: the song's description.
pub fn jukebox_line(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:jukebox_playable") {
        return None;
    }
    let song = match stack
        .data()
        .and_then(|d| d.other("minecraft:jukebox_playable"))
    {
        Some(Tag::String(song)) => song.to_string_lossy(),
        // An inline song carries its own description.
        Some(inline) => {
            return inline
                .get("description")
                .map(|d| crate::entity_nbt::name_text(d).plain());
        }
        None => info?.jukebox_song.clone()?,
    };
    let key = format!("jukebox_song.{}", song.replacen(':', ".", 1));
    Some(translate(&key).unwrap_or(key))
}

/// `DyedItemColor.addToTooltip`: "Dyed" (italic, the flag), or with advanced
/// tooltips the colour's hex code.
pub fn dyed_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    advanced: bool,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<(String, bool)> {
    let rgb = stack.data()?.dyed_color?;
    if !display.shows("minecraft:dyed_color") {
        return None;
    }
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    Some(if advanced {
        let hex = format!("#{:06X}", rgb & 0xFF_FFFF);
        (text("item.color").replacen("%s", &hex, 1), false)
    } else {
        (text("item.dyed"), true)
    })
}

/// `ItemStack.UNBREAKABLE_TOOLTIP`.
pub fn unbreakable_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    (stack.data()?.other("minecraft:unbreakable").is_some()
        && display.shows("minecraft:unbreakable"))
    .then(|| translate("item.unbreakable").unwrap_or_else(|| "item.unbreakable".into()))
}

/// `ItemStack.addDetailsToTooltip`'s adventure lines, `can_break` then
/// `can_place_on`: each shown one adds an empty line, its gray header
/// (`AdventureModePredicate.CAN_BREAK_HEADER` / `CAN_PLACE_HEADER`) and
/// `AdventureModePredicate.computeTooltip`: `item.canUse.unknown` in gray when
/// some predicate names no blocks, else every distinct block of the
/// predicates' holder sets by its name (`Block.getName`) in dark gray.
/// `tag_blocks` lists the blocks of a block tag.
pub fn adventure_lines(
    stack: &ItemStack,
    display: &TooltipDisplay,
    tag_blocks: &dyn Fn(&str) -> Vec<String>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let namespaced = |id: &str| {
        if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        }
    };
    let (gray, dark_gray) = (rgba(170, 170, 170, 255), rgba(85, 85, 85, 255));
    let mut lines = Vec::new();
    for (component, header) in [
        ("minecraft:can_break", "item.canBreak"),
        ("minecraft:can_place_on", "item.canPlace"),
    ] {
        let Some(tag) = stack.data().and_then(|d| d.other(component)) else {
            continue;
        };
        if !display.shows(component) {
            continue;
        }
        lines.push((String::new(), u32::MAX));
        lines.push((text(header), gray));
        let predicates: Vec<&Tag> = match tag {
            Tag::List(list) => list.iter().collect(),
            single => vec![single],
        };
        if predicates.iter().any(|p| p.get("blocks").is_none()) {
            lines.push((text("item.canUse.unknown"), gray));
            continue;
        }
        let mut blocks: Vec<String> = Vec::new();
        for p in predicates {
            let ids = match p.get("blocks") {
                Some(Tag::String(entry)) => {
                    let entry = entry.to_string_lossy();
                    match entry.strip_prefix('#') {
                        Some(tag) => tag_blocks(&namespaced(tag)),
                        None => vec![namespaced(&entry)],
                    }
                }
                Some(Tag::List(entries)) => entries
                    .iter()
                    .filter_map(|e| match e {
                        Tag::String(id) => Some(namespaced(&id.to_string_lossy())),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            for id in ids {
                if !blocks.contains(&id) {
                    blocks.push(id);
                }
            }
        }
        for id in blocks {
            lines.push((
                text(&format!("block.{}", id.replacen(':', ".", 1))),
                dark_gray,
            ));
        }
    }
    lines
}

/// `OminousBottleAmplifier.addToTooltip`: the effect the bottle gives,
/// `MobEffectInstance(BAD_OMEN, 120000, amplifier)`, listed as
/// `PotionContents.addPotionTooltip` lists it, in the colour of Bad Omen's
/// category (NEUTRAL, blue). An ominous bottle without the component has the
/// item's default amplifier, 0.
pub fn ominous_bottle_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<(String, u32)> {
    const BAD_OMEN_TICKS: i32 = 120_000;
    let amplifier = stack
        .data()
        .and_then(|d| d.ominous_amplifier)
        .or_else(|| (&*stack.item == "minecraft:ominous_bottle").then_some(0))?;
    if !display.shows("minecraft:ominous_bottle_amplifier") {
        return None;
    }
    use crate::text::{Text, format_translation};
    let text = |key: &str, fallback: &str| translate(key).unwrap_or_else(|| fallback.to_owned());
    // PotionContents.getPotionDescription.
    let mut line = Text::literal(text(
        "effect.minecraft.bad_omen",
        "effect.minecraft.bad_omen",
    ));
    if amplifier > 0 {
        let potency = translate(&format!("potion.potency.{amplifier}"))
            .unwrap_or_else(|| crate::enchant::level_name(u32::from(amplifier) + 1));
        line = format_translation(
            &text("potion.withAmplifier", "%s %s"),
            &[line, Text::literal(potency)],
        );
    }
    let duration = Text::literal(crate::brewing::format_duration(BAD_OMEN_TICKS));
    let line = format_translation(&text("potion.withDuration", "%s (%s)"), &[line, duration]);
    Some((line.plain(), rgba(85, 85, 255, 255)))
}

/// `PotionContents.addPotionTooltip`'s attribute block: an empty line, "When
/// Applied:" and the modifiers of the potion's effects (blue for a gain, red
/// for a loss); nothing for a potion whose effects add none.
pub fn potion_modifier_lines(
    stack: &ItemStack,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    if !is_potion_item(&stack.item) {
        return Vec::new();
    }
    let effects = crate::brewing::contents_effects(stack);
    effect_modifier_lines(
        effects
            .iter()
            .map(|(effect, _, amplifier)| (*effect, *amplifier)),
        translate,
    )
}

/// `addPotionTooltip`'s attribute block for (effect id, amplifier) entries.
fn effect_modifier_lines<'a>(
    effects: impl IntoIterator<Item = (&'a str, i32)>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let applied: Vec<(String, u32)> = effects
        .into_iter()
        .filter_map(|(effect, amplifier)| {
            let (attribute, amount, operation) =
                effect_modifier(effect.trim_start_matches("minecraft:"))?;
            let amount = amount * (amplifier + 1) as f64;
            let line = modifier_line(
                translate,
                attribute,
                amount,
                operation,
                false,
                Display::Ranged,
            )?;
            let color = if amount > 0.0 {
                rgba(85, 85, 255, 255)
            } else {
                rgba(255, 85, 85, 255)
            };
            Some((line, color))
        })
        .collect();
    if applied.is_empty() {
        return applied;
    }
    let mut lines = vec![
        (String::new(), u32::MAX),
        (
            translate("potion.whenDrank").unwrap_or_else(|| "potion.whenDrank".into()),
            rgba(170, 0, 170, 255),
        ),
    ];
    lines.extend(applied);
    lines
}

/// `SuspiciousStewEffects.addToTooltip`: only with the creative tooltip flag,
/// the stew's effects as `PotionContents.addPotionTooltip` lists them (each in
/// its `MobEffectCategory` colour, "No Effects" for none), then their
/// attribute modifiers.
pub fn stew_lines(
    stack: &ItemStack,
    creative: bool,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let effects = stack.data().map_or(&[][..], |d| &d.stew_effects[..]);
    // The component is the item's default (empty) on a suspicious stew.
    if !creative
        || (effects.is_empty() && &*stack.item != "minecraft:suspicious_stew")
        || !display.shows("minecraft:suspicious_stew_effects")
    {
        return Vec::new();
    }
    if effects.is_empty() {
        let none = translate("effect.none").unwrap_or_else(|| "No Effects".into());
        return vec![(none, rgba(170, 170, 170, 255))];
    }
    // `Entry.createEffectInstance`: amplifier 0.
    let entries = effects.iter().map(|(id, duration)| (&**id, *duration, 0));
    let mut lines: Vec<(String, u32)> = crate::brewing::effect_lines(entries, 1.0, translate)
        .into_iter()
        .map(|(line, category)| {
            // MobEffectCategory.getTooltipFormatting: harmful red, else blue.
            let color = match category {
                crate::brewing::Category::Harmful => rgba(255, 85, 85, 255),
                _ => rgba(85, 85, 255, 255),
            };
            (line, color)
        })
        .collect();
    lines.extend(effect_modifier_lines(
        effects.iter().map(|(id, _)| (&**id, 0)),
        translate,
    ));
    lines
}

/// `ItemStack.components.size()`: the item's default components with the
/// stack's own changes applied.
pub fn component_count(stack: &ItemStack, info: Option<&ItemInfo>) -> usize {
    let mut keys: std::collections::HashSet<String> = info
        .map(|i| i.components.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(Tag::Compound(patch)) = crate::item_data::components_tag(stack.damage, stack.data())
    {
        for key in patch.keys() {
            let key = key.to_string_lossy();
            match key.strip_prefix('!') {
                Some(removed) => keys.remove(removed),
                None => keys.insert(key),
            };
        }
    }
    keys.len()
}

/// Where each component detail of a tooltip starts, for the placements of the appenders a mod
/// registered. Recorded only when some appender is registered: otherwise it is empty.
pub struct DetailMarks {
    /// Each detail's component and the line it starts at, in the order they are listed.
    starts: Option<Vec<(&'static str, usize)>>,
    /// The line the details end at, before the durability and advanced lines.
    finish: usize,
}

impl DetailMarks {
    /// Marks for one tooltip; `registered` is whether any appender is registered.
    pub fn new(registered: bool) -> Self {
        Self {
            starts: registered.then(Vec::new),
            finish: 0,
        }
    }
    /// The detail of component `component` starts before line `at`.
    pub fn start(&mut self, component: &'static str, at: usize) {
        if let Some(starts) = &mut self.starts {
            starts.push((component, at));
        }
    }
    /// The details end before line `at`.
    pub fn end(&mut self, at: usize) {
        self.finish = at;
    }
}

/// The line and logical component boundary for a placement, with a rank at the same boundary.
fn anchor(
    placement: &TooltipPlacement,
    starts: &[(&str, usize)],
    end: usize,
) -> (usize, usize, u8) {
    let start_of = |id: &str| starts.iter().position(|(component, _)| *component == id);
    match placement {
        TooltipPlacement::BeforeAll => (starts.first().map_or(end, |s| s.1), 0, 0),
        TooltipPlacement::Before(id) => {
            let i = start_of(id.as_str()).unwrap_or(starts.len());
            (starts.get(i).map_or(end, |s| s.1), i, 1)
        }
        TooltipPlacement::After(id) => {
            let i = start_of(id.as_str()).map_or(starts.len(), |i| i + 1);
            (starts.get(i).map_or(end, |s| s.1), i, 0)
        }
        TooltipPlacement::AfterAll => (end, starts.len(), 2),
    }
}

/// `RegisterTooltipAppendersEvent`: adds the lines of each registered appender where its
/// placement puts them. `registered` yields (component, placement, its mod enabled, appender) in
/// registration order; an appender is asked only for a stack that carries its component and
/// whose tooltip `display` shows it.
pub fn add_appender_lines<'a, A: ?Sized + 'a>(
    lines: &mut Vec<(Text, u32)>,
    marks: &DetailMarks,
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    append: impl Fn(&A, &ItemStack, &mut Vec<(Text, u32)>),
    registered: impl Iterator<Item = (&'a str, &'a TooltipPlacement, bool, &'a A)>,
) {
    let starts = marks.starts.as_deref().unwrap_or(&[]);
    let display = tooltip_display(stack);
    let mut added = Vec::new();
    for (seq, (component, placement, enabled, appender)) in registered.enumerate() {
        if !enabled || !display.shows(component) || !stack.carries_component(info, component) {
            continue;
        }
        let mut out = Vec::new();
        append(appender, stack, &mut out);
        let (at, boundary, rank) = anchor(placement, starts, marks.finish);
        added.push((at, boundary, rank, seq, out));
    }
    if added.is_empty() {
        return;
    }
    added.sort_by_key(|(at, boundary, rank, seq, _)| (*at, *boundary, *rank, *seq));
    let mut added = added.into_iter().peekable();
    let mut merged = Vec::with_capacity(lines.len());
    for (index, line) in std::mem::take(lines).into_iter().enumerate() {
        while let Some((_, _, _, _, out)) = added.next_if(|(at, ..)| *at <= index) {
            merged.extend(out);
        }
        merged.push(line);
    }
    merged.extend(added.flat_map(|(.., out)| out));
    *lines = merged;
}

/// `SmithingTemplateItem.appendHoverText`: the template's kind, what it applies to
/// and its ingredients (gray titles over blue descriptions); none for other items.
pub fn smithing_template_lines(
    stack: &ItemStack,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let path = stack.item.trim_start_matches("minecraft:");
    if !path.ends_with("_smithing_template") {
        return Vec::new();
    }
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let kind = if path == "netherite_upgrade_smithing_template" {
        "netherite_upgrade"
    } else {
        "armor_trim"
    };
    let (title, description) = (rgba(170, 170, 170, 255), rgba(85, 85, 255, 255));
    let described = |what: &str| {
        let key = format!("item.minecraft.smithing_template.{kind}.{what}");
        (format!(" {}", text(&key)), description)
    };
    vec![
        (text("item.minecraft.smithing_template"), title),
        (String::new(), u32::MAX),
        (text("item.minecraft.smithing_template.applies_to"), title),
        described("applies_to"),
        (text("item.minecraft.smithing_template.ingredients"), title),
        described("ingredients"),
    ]
}

/// `DiscFragmentItem.appendHoverText`: the gray "Music Disc - 5" line; none for
/// other items.
pub fn disc_fragment_line(
    stack: &ItemStack,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if &*stack.item != "minecraft:disc_fragment_5" {
        return None;
    }
    let key = "item.minecraft.disc_fragment_5.desc";
    Some(translate(key).unwrap_or_else(|| key.to_owned()))
}

/// The enchantment components in `ItemStack.addDetailsToTooltip` order.
pub fn enchantment_lines(
    stack: &ItemStack,
    display: &TooltipDisplay,
    enchantments: &Enchantments,
) -> Vec<(String, bool)> {
    let mut lines = Vec::new();
    if display.shows("minecraft:stored_enchantments") {
        lines.extend(enchantments.tooltip(stack.stored_enchantments()));
    }
    if display.shows("minecraft:enchantments") {
        lines.extend(enchantments.tooltip(stack.enchantments()));
    }
    lines
}

/// The lines of `stack` below its name that its components give, in
/// `ItemStack.addDetailsToTooltip` order. `style` answers each attribute's declared tooltip
/// style, as [`attribute_tooltip`]'s does.
pub fn detail_lines(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    attributes: &dyn AttributeTooltipListener,
    enchantments: &Enchantments,
    translate: &dyn Fn(&str) -> Option<String>,
    style: &dyn Fn(&str) -> Option<AttributeStyle>,
) -> Vec<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let path = stack.item.trim_start_matches("minecraft:");
    let mut lines = Vec::new();
    lines.extend(
        smithing_template_lines(stack, translate)
            .into_iter()
            .map(|l| l.0),
    );
    lines.extend(disc_fragment_line(stack, translate));
    let display = tooltip_display(stack);
    lines.extend(bees_line(stack, &display, translate));
    lines.extend(honey_line(stack, &display, translate));
    let data = stack.data();
    // PotDecorations.addToTooltip.
    if let Some(sherds) = data
        .and_then(|d| d.pot_decorations.as_ref())
        .filter(|s| s.iter().any(Option::is_some))
    {
        lines.push(String::new());
        for sherd in sherds.iter() {
            let item = sherd.as_ref().map_or("minecraft:brick", |s| &*s.item);
            lines.push(name_line(&ItemStack::new(item, 1), translate));
        }
    }
    lines.extend(firework_lines(stack, translate));
    // PotionContents.addToTooltip: the effects, then their attribute modifiers.
    if is_potion_item(&stack.item) {
        let scale = match path {
            "lingering_potion" => 0.25,
            "tipped_arrow" => 0.125,
            _ => 1.0,
        };
        lines.extend(
            crate::brewing::tooltip(stack, scale, translate)
                .into_iter()
                .map(|l| l.0),
        );
        lines.extend(
            potion_modifier_lines(stack, translate)
                .into_iter()
                .map(|l| l.0),
        );
    }
    lines.extend(jukebox_line(stack, info, &display, translate));
    // Trim.addToTooltip.
    if let Some((material, pattern)) = data
        .and_then(|d| d.trim.as_ref())
        .and_then(crate::item_data::trim::named)
    {
        let short = |s: &str| s.trim_start_matches("minecraft:").to_owned();
        lines.push(text("item.minecraft.smithing_template.upgrade"));
        lines.push(format!(
            " {}",
            text(&format!("trim_pattern.minecraft.{}", short(pattern)))
        ));
        lines.push(format!(
            " {}",
            text(&format!("trim_material.minecraft.{}", short(material)))
        ));
    }
    lines.extend(
        enchantment_lines(stack, &display, enchantments)
            .into_iter()
            .map(|(text, _)| text),
    );
    lines.extend(dyed_line(stack, &display, false, translate).map(|l| l.0));
    let modifiers =
        crate::item_attributes::attribute_modifiers_or(stack, info.map_or(&[], |i| &i.modifiers));
    // Recipe search has no player, no advanced flag and no creative slot.
    let ctx = AttributeTooltipContext {
        player_base: None,
        advanced: false,
        creative: false,
    };
    let (modifier_lines, added) = attribute_tooltip(
        stack, &modifiers, &ctx,
        // The search text lists attribute lines whatever the stack's display says, as before.
        true, attributes, translate, style,
    );
    lines.extend(modifier_lines.into_iter().map(|(line, _)| line));
    lines.extend(added.into_iter().map(|(text, _)| text.plain()));
    lines.extend(ominous_bottle_line(stack, &display, translate).map(|l| l.0));
    lines
}

/// `Fireworks.addToTooltip` (a rocket's flight duration, its explosions with
/// runs of equal ones counted) and `FireworkExplosion.addToTooltip` (a
/// star's), unless the stack's tooltip display hides them. A rocket without
/// the component has the item's default: flight duration 1, no explosions.
pub fn firework_lines(
    stack: &ItemStack,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    use crate::particles::FireworkExplosion;
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let shape = |e: &FireworkExplosion| {
        text(&format!(
            "item.minecraft.firework_star.shape.{}",
            e.shape.name()
        ))
    };
    // FireworkExplosion.appendColors: dye names (getColorName), comma-separated.
    let colors = |colors: &[u32]| {
        colors
            .iter()
            .map(|c| {
                match crate::particles::DYE_FIREWORK_COLORS
                    .iter()
                    .find(|(_, firework)| firework == c)
                {
                    Some((name, _)) => text(&format!("item.minecraft.firework_star.{name}")),
                    None => text("item.minecraft.firework_star.custom_color"),
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    // FireworkExplosion.addAdditionalTooltip.
    let details = |e: &FireworkExplosion, indent: &str, lines: &mut Vec<String>| {
        if !e.colors.is_empty() {
            lines.push(format!("{indent}{}", colors(&e.colors)));
        }
        if !e.fade.is_empty() {
            let fade = text("item.minecraft.firework_star.fade_to");
            lines.push(format!("{indent}{fade} {}", colors(&e.fade)));
        }
        if e.trail {
            lines.push(format!(
                "{indent}{}",
                text("item.minecraft.firework_star.trail")
            ));
        }
        if e.twinkle {
            lines.push(format!(
                "{indent}{}",
                text("item.minecraft.firework_star.flicker")
            ));
        }
    };
    let display = tooltip_display(stack);
    let data = stack.data();
    let mut lines = Vec::new();
    let fireworks = match data.and_then(|d| d.fireworks.as_ref()) {
        Some((flight, explosions)) => Some((*flight, explosions.as_slice())),
        None => (&*stack.item == "minecraft:firework_rocket").then_some((1, &[][..])),
    };
    if let Some((flight, explosions)) = fireworks
        && display.shows("minecraft:fireworks")
    {
        if flight > 0 {
            lines.push(
                text("item.minecraft.firework_rocket.flight_duration").replacen(
                    "%s",
                    &flight.to_string(),
                    1,
                ),
            );
        }
        // Fireworks.addExplosionTooltip for each run of equal explosions.
        for run in explosions.chunk_by(|a, b| a == b) {
            let e = &run[0];
            lines.push(if run.len() == 1 {
                text("item.minecraft.firework_rocket.single_star").replacen("%s", &shape(e), 1)
            } else {
                text("item.minecraft.firework_rocket.multiple_stars")
                    .replacen("%s", &run.len().to_string(), 1)
                    .replacen("%s", &shape(e), 1)
            });
            details(e, "  ", &mut lines);
        }
    }
    if let Some(e) = data.and_then(|d| d.firework_explosion.as_ref())
        && display.shows("minecraft:firework_explosion")
    {
        lines.push(shape(e));
        details(e, "", &mut lines);
    }
    lines
}

/// `ItemContainerContents.addToTooltip`: "count x name" for the first five
/// stacks, by slot, then how many more there are (an italic line).
pub fn container_lines(
    stack: &ItemStack,
    name: &dyn Fn(&ItemStack) -> String,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let Some(data) = stack.data() else {
        return Vec::new();
    };
    let mut slots: Vec<&(u8, ItemStack)> = data.container.iter().collect();
    slots.sort_by_key(|(slot, _)| *slot);
    let count = translate("item.container.item_count").unwrap_or_else(|| "%s x %s".into());
    let mut lines: Vec<String> = slots
        .iter()
        .take(5)
        .map(|(_, s)| horizon_protocol::text::format_java(&count, &[s.count.to_string(), name(s)]))
        .collect();
    if slots.len() > 5 {
        let more =
            translate("item.container.more_items").unwrap_or_else(|| "and %s more...".into());
        lines.push(horizon_protocol::text::format_java(
            &more,
            &[(slots.len() - 5).to_string()],
        ));
    }
    lines
}

/// Every line of `stack`'s tooltip, name first. `style` is as [`detail_lines`]'s.
pub fn lines(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    attributes: &dyn AttributeTooltipListener,
    enchantments: &Enchantments,
    translate: &dyn Fn(&str) -> Option<String>,
    style: &dyn Fn(&str) -> Option<AttributeStyle>,
) -> Vec<String> {
    let mut lines = vec![name_line(stack, translate)];
    if tooltip_display(stack).shows("minecraft:lore") {
        lines.extend(lore_texts(stack).into_iter().map(|t| t.plain()));
    }
    lines.extend(detail_lines(
        stack,
        info,
        attributes,
        enchantments,
        translate,
        style,
    ));
    lines
}

/// `ItemLore.addToTooltip`: each lore component under dark purple italic, with
/// the component's own styles (including explicit `false`) winning on children.
pub fn lore_texts(stack: &ItemStack) -> Vec<crate::text::Text> {
    let Some(raw) = stack.data().and_then(|d| d.other("minecraft:lore")) else {
        return Vec::new();
    };
    let base = crate::text::Style {
        color: Some(crate::text::DARK_PURPLE),
        italic: Some(true),
        ..Default::default()
    };
    let lines: Vec<crate::text::Text> = match raw {
        Tag::List(entries) => entries
            .iter()
            .map(|entry| crate::entity_nbt::name_text(entry).resolve_legacy())
            .collect(),
        Tag::Compound(_) | Tag::String(_) => {
            vec![crate::entity_nbt::name_text(raw).resolve_legacy()]
        }
        _ => Vec::new(),
    };
    lines
        .into_iter()
        .map(|text| text.with_style(&base))
        .collect()
}

/// Preserve child colours and explicit styles in the stored hover name.
/// A custom name inherits italic from `ItemStack.getStyledHoverName`.
pub fn styled_name(stack: &ItemStack, fallback: impl FnOnce() -> String) -> crate::text::Text {
    if let Some(component) = stack.data().and_then(|d| d.custom_name.as_ref()) {
        return crate::entity_nbt::name_text(component).with_style(&crate::text::Style {
            italic: Some(true),
            ..Default::default()
        });
    }
    // A potion or tipped arrow is named by its contents, not its `item_name`.
    if crate::brewing::contents_name_key(stack).is_none()
        && let Some(name) = stack.item_name_text()
    {
        return name;
    }
    crate::text::Text::untrusted_legacy(&fallback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::loot::PackOverlay;
    use std::sync::Arc;

    #[test]
    fn disc_fragment_shows_music_disc_line_first() {
        let enchantments = Enchantments::default();
        let translate = |k: &str| {
            (k == "item.minecraft.disc_fragment_5.desc").then(|| "Music Disc - 5".to_owned())
        };
        let stack = ItemStack::new("minecraft:disc_fragment_5", 1);
        assert_eq!(
            disc_fragment_line(&stack, &translate).as_deref(),
            Some("Music Disc - 5")
        );
        let lines = detail_lines(
            &stack,
            None,
            &NoAttributeListener,
            &enchantments,
            &translate,
            &|_| None,
        );
        assert_eq!(lines.first().map(String::as_str), Some("Music Disc - 5"));
        let other = ItemStack::new("minecraft:stick", 1);
        assert_eq!(disc_fragment_line(&other, &translate), None);
        assert!(
            !detail_lines(
                &other,
                None,
                &NoAttributeListener,
                &enchantments,
                &translate,
                &|_| None,
            )
            .iter()
            .any(|l| l == "Music Disc - 5")
        );
    }

    #[test]
    fn styled_name_keeps_gradient_spans_and_explicit_italic() {
        let component = Tag::compound([
            ("text", Tag::String("".into())),
            (
                "extra",
                Tag::List(vec![
                    Tag::compound([
                        ("text", Tag::String("A".into())),
                        ("color", Tag::String("#ff0000".into())),
                        ("italic", Tag::Byte(0)),
                    ]),
                    Tag::compound([
                        ("text", Tag::String("B".into())),
                        ("color", Tag::String("#0000ff".into())),
                    ]),
                ]),
            ),
        ]);
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().custom_name = Some(component);
        let text = styled_name(&stack, || "unused".into());
        assert_eq!(text.plain(), "AB");
        assert_eq!(text.spans[0].style.color, Some(0xff0000));
        assert_eq!(text.spans[1].style.color, Some(0x0000ff));
        assert!(!text.spans[0].style.is_italic());
        assert!(text.spans[1].style.is_italic());
    }

    #[test]
    fn lore_retains_translation_component() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().set_other(
            "minecraft:lore",
            Some(Tag::List(vec![Tag::compound([
                ("translate", Tag::String("example.lore".into())),
                ("with", Tag::List(vec![Tag::String("argument".into())])),
            ])])),
        );
        let lines = lore_texts(&stack);
        // Decoding canonicalizes string arguments to literal text components.
        assert_eq!(
            lines[0].component(),
            serde_json::json!({
                "translate": "example.lore",
                "with": [{"text": "argument"}],
                "color": "dark_purple",
                "italic": true
            })
        );
        assert_eq!(
            lines[0].spans.last().unwrap().style.color,
            Some(crate::text::DARK_PURPLE)
        );
    }

    #[test]
    fn lore_keeps_component_colours_under_dark_purple_italic() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().set_other(
            "minecraft:lore",
            Some(Tag::List(vec![
                Tag::compound([
                    ("text", Tag::String("Gold line".into())),
                    ("color", Tag::String("gold".into())),
                    ("italic", Tag::Byte(0)),
                ]),
                Tag::String("Plain lore".into()),
            ])),
        );
        let lines = lore_texts(&stack);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].plain(), "Gold line");
        assert_eq!(lines[0].spans[0].style.color, Some(crate::text::GOLD));
        assert!(!lines[0].spans[0].style.is_italic());
        assert_eq!(lines[1].plain(), "Plain lore");
        assert_eq!(
            lines[1].spans[0].style.color,
            Some(crate::text::DARK_PURPLE)
        );
        assert!(lines[1].spans[0].style.is_italic());
        assert!(tooltip_display(&stack).shows("minecraft:lore"));
        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:lore".into())]),
            )])),
        );
        assert!(!tooltip_display(&stack).shows("minecraft:lore"));
    }

    #[test]
    fn tooltip_display_controls_name_and_component_visibility() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        let display = tooltip_display(&stack);
        assert!(display.name_visible(false));
        assert!(display.shows("minecraft:attribute_modifiers"));

        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
            )])),
        );
        let display = tooltip_display(&stack);
        assert!(display.name_visible(false));
        assert!(!display.shows("minecraft:attribute_modifiers"));
        assert!(display.shows("minecraft:enchantments"));

        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([("hide_tooltip", Tag::Byte(1))])),
        );
        let display = tooltip_display(&stack);
        assert!(!display.name_visible(false));
        assert!(display.name_visible(true));
        assert!(!display.shows("minecraft:attribute_modifiers"));
    }

    /// `BundleItem.getTooltipImage` is a tooltip detail of
    /// `minecraft:bundle_contents`, so a `minecraft:tooltip_display` that
    /// hides the component - or the whole tooltip - takes the contents grid
    /// with it. The draw path asks for the grid through the same function.
    #[test]
    fn a_bundle_contents_grid_follows_the_tooltip_display() {
        // The grid is `BundleItem`'s: the table knows the bundle item.
        let mut items = Items::for_tests(Default::default());
        items.items.insert(
            "minecraft:bundle".into(),
            crate::items::ItemInfo {
                bundle: true,
                max_stack: 1,
                ..Default::default()
            },
        );
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = vec![ItemStack::new("minecraft:stone", 3)];
        assert!(bundle_tooltip(&items, &bundle, 0.0, 0.0, 96.0, 2).is_some());
        // A stack that is not a bundle has no grid.
        let stone = ItemStack::new("minecraft:stone", 1);
        assert!(bundle_tooltip(&items, &stone, 0.0, 0.0, 96.0, 2).is_none());
        // A display that hides the contents component hides the grid.
        bundle.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:bundle_contents".into())]),
            )])),
        );
        assert!(bundle_tooltip(&items, &bundle, 0.0, 0.0, 96.0, 2).is_none());
        // So does one that hides the whole tooltip.
        bundle.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([("hide_tooltip", Tag::Byte(1))])),
        );
        assert!(bundle_tooltip(&items, &bundle, 0.0, 0.0, 96.0, 2).is_none());
        // A component the display does not name leaves the grid alone.
        bundle.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:lore".into())]),
            )])),
        );
        assert!(bundle_tooltip(&items, &bundle, 0.0, 0.0, 96.0, 2).is_some());
    }

    #[test]
    fn stew_effects_show_only_with_the_creative_flag() {
        let translate = |key: &str| {
            Some(
                match key {
                    "effect.minecraft.fire_resistance" => "Fire Resistance",
                    "effect.minecraft.weakness" => "Weakness",
                    "effect.minecraft.saturation" => "Saturation",
                    "effect.none" => "No Effects",
                    "potion.whenDrank" => "When Applied:",
                    "attribute.modifier.take.0" => "-%s %s",
                    "attribute.name.attack_damage" => "Attack Damage",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let (blue, red) = (rgba(85, 85, 255, 255), rgba(255, 85, 85, 255));
        let lines = |stack: &ItemStack, creative: bool| {
            stew_lines(stack, creative, &tooltip_display(stack), &translate)
        };
        let mut stew = ItemStack::new("minecraft:suspicious_stew", 1);
        // The item's default, an empty list.
        assert_eq!(
            lines(&stew, true),
            [("No Effects".to_owned(), rgba(170, 170, 170, 255))]
        );
        stew.data_mut().stew_effects = vec![
            (Arc::from("minecraft:fire_resistance"), 60),
            (Arc::from("minecraft:weakness"), 140),
            (Arc::from("minecraft:saturation"), 7),
        ];
        assert!(lines(&stew, false).is_empty());
        assert_eq!(
            lines(&stew, true),
            [
                ("Fire Resistance (00:03)".to_owned(), blue),
                ("Weakness (00:07)".to_owned(), red),
                // 20 ticks or less: no duration.
                ("Saturation".to_owned(), blue),
                (String::new(), u32::MAX),
                ("When Applied:".to_owned(), rgba(170, 0, 170, 255)),
                ("-4 Attack Damage".to_owned(), red),
            ]
        );
        assert!(lines(&ItemStack::new("minecraft:bowl", 1), true).is_empty());
    }

    /// `PotionContents.addPotionTooltip` collects the attribute modifiers of
    /// every effect, the stack's custom ones included.
    #[test]
    fn a_custom_effects_attribute_modifier_is_listed() {
        let translate = |key: &str| match key {
            "potion.whenDrank" => Some("When Applied:".to_owned()),
            "attribute.modifier.plus.0" => Some("+%s %s".to_owned()),
            "attribute.name.attack_damage" => Some("Attack Damage".to_owned()),
            _ => None,
        };
        let mut potion = ItemStack::new("minecraft:potion", 1);
        assert!(potion_modifier_lines(&potion, &translate).is_empty());
        potion.data_mut().potion_extra = vec![(
            "custom_effects".into(),
            Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:strength".into())),
                ("duration", Tag::Int(600)),
            ])]),
        )];
        let lines: Vec<_> = potion_modifier_lines(&potion, &translate)
            .into_iter()
            .map(|l| l.0)
            .collect();
        assert_eq!(lines, ["", "When Applied:", "+3 Attack Damage"]);
    }

    #[test]
    fn ominous_bottle_lists_bad_omen_at_its_amplifier_for_100_minutes() {
        let enchantments = Enchantments::default();
        let lang = |key: &str| {
            Some(
                match key {
                    "effect.minecraft.bad_omen" => "Bad Omen",
                    "potion.withAmplifier" => "%s %s",
                    "potion.withDuration" => "%s (%s)",
                    "potion.potency.3" => "IV",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let duration = crate::brewing::format_duration(120_000);
        let blue = rgba(85, 85, 255, 255);
        // The item's default amplifier, 0: no potency.
        let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
        let line = |s: &ItemStack| ominous_bottle_line(s, &tooltip_display(s), &lang);
        assert_eq!(
            line(&bottle),
            Some((format!("Bad Omen ({duration})"), blue))
        );
        bottle.data_mut().ominous_amplifier = Some(3);
        assert_eq!(
            line(&bottle),
            Some((format!("Bad Omen IV ({duration})"), blue))
        );
        assert!(
            lines(
                &bottle,
                None,
                &NoAttributeListener,
                &enchantments,
                &lang,
                &|_| None
            )
            .contains(&format!("Bad Omen IV ({duration})")),
            "{:?}",
            lines(
                &bottle,
                None,
                &NoAttributeListener,
                &enchantments,
                &lang,
                &|_| None
            )
        );
        // A numbered template, as some languages write it.
        let numbered = |key: &str| match key {
            "potion.withDuration" => Some("%2$s: %1$s".to_owned()),
            other => lang(other),
        };
        assert_eq!(
            ominous_bottle_line(&bottle, &tooltip_display(&bottle), &numbered).map(|l| l.0),
            Some(format!("{duration}: Bad Omen IV"))
        );
        bottle.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String(
                    "minecraft:ominous_bottle_amplifier".into(),
                )]),
            )])),
        );
        assert_eq!(line(&bottle), None);
        assert_eq!(line(&ItemStack::new("minecraft:glass_bottle", 1)), None);
    }

    #[test]
    fn ominous_banner_keeps_its_default_hidden_pattern_tooltip() {
        let mut banner = crate::mobs::raider::ominous_banner();
        let display = tooltip_display(&banner);
        assert!(!display.shows("minecraft:banner_patterns"));

        banner.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::Compound(Default::default())),
        );
        let display = tooltip_display(&banner);
        assert!(display.shows("minecraft:banner_patterns"));
    }

    #[test]
    fn ominous_banner_name_and_rarity_use_the_stack() {
        let translate = |key: &str| match key {
            "block.minecraft.ominous_banner" => Some("Ominous Banner".to_owned()),
            "block.minecraft.white_banner" => Some("White Banner".to_owned()),
            _ => None,
        };
        let mut banner = crate::mobs::raider::ominous_banner();
        assert_eq!(name_line(&banner, &translate), "Ominous Banner");
        assert_eq!(name_line(&banner, &|_| None), "Ominous Banner");
        assert_eq!(rarity(&banner, "common"), "uncommon");
        banner.data_mut().custom_name = Some(Tag::String("Captain's Standard".into()));
        assert_eq!(name_line(&banner, &translate), "Captain's Standard");
        let mut custom_item = ItemStack::new("minecraft:diamond_sword", 1);
        custom_item.data_mut().other.push((
            "minecraft:item_name".into(),
            Tag::String("Excalibur".into()),
        ));
        assert_eq!(name_line(&custom_item, &translate), "Excalibur");
        custom_item.data_mut().custom_name = Some(Tag::String("Custom Blade".into()));
        assert_eq!(name_line(&custom_item, &translate), "Custom Blade");
        banner.enchant("minecraft:unbreaking", 1);
        assert_eq!(rarity(&banner, "common"), "rare");
        let plain = ItemStack::new("minecraft:white_banner", 1);
        assert_eq!(name_line(&plain, &translate), "White Banner");
        assert_eq!(rarity(&plain, "common"), "common");
    }

    /// `/give @a stick[rarity=epic,enchantments={aqua_affinity:1}]`: the
    /// stack's rarity wins over the item's, and stays epic when enchanted.
    #[test]
    fn a_stacks_rarity_component_sets_its_rarity() {
        let mut stick = ItemStack::new("minecraft:stick", 1);
        stick
            .data_mut()
            .other
            .push(("minecraft:rarity".into(), Tag::String("epic".into())));
        assert_eq!(rarity(&stick, "common"), "epic");
        stick.enchant("minecraft:aqua_affinity", 1);
        assert_eq!(rarity(&stick, "common"), "epic");
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword
            .data_mut()
            .other
            .push(("minecraft:rarity".into(), Tag::String("uncommon".into())));
        sword.enchant("minecraft:sharpness", 1);
        assert_eq!(rarity(&sword, "common"), "rare");
    }

    #[test]
    fn lore_texts_handles_compound_lists() {
        // Direct command components from issue #6405
        let cmd = r#"[custom_name=[{"text":"Simple Custom Name","italic":false}],lore=[[{"text":"Simple Custom Lore","italic":false}]],rarity=epic,enchantments={aqua_affinity:1},unbreakable={}]"#;
        let comp = horizon_items::item_data::command_components(cmd).unwrap();
        let (damage, extra) = horizon_items::item_data::decode_components(Some(&comp));
        let stack = ItemStack {
            item: "minecraft:stick".into(),
            count: 1,
            damage,
            extra,
        };
        assert_eq!(rarity(&stack, "common"), "epic");
        let name = styled_name(&stack, || "fallback".into());
        assert_eq!(name.plain(), "Simple Custom Name");
        assert_eq!(name.spans[0].style.italic, Some(false));

        let lore = lore_texts(&stack);
        assert_eq!(lore.len(), 1);
        assert_eq!(lore[0].plain(), "Simple Custom Lore");
        assert_eq!(lore[0].spans[0].style.italic, Some(false));
        assert_eq!(lore[0].spans[0].style.color, Some(crate::text::DARK_PURPLE));
        assert!(tooltip_display(&stack).shows("minecraft:lore"));
    }

    /// `PotionItem.getName`: the hotbar's and the tooltip's name of a potion
    /// with an `item_name` is its contents', and an arrow's the same.
    #[test]
    fn a_potion_with_an_item_name_is_named_by_its_contents() {
        let comp = horizon_items::item_data::command_components(
            r#"[item_name="Bottle",potion_contents={potion:"minecraft:long_swiftness"}]"#,
        )
        .unwrap();
        let (damage, extra) = horizon_items::item_data::decode_components(Some(&comp));
        let mut stack = ItemStack {
            item: "minecraft:potion".into(),
            count: 1,
            damage,
            extra,
        };
        let translate = |key: &str| match key {
            "item.minecraft.potion.effect.swiftness" => Some("Potion of Swiftness".to_owned()),
            "item.minecraft.tipped_arrow.effect.swiftness" => Some("Arrow of Swiftness".to_owned()),
            _ => None,
        };
        let name = styled_name(&stack, || name_line(&stack, &translate));
        assert_eq!(name.plain(), "Potion of Swiftness");
        stack.item = "minecraft:tipped_arrow".into();
        assert_eq!(name_line(&stack, &translate), "Arrow of Swiftness");
        // Any other item keeps its item_name.
        stack.item = "minecraft:stick".into();
        assert_eq!(styled_name(&stack, || "fallback".into()).plain(), "Bottle");
    }

    #[test]
    fn name_line_translates_items_of_any_namespace() {
        let translate = |key: &str| match key {
            "item.example.ruby" => Some("Ruby".to_owned()),
            "block.example.ruby_block" => Some("Block of Ruby".to_owned()),
            "item.example.tools.chisel" => Some("Chisel".to_owned()),
            "item.minecraft.diamond" => Some("Diamond".to_owned()),
            _ => None,
        };
        let name = |id: &str| name_line(&ItemStack::new(id, 1), &translate);
        // `Util.makeDescriptionId`: the item's own namespace, `/` as `.`.
        assert_eq!(name("example:ruby"), "Ruby");
        assert_eq!(name("example:ruby_block"), "Block of Ruby");
        assert_eq!(name("example:tools/chisel"), "Chisel");
        assert_eq!(name("minecraft:diamond"), "Diamond");
        // No translation: the path, as the creative names fall back to.
        assert_eq!(name("example:raw_ruby"), "raw ruby");
    }

    #[test]
    fn player_head_names_use_the_profile() {
        let translate = |key: &str| match key {
            "block.minecraft.player_head.named" => Some("%s's Head".to_owned()),
            "block.minecraft.player_head" => Some("Player Head".to_owned()),
            _ => None,
        };
        let mut named = ItemStack::new("minecraft:player_head", 1);
        named
            .data_mut()
            .set_other("minecraft:profile", Some(Tag::String("Notch".into())));
        assert_eq!(name_line(&named, &translate), "Notch's Head");

        let mut compound = ItemStack::new("minecraft:player_head", 1);
        compound.data_mut().set_other(
            "minecraft:profile",
            Some(Tag::compound([("name", Tag::String("Notch".into()))])),
        );
        assert_eq!(name_line(&compound, &translate), "Notch's Head");

        let mut id_only = ItemStack::new("minecraft:player_head", 1);
        id_only.data_mut().set_other(
            "minecraft:profile",
            Some(Tag::compound([("id", Tag::IntArray(vec![1, 2, 3, 4]))])),
        );
        assert_eq!(name_line(&id_only, &translate), "Player Head");

        named.data_mut().custom_name = Some(Tag::String("Trophy".into()));
        assert_eq!(name_line(&named, &translate), "Trophy");
    }

    #[test]
    fn container_lines_list_five_stacks_then_how_many_more() {
        let mut chest = ItemStack::new("minecraft:chest", 1);
        for slot in [9u8, 1, 2, 3, 4, 5, 6] {
            chest
                .data_mut()
                .container
                .push((slot, ItemStack::new("minecraft:stone", u32::from(slot))));
        }
        let lang = |k: &str| {
            Some(
                match k {
                    "item.container.item_count" => "%s x %s",
                    "item.container.more_items" => "and %s more...",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let name = |s: &ItemStack| s.item.trim_start_matches("minecraft:").to_owned();
        assert_eq!(
            container_lines(&chest, &name, &lang),
            [
                "1 x stone",
                "2 x stone",
                "3 x stone",
                "4 x stone",
                "5 x stone",
                "and 2 more..."
            ]
        );
        assert!(container_lines(&ItemStack::new("minecraft:chest", 1), &name, &lang).is_empty());
        // German writes the count after the name: "%2$s× %1$s".
        let german = |k: &str| match k {
            "item.container.item_count" => Some("%2$s× %1$s".to_owned()),
            other => lang(other),
        };
        assert_eq!(container_lines(&chest, &name, &german)[0], "stone× 1");
    }

    #[test]
    fn fireworks_use_loaded_tooltip_translations() {
        let root = std::env::temp_dir().join(format!(
            "horizon-firework-tooltip-language-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let dir = root.join("assets/minecraft/lang");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("en_us.json"),
            serde_json::to_vec(&serde_json::json!({
                "item.minecraft.firework_rocket.flight_duration": "Flight Duration: %s",
                "item.minecraft.firework_rocket.multiple_stars": "%s x %s",
                "item.minecraft.firework_star.shape.small_ball": "Small Ball",
                "item.minecraft.firework_star.red": "Red",
                "item.minecraft.firework_star.custom_color": "Custom",
                "item.minecraft.firework_star.fade_to": "Fade to",
                "item.minecraft.firework_star.trail": "Trail",
                "item.minecraft.firework_star.flicker": "Twinkle",
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let enchantments = crate::enchant::Enchantments::load(&root, &defaults).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let translate = |key: &str| enchantments.translate(key).map(str::to_owned);
        let mut rocket = ItemStack::new("minecraft:firework_rocket", 1);
        assert_eq!(firework_lines(&rocket, &translate), ["Flight Duration: 1"]);
        let explosion = crate::particles::FireworkExplosion {
            colors: vec![0xB3312C],
            fade: vec![0x123456],
            trail: true,
            twinkle: true,
            ..Default::default()
        };
        rocket.data_mut().fireworks = Some((3, vec![explosion.clone(), explosion.clone()]));
        assert_eq!(
            firework_lines(&rocket, &translate),
            [
                "Flight Duration: 3",
                "2 x Small Ball",
                "  Red",
                "  Fade to Custom",
                "  Trail",
                "  Twinkle",
            ]
        );
        let mut star = ItemStack::new("minecraft:firework_star", 1);
        star.data_mut().firework_explosion = Some(explosion);
        assert_eq!(
            firework_lines(&star, &translate),
            ["Small Ball", "Red", "Fade to Custom", "Trail", "Twinkle"]
        );
    }

    #[test]
    fn amounts_use_two_decimals_at_most() {
        assert_eq!(format_amount(8.0), "8");
        assert_eq!(format_amount(3.2000000953674316), "3.2");
        assert_eq!(format_amount(2.8200000524520874), "2.82");
        assert_eq!(format_amount(0.125), "0.12");
        assert_eq!(format_amount(0.375), "0.38");
    }

    #[test]
    fn extreme_amounts_print_as_the_reference_does() {
        let lang = |k: &str| {
            Some(
                match k {
                    "attribute.modifier.plus.0" => "+%s %s",
                    "attribute.modifier.take.0" => "-%s %s",
                    "attribute.modifier.equals.0" => "%s %s",
                    "attribute.modifier.plus.1" => "+%s%% %s",
                    "attribute.modifier.take.1" => "-%s%% %s",
                    "attribute.modifier.equals.1" => "%s%% %s",
                    "attribute.name.armor" => "Armor",
                    "attribute.name.speed" => "Speed",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let line = |attribute, amount, total, display| {
            modifier_line(&lang, attribute, amount, 0, total, display)
        };
        let plain = |amount, total| line("armor", amount, total, Display::Ranged);
        let percent = |amount, total| line("speed", amount, total, Display::Percentage);
        let cases = [
            (f64::INFINITY, false, "+\u{221e} Armor", "+\u{221e}% Speed"),
            (
                f64::NEG_INFINITY,
                false,
                "-\u{221e} Armor",
                "-\u{221e}% Speed",
            ),
            (f64::INFINITY, true, "\u{221e} Armor", "\u{221e}% Speed"),
            (
                f64::NEG_INFINITY,
                true,
                "-\u{221e} Armor",
                "-\u{221e}% Speed",
            ),
            (f64::NAN, false, "-NaN Armor", "-NaN% Speed"),
            (f64::NAN, true, "NaN Armor", "NaN% Speed"),
            (-0.001, true, "-0 Armor", "-0.1% Speed"),
            (-0.00001, true, "-0 Armor", "-0% Speed"),
            (-0.0, true, "-0 Armor", "-0% Speed"),
        ];
        for (amount, total, want_plain, want_percent) in cases {
            assert_eq!(
                plain(amount, total).as_deref(),
                Some(want_plain),
                "{amount} {total}"
            );
            assert_eq!(
                percent(amount, total).as_deref(),
                Some(want_percent),
                "{amount} {total}"
            );
        }
        // A zero amount, either sign, is no line when it is not a total.
        assert_eq!(plain(-0.0, false), None);
    }

    #[test]
    fn equipment_attributes_use_player_totals_and_slot_group_colors() {
        let modifiers = [
            ("attack_damage", "base_attack_damage", 6.0, "mainhand"),
            ("attack_speed", "base_attack_speed", -2.4, "mainhand"),
            ("armor", "armor", 3.0, "head"),
        ]
        .map(|(attribute, id, amount, slot)| {
            horizon_items::item_data::ModifierRow::new(
                &format!("minecraft:{attribute}"),
                &format!("minecraft:{id}"),
                amount,
                "add_value",
                slot,
            )
        });
        let root = std::env::temp_dir().join(format!(
            "horizon-tooltip-language-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let dir = root.join("assets/minecraft/lang");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("en_us.json"),
            serde_json::to_vec(&serde_json::json!({
                "item.modifiers.mainhand": "When in Main Hand:",
                "item.modifiers.head": "When on Head:",
                "attribute.modifier.equals.0": "%s %s",
                "attribute.modifier.plus.0": "+%s %s",
                "attribute.modifier.take.0": "-%s %s",
                "attribute.name.attack_damage": "Attack Damage",
                "attribute.name.attack_speed": "Attack Speed",
                "attribute.name.armor": "Armor",
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let enchantments = crate::enchant::Enchantments::load(&root, &defaults).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let lang = |key: &str| enchantments.translate(key).map(str::to_owned);
        let green = rgba(0, 170, 0, 255);
        let gray = rgba(170, 170, 170, 255);
        let blue = rgba(85, 85, 255, 255);
        let red = rgba(255, 85, 85, 255);
        assert_eq!(
            attribute_lines(&modifiers, Some([1.0, 4.0]), &lang),
            [
                (String::new(), u32::MAX),
                ("When in Main Hand:".into(), gray),
                (" 7 Attack Damage".into(), green),
                (" 1.6 Attack Speed".into(), green),
                (String::new(), u32::MAX),
                ("When on Head:".into(), gray),
                ("+3 Armor".into(), blue),
            ]
        );
        let recipe = attribute_lines(&modifiers, None, &lang);
        assert_eq!(recipe[2], ("+6 Attack Damage".into(), blue));
        assert_eq!(recipe[3], ("-2.4 Attack Speed".into(), red));
        let custom_base = attribute_lines(&modifiers, Some([2.0, 6.0]), &lang);
        assert_eq!(custom_base[2].0, " 8 Attack Damage");
        assert_eq!(custom_base[3].0, " 3.6 Attack Speed");
        assert_eq!(
            modifier_line(&lang, "attack_damage", 0.0, 0, true, Display::Ranged).as_deref(),
            Some("0 Attack Damage")
        );
    }

    /// A declared attribute reads as its display says (`PercentageAttribute`, `BooleanAttribute`),
    /// its base modifier reads as the green total with a player and as a plain line without one,
    /// and F3+H follows a mod attribute's line with its modifier id.
    #[test]
    fn a_declared_attribute_reads_as_its_display_and_base_modifier() {
        use horizon_items::item_data::ModifierRow;
        let rows = [
            ("mymod:swim", "mymod:boots", 0.05),
            ("mymod:glide", "mymod:wings", 1.0),
            ("mymod:mana", "mymod:base_mana", 5.0),
            ("mymod:glide", "mymod:clipped", 0.0),
        ]
        .map(|(attribute, id, amount)| {
            ModifierRow::new(attribute, id, amount, "add_value", "mainhand")
        });
        let lang = |key: &str| {
            Some(
                match key {
                    "item.modifiers.mainhand" => "When in Main Hand:",
                    "attribute.modifier.plus.0" => "+%s %s",
                    "attribute.modifier.plus.1" => "+%s%% %s",
                    "attribute.modifier.equals.0" => "%s %s",
                    "horizon.attribute.modifier.enabled" => "%s: Enabled",
                    "horizon.attribute.modifier.disabled" => "%s: Disabled",
                    "attribute.name.mymod:swim" => "Swim Speed",
                    "attribute.name.mymod:glide" => "Gliding",
                    "attribute.name.mymod:mana" => "Mana",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let style = |attribute: &str| {
            let (display, base_modifier) = match attribute {
                "mymod:swim" => (Display::Percentage, None),
                "mymod:glide" => (Display::Boolean, None),
                "mymod:mana" => (Display::Ranged, Some("mymod:base_mana".to_owned())),
                _ => return None,
            };
            Some(AttributeStyle {
                tooltip: Tooltip {
                    display,
                    base_modifier,
                    ..Tooltip::default()
                },
                player_base: Some(10.0),
            })
        };
        let lines = |player: Option<[f64; 2]>, advanced: bool| -> Vec<(String, u32)> {
            let filter = AttributeFilter::default();
            attribute_lines_filtered(&rows, player, &filter, advanced, &lang, &style)
                .into_iter()
                .skip(2)
                .collect()
        };
        let (blue, green, debug) = (
            rgba(85, 85, 255, 255),
            rgba(0, 170, 0, 255),
            rgba(85, 85, 85, 255),
        );
        assert_eq!(
            lines(Some([1.0, 4.0]), false),
            [
                ("+5% Swim Speed".to_owned(), blue),
                ("Gliding: Enabled".to_owned(), blue),
                (" 15 Mana".to_owned(), green),
            ]
        );
        // Recipe search has no player: the base modifier is a plain line.
        assert_eq!(lines(None, false)[2], ("+5 Mana".to_owned(), blue));
        let advanced = lines(None, true);
        assert_eq!(advanced.len(), 6);
        assert_eq!(advanced[1], ("  mymod:boots".to_owned(), debug));
    }

    /// A Percentage attribute's added value reads through the percent translation, whatever
    /// its word order, and Boolean modifiers use enabled, disabled or invalid instead of numbers.
    #[test]
    fn a_percentage_reads_through_the_percent_translation_and_booleans_name_their_state() {
        let lang = |key: &str| {
            Some(
                match key {
                    "attribute.modifier.plus.1" => "%2$s: +%1$s%%",
                    "attribute.modifier.take.1" => "%2$s: -%1$s%%",
                    "horizon.attribute.modifier.disabled" => "%s: Disabled",
                    "horizon.attribute.modifier.enabled" => "%s: Enabled",
                    "horizon.attribute.modifier.invalid" => "%s: not valid",
                    "attribute.name.mymod:swim" => "Swim Speed",
                    "attribute.name.mymod:glide" => "Gliding",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let line =
            |attribute, amount, display| modifier_line(&lang, attribute, amount, 0, false, display);
        assert_eq!(
            line("mymod:swim", 0.05, Display::Percentage).as_deref(),
            Some("Swim Speed: +5%")
        );
        assert_eq!(
            line("mymod:swim", -0.5, Display::Percentage).as_deref(),
            Some("Swim Speed: -50%")
        );
        assert_eq!(line("mymod:swim", 0.0, Display::Percentage), None);
        for (amount, operation, expected) in [
            (1.0, 0, "Gliding: Enabled"),
            (0.0, 0, "Gliding: not valid"),
            (-1.0, 0, "Gliding: not valid"),
            (-1.0, 2, "Gliding: Disabled"),
            (-1.5, 2, "Gliding: Disabled"),
            (-1.99, 2, "Gliding: Disabled"),
            (0.5, 2, "Gliding: not valid"),
            (0.5, 1, "Gliding: not valid"),
            (f64::NAN, 0, "Gliding: not valid"),
        ] {
            assert_eq!(
                modifier_line(
                    &lang,
                    "mymod:glide",
                    amount,
                    operation,
                    false,
                    Display::Boolean
                )
                .as_deref(),
                Some(expected)
            );
        }
    }

    /// The recipe book's search text reads each attribute as its declared style says.
    #[test]
    fn the_search_text_reads_declared_attribute_styles() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        let row = |attribute: &str, amount: f64| {
            Tag::compound([
                ("type", Tag::String(attribute.into())),
                ("id", Tag::String("mymod:charm".into())),
                ("amount", Tag::Double(amount.to_bits())),
                ("operation", Tag::String("add_value".into())),
                ("slot", Tag::String("mainhand".into())),
            ])
        };
        stack.data_mut().set_other(
            "minecraft:attribute_modifiers",
            Some(Tag::List(vec![
                row("mymod:swim", 0.05),
                row("mymod:glide", 0.0),
            ])),
        );
        let lang = |key: &str| {
            Some(
                match key {
                    "attribute.modifier.plus.0" => "+%s %s",
                    "attribute.modifier.plus.1" => "+%s%% %s",
                    "horizon.attribute.modifier.disabled" => "%s: Disabled",
                    "attribute.name.mymod:swim" => "Swim Speed",
                    "attribute.name.mymod:glide" => "Gliding",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let style = |attribute: &str| {
            let display = match attribute {
                "mymod:swim" => Display::Percentage,
                "mymod:glide" => Display::Boolean,
                _ => return None,
            };
            Some(AttributeStyle {
                tooltip: Tooltip {
                    display,
                    ..Tooltip::default()
                },
                player_base: None,
            })
        };
        let enchantments = Enchantments::default();
        let search = |style: &dyn Fn(&str) -> Option<AttributeStyle>| {
            lines(
                &stack,
                None,
                &NoAttributeListener,
                &enchantments,
                &lang,
                style,
            )
        };
        let styled = search(&style);
        assert!(styled.iter().any(|l| l == "+5% Swim Speed"), "{styled:?}");
        assert!(!styled.iter().any(|l| l.contains("Gliding")), "{styled:?}");
        // Without the catalog both read as plain numbers, as before.
        let plain = search(&|_| None);
        assert!(plain.iter().any(|l| l == "+0.05 Swim Speed"), "{plain:?}");
        assert!(!plain.iter().any(|l| l.contains("Gliding")), "{plain:?}");
    }

    fn sword_rows() -> Vec<horizon_items::item_data::ModifierRow> {
        use horizon_items::item_data::ModifierRow;
        vec![
            ModifierRow::new(
                "minecraft:attack_damage",
                "minecraft:base_attack_damage",
                6.0,
                "add_value",
                "mainhand",
            ),
            ModifierRow::new(
                "minecraft:attack_speed",
                "minecraft:base_attack_speed",
                -2.4,
                "add_value",
                "mainhand",
            ),
            ModifierRow::new("minecraft:armor", "mod:plating", 3.0, "add_value", "head"),
        ]
    }

    #[test]
    fn a_filter_hides_modifiers_by_id_group_or_all_and_drops_empty_headings() {
        let rows = sword_rows();
        let lang = |key: &str| Some(key.to_owned());
        let text = |filter: &AttributeFilter| -> Vec<String> {
            attribute_lines_filtered(&rows, None, filter, false, &lang, &|_| None)
                .into_iter()
                .map(|(line, _)| line)
                .collect()
        };
        let all = text(&AttributeFilter::default());
        assert_eq!(all.len(), 7, "two headings, three modifiers, two blanks");
        assert_eq!(
            all,
            text(&AttributeFilter {
                ids: vec!["mod:nothing".into()],
                ..Default::default()
            }),
            "an id no modifier has hides nothing"
        );
        // A bare name is `minecraft:`'s.
        let no_damage = text(&AttributeFilter {
            ids: vec!["base_attack_damage".into()],
            ..Default::default()
        });
        assert_eq!(no_damage.len(), 6);
        assert!(!no_damage.iter().any(|l| l.contains("attack_damage")));
        // Hiding a group takes its heading with it.
        let no_head = text(&AttributeFilter {
            groups: vec!["head".into()],
            ..Default::default()
        });
        assert!(!no_head.iter().any(|l| l.contains("item.modifiers.head")));
        assert!(
            no_head
                .iter()
                .any(|l| l.contains("item.modifiers.mainhand"))
        );
        assert_eq!(no_head.len(), 4);
        assert!(
            text(&AttributeFilter {
                skip_all: true,
                ..Default::default()
            })
            .is_empty()
        );
    }

    /// Answers the two questions as a mod's two hooks would.
    struct Hide(AttributeFilter, Vec<(&'static str, u32)>);
    impl AttributeTooltipListener for Hide {
        fn skipped(&self, _: &ItemStack, _: &AttributeTooltipContext) -> AttributeFilter {
            // Only a shown display gathers skips; the second case below hides the modifiers.
            assert_ne!(
                self.0,
                AttributeFilter::default(),
                "gather asked with nothing shown"
            );
            self.0.clone()
        }
        fn extra(
            &self,
            _: &ItemStack,
            ctx: &AttributeTooltipContext,
            shown: bool,
        ) -> Vec<(Text, u32)> {
            assert!(ctx.player_base.is_none());
            assert_eq!(shown, self.0 != AttributeFilter::default());
            self.1.iter().map(|&(t, c)| (Text::legacy(t), c)).collect()
        }
    }

    #[test]
    fn the_builder_filters_then_adds_even_when_the_display_hides_the_modifiers() {
        let rows = sword_rows();
        let lang = |key: &str| Some(key.to_owned());
        let stack = ItemStack::new("minecraft:diamond_sword", 1);
        let ctx = AttributeTooltipContext {
            player_base: None,
            advanced: false,
            creative: false,
        };
        let (lines, added) = attribute_tooltip(
            &stack,
            &rows,
            &ctx,
            true,
            &Hide(
                AttributeFilter {
                    groups: vec!["mainhand".into()],
                    ..Default::default()
                },
                vec![("Rune: fire", 7)],
            ),
            &lang,
            &|_| None,
        );
        assert_eq!(lines.len(), 3, "only the head modifier is left");
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].1, 7);
        // The display hides the modifiers: no gather, but the lines are still asked for.
        let (lines, added) = attribute_tooltip(
            &stack,
            &rows,
            &ctx,
            false,
            &Hide(AttributeFilter::default(), vec![("note", u32::MAX)]),
            &lang,
            &|_| None,
        );
        assert!(lines.is_empty());
        assert_eq!(added.len(), 1);
    }

    #[test]
    #[ignore = "daily"]
    fn equipment_attributes_with_prepared_items_and_language() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = crate::items::Items::catalog(&root, &PackOverlay::NONE, &defaults).unwrap();
        let enchantments = crate::enchant::Enchantments::load(&root, &defaults).unwrap();
        let translate = |key: &str| enchantments.translate(key).map(str::to_owned);
        for (item, expected) in [
            (
                "minecraft:diamond_sword",
                [
                    "",
                    "When in Main Hand:",
                    " 7 Attack Damage",
                    " 1.6 Attack Speed",
                ],
            ),
            (
                "minecraft:diamond_helmet",
                ["", "When on Head:", "+3 Armor", "+2 Armor Toughness"],
            ),
        ] {
            let info = items.get(item).unwrap();
            let lines = attribute_lines(&info.modifiers, Some([1.0, 4.0]), &translate);
            assert_eq!(
                lines
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{item}"
            );
        }
    }

    #[test]
    fn modifier_lines_follow_the_sign_and_operation() {
        let lang = |k: &str| {
            Some(
                match k {
                    "attribute.modifier.plus.2" => "+%s%% %s",
                    "attribute.modifier.take.2" => "-%s%% %s",
                    "attribute.name.movement_speed" => "Speed",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert_eq!(
            modifier_line(&lang, "movement_speed", 0.2, 2, false, Display::Ranged).as_deref(),
            Some("+20% Speed")
        );
        assert_eq!(
            modifier_line(&lang, "movement_speed", -0.6, 2, false, Display::Ranged).as_deref(),
            Some("-60% Speed")
        );
        assert_eq!(
            modifier_line(&lang, "movement_speed", 0.0, 2, false, Display::Ranged),
            None
        );
    }

    #[test]
    fn test_russian_wooden_sword_modifiers() {
        let lang = |k: &str| {
            Some(
                match k {
                    "attribute.modifier.equals.0" => "%2$s: %1$s",
                    "attribute.modifier.plus.0" => "%2$s: +%1$s",
                    "attribute.modifier.take.0" => "%2$s: -%1$s",
                    "attribute.modifier.equals.2" => "%2$s: %1$s%%",
                    "attribute.modifier.plus.2" => "%2$s: +%1$s%%",
                    "attribute.modifier.take.2" => "%2$s: -%1$s%%",
                    "attribute.name.attack_damage" => "Урон при атаке",
                    "attribute.name.attack_speed" => "Скорость атаки",
                    "attribute.name.movement_speed" => "Скорость",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert_eq!(
            modifier_line(&lang, "attack_damage", 4.0, 0, true, Display::Ranged).as_deref(),
            Some("Урон при атаке: 4")
        );
        assert_eq!(
            modifier_line(&lang, "attack_speed", 1.6, 0, true, Display::Ranged).as_deref(),
            Some("Скорость атаки: 1.6")
        );
        assert_eq!(
            modifier_line(&lang, "movement_speed", -0.6, 2, false, Display::Ranged).as_deref(),
            Some("Скорость: -60%")
        );
    }
    #[test]
    fn modifier_lines_support_positional_format() {
        let lang = |k: &str| {
            Some(
                match k {
                    "attribute.modifier.equals.0" => "%2$s: %1$s",
                    "attribute.modifier.plus.2" => "+%1$s%% %2$s",
                    "attribute.name.attack_damage" => "Damage",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert_eq!(
            modifier_line(&lang, "attack_damage", 5.0, 0, true, Display::Ranged).as_deref(),
            Some("Damage: 5")
        );
        assert_eq!(
            modifier_line(&lang, "attack_damage", 0.2, 2, false, Display::Ranged).as_deref(),
            Some("+20% Damage")
        );
    }

    #[test]
    fn a_spawn_egg_of_a_monster_says_so_in_peaceful() {
        let lang =
            |k: &str| (k == "item.spawn_egg.peaceful").then(|| "Disabled in Peaceful".to_owned());
        let monsters = |id: &str| !matches!(id, "minecraft:zombie" | "minecraft:creeper");
        let line = |s: &ItemStack, peaceful| {
            spawn_egg_peaceful_line(s, peaceful, &tooltip_display(s), &monsters, &lang)
        };
        let zombie = ItemStack::new("minecraft:zombie_spawn_egg", 1);
        assert_eq!(line(&zombie, true).as_deref(), Some("Disabled in Peaceful"));
        // Not in Peaceful, or an egg of a mob that is allowed there, or no egg.
        assert_eq!(line(&zombie, false), None);
        assert_eq!(
            line(&ItemStack::new("minecraft:pig_spawn_egg", 1), true),
            None
        );
        assert_eq!(line(&ItemStack::new("minecraft:stone", 1), true), None);
        // The entity data's type wins over the egg's own.
        let mut pig = ItemStack::new("minecraft:pig_spawn_egg", 1);
        pig.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([(
                "id",
                Tag::String("minecraft:creeper".into()),
            )])),
        );
        assert_eq!(line(&pig, true).as_deref(), Some("Disabled in Peaceful"));
        // Any item with the component, and an id without its namespace.
        let mut stand = ItemStack::new("minecraft:armor_stand", 1);
        stand.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([("id", Tag::String("zombie".into()))])),
        );
        assert_eq!(line(&stand, true).as_deref(), Some("Disabled in Peaceful"));
    }

    /// An appender that adds its own name, so the order of the lines shows.
    struct Named(&'static str);
    impl Named {
        fn append(&self, _: &ItemStack, lines: &mut Vec<(Text, u32)>) {
            lines.push((Text::literal(self.0), u32::MAX));
        }
    }

    #[test]
    fn empty_component_boundaries_keep_builder_order() {
        let mut marks = DetailMarks::new(true);
        for id in [
            "minecraft:tropical_fish/pattern",
            "minecraft:instrument",
            "minecraft:map_id",
            "minecraft:bees",
            "minecraft:container",
            "minecraft:banner_patterns",
            "minecraft:written_book_content",
            "minecraft:charged_projectiles",
            "minecraft:fireworks",
            "minecraft:potion_contents",
            "minecraft:jukebox_playable",
            "minecraft:trim",
            "minecraft:stored_enchantments",
            "minecraft:enchantments",
            "minecraft:dyed_color",
            "minecraft:lore",
            "minecraft:attribute_modifiers",
            "minecraft:unbreakable",
            "minecraft:ominous_bottle_amplifier",
            "minecraft:suspicious_stew_effects",
            "minecraft:block_state",
            "minecraft:entity_data",
            "minecraft:block_entity_data",
        ] {
            marks.start(id, 1);
        }
        marks.end(1);
        let before = TooltipPlacement::Before("minecraft:bees".into());
        let after = TooltipPlacement::After("minecraft:enchantments".into());
        let starts = marks.starts.as_deref().unwrap();
        assert!(anchor(&TooltipPlacement::BeforeAll, starts, 1) < anchor(&before, starts, 1));
        assert!(anchor(&before, starts, 1) < anchor(&after, starts, 1));
        assert!(anchor(&after, starts, 1) < anchor(&TooltipPlacement::AfterAll, starts, 1));
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack
            .data_mut()
            .set_other("test:component", Some(Tag::Int(1)));
        let names = [
            Named("after"),
            Named("before 1"),
            Named("first"),
            Named("before 2"),
            Named("last"),
        ];
        let placements = [
            &after,
            &before,
            &TooltipPlacement::BeforeAll,
            &before,
            &TooltipPlacement::AfterAll,
        ];
        let mut lines = vec![(Text::literal("name"), u32::MAX)];
        add_appender_lines(
            &mut lines,
            &marks,
            &stack,
            None,
            Named::append,
            placements
                .into_iter()
                .zip(&names)
                .map(|(placement, appender)| {
                    ("test:component", placement, true, appender as &Named)
                }),
        );
        assert_eq!(
            lines
                .iter()
                .map(|(text, _)| text.plain())
                .collect::<Vec<_>>(),
            ["name", "first", "before 1", "before 2", "after", "last"]
        );
    }

    /// Two mods' appenders, before and after the details: a component's lines go where its
    /// placement puts them, and only for a stack that carries the component, whose tooltip
    /// shows it, and whose mod is enabled.
    #[test]
    fn appended_lines_go_where_their_placement_puts_them() {
        let mut lines = vec![
            (Text::literal("lore"), u32::MAX),
            (Text::literal("enchantments"), u32::MAX),
        ];
        let marks = DetailMarks {
            starts: Some(vec![("minecraft:lore", 0), ("minecraft:enchantments", 1)]),
            finish: 2,
        };
        let mut stack = ItemStack::new("minecraft:stick", 1);
        for id in [
            "mymod:charge",
            "mymod:glow",
            "mymod:hidden",
            "mymod:tail",
            "mymod:pre",
            "mymod:off",
        ] {
            stack.data_mut().set_other(id, Some(Tag::Int(1)));
        }
        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("mymod:hidden".into())]),
            )])),
        );
        let (after_lore, before_all, after_all, before_enchantments) = (
            TooltipPlacement::After("minecraft:lore".into()),
            TooltipPlacement::BeforeAll,
            TooltipPlacement::AfterAll,
            TooltipPlacement::Before("minecraft:enchantments".into()),
        );
        let (charge, glow, hidden, tail, pre, off, absent) = (
            Named("mod A: after lore"),
            Named("mod B: before all"),
            Named("mod A: hidden"),
            Named("mod A: after all"),
            Named("mod A: before enchantments"),
            Named("mod B: disabled"),
            Named("mod B: not carried"),
        );
        let registered = [
            ("mymod:charge", &after_lore, true, &charge as &Named),
            ("mymod:glow", &before_all, true, &glow as &Named),
            ("mymod:hidden", &after_all, true, &hidden as &Named),
            ("other:absent", &after_all, true, &absent as &Named),
            ("mymod:off", &after_all, false, &off as &Named),
            ("mymod:pre", &before_enchantments, true, &pre as &Named),
            ("mymod:tail", &after_all, true, &tail as &Named),
        ];
        add_appender_lines(
            &mut lines,
            &marks,
            &stack,
            None,
            Named::append,
            registered.into_iter(),
        );
        let texts: Vec<String> = lines.iter().map(|(text, _)| text.plain()).collect();
        assert_eq!(
            texts,
            [
                "mod B: before all",
                "lore",
                "mod A: after lore",
                "mod A: before enchantments",
                "enchantments",
                "mod A: after all",
            ]
        );
    }

    /// A component that only the live data sets (`map_post_processing`, which the saved form
    /// leaves out) still reaches its appender.
    #[test]
    fn an_appender_sees_a_transient_component() {
        let mut map = ItemStack::new("minecraft:filled_map", 1);
        map.data_mut().map_post_processing = Some(1);
        let mut lines = Vec::new();
        let scale = Named("scale");
        let after_all = TooltipPlacement::AfterAll;
        add_appender_lines(
            &mut lines,
            &DetailMarks::new(true),
            &map,
            None,
            Named::append,
            [(
                "minecraft:map_post_processing",
                &after_all,
                true,
                &scale as &Named,
            )]
            .into_iter(),
        );
        let texts: Vec<String> = lines.iter().map(|(text, _)| text.plain()).collect();
        assert_eq!(texts, ["scale"]);
    }
}

#[cfg(test)]
mod stack_modifier_tests {
    use horizon_client::items::ItemStack;

    #[test]
    fn tooltip_attributes_come_from_the_stack() {
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        let attack = horizon_core::nbt::Tag::compound([
            (
                "type",
                horizon_core::nbt::Tag::String("minecraft:attack_damage".into()),
            ),
            (
                "id",
                horizon_core::nbt::Tag::String("minecraft:test".into()),
            ),
            ("amount", horizon_core::nbt::Tag::Double(5.0f64.to_bits())),
            (
                "operation",
                horizon_core::nbt::Tag::String("add_value".into()),
            ),
            ("slot", horizon_core::nbt::Tag::String("mainhand".into())),
        ]);
        sword.data_mut().set_other(
            "minecraft:attribute_modifiers",
            Some(horizon_core::nbt::Tag::List(vec![attack])),
        );
        let modifiers = sword.default_attribute_modifiers_or(&[]);
        assert_eq!(modifiers.len(), 1);
        assert_eq!(modifiers[0].attribute, "minecraft:attack_damage");
        assert_eq!(modifiers[0].amount, 5.0);
        sword.data_mut().set_other(
            "!minecraft:attribute_modifiers",
            Some(horizon_core::nbt::Tag::compound(Vec::<(
                &str,
                horizon_core::nbt::Tag,
            )>::new())),
        );
        assert!(sword.default_attribute_modifiers_or(&[]).is_empty());
    }
}
