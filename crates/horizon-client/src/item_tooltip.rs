//! Item tooltip formatting: equipment modifiers with a player's base values,
//! and lines without player context for the recipe book's search tree.
use crate::{
    hud::rgba,
    items::{ItemInfo, ItemStack},
};
use horizon_world::nbt::Tag;

/// The `minecraft:tooltip_display` component applied by `ItemStack` while it
/// builds a hover tooltip.
pub struct TooltipDisplay<'a> {
    component: Option<&'a Tag>,
    ominous_banner: bool,
}

impl<'a> TooltipDisplay<'a> {
    pub fn for_stack(stack: &'a ItemStack) -> Self {
        Self {
            component: stack
                .data()
                .and_then(|data| data.other("minecraft:tooltip_display")),
            ominous_banner: stack.data().is_some_and(|data| data.ominous_banner),
        }
    }

    fn hides_tooltip(&self) -> bool {
        matches!(
            self.component.and_then(|display| display.get("hide_tooltip")),
            Some(Tag::Byte(value)) if *value != 0
        )
    }

    /// `ItemStack.getTooltipLines`: `hide_tooltip` suppresses the name unless
    /// the tooltip flag is creative.
    pub fn name_visible(&self, creative: bool) -> bool {
        creative || !self.hides_tooltip()
    }

    /// `TooltipDisplay.shows`: whether details for a data component are shown.
    pub fn shows(&self, component: &str) -> bool {
        if self.hides_tooltip() {
            return false;
        }
        // `item_data::decode_components` models the raid captain banner's
        // default hidden pattern tooltip as `ominous_banner` instead of
        // retaining its redundant `tooltip_display` component.
        if self.ominous_banner
            && self.component.is_none()
            && component == "minecraft:banner_patterns"
        {
            return false;
        }
        let Some(Tag::List(hidden)) = self
            .component
            .and_then(|display| display.get("hidden_components"))
        else {
            return true;
        };
        let component = horizon_world::nbt::JavaString::from(component);
        !hidden
            .iter()
            .any(|tag| matches!(tag, Tag::String(id) if id == &component))
    }
}

/// `EquipmentSlotGroup` in declaration order, the order tooltips list them in.
const SLOT_GROUPS: [&str; 11] = [
    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
    "saddle",
];

/// `ItemAttributeModifiers.ATTRIBUTE_MODIFIER_FORMAT`, `DecimalFormat("#.##")`
/// of a positive amount (half-even on the exact value).
fn format_amount(value: f64) -> String {
    let text = format!("{:.2}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `ItemAttributeModifiers.Display.Default.apply`: a modifier's line, none
/// for a zero amount. `percent` is the modifier's operation id (0: added
/// value, 1 and 2: multiplied).
fn modifier_line(
    translate: &dyn Fn(&str) -> Option<String>,
    attribute: &str,
    amount: f64,
    operation: usize,
    total: bool,
) -> Option<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let scale = match operation {
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
    } else {
        return None;
    };
    Some(
        text(&format!("attribute.modifier.{kind}.{operation}"))
            .replacen("%s", &format_amount(shown), 1)
            .replacen("%s", &text(&format!("attribute.name.{attribute}")), 1)
            .replace("%%", "%"),
    )
}

/// `ItemStack.addAttributeTooltips`: slot groups, colors and optional player
/// attack damage/speed bases. Recipe search has no player context.
pub fn attribute_lines(
    modifiers: &[(String, String, f64, String, String)],
    player_base: Option<[f64; 2]>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let mut lines = Vec::new();
    for group in SLOT_GROUPS {
        let mut first = true;
        for (attribute, modifier, amount, operation, slot) in modifiers {
            if slot != group {
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
            let base = player_base.and_then(|[damage, speed]| match modifier.as_str() {
                "minecraft:base_attack_damage" => Some(damage),
                "minecraft:base_attack_speed" => Some(speed),
                _ => None,
            });
            if let Some(line) = modifier_line(
                translate,
                path,
                amount + base.unwrap_or(0.0),
                id,
                base.is_some(),
            ) {
                let (line, color) = if base.is_some() {
                    (format!(" {line}"), rgba(0, 170, 0, 255))
                } else if *amount > 0.0 {
                    (line, rgba(85, 85, 255, 255))
                } else {
                    (line, rgba(255, 85, 85, 255))
                };
                lines.push((line, color));
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

/// The name line of `stack`, as the item's translation or, for potions and
/// smithing templates, its name component gives it.
pub fn name_line(stack: &ItemStack, translate: &dyn Fn(&str) -> Option<String>) -> String {
    let path = stack.item.trim_start_matches("minecraft:");
    if let Some(name) = stack.custom_name() {
        return name.to_owned();
    }
    if stack.data().is_some_and(|d| d.ominous_banner) {
        return translate("block.minecraft.ominous_banner")
            .unwrap_or_else(|| "Ominous Banner".to_owned());
    }
    let key = if is_potion_item(&stack.item) {
        Some(crate::brewing::potion_name_key(&stack.item, stack.potion()))
    } else if let Some(pattern) = path.strip_suffix("_armor_trim_smithing_template") {
        Some(format!("trim_pattern.minecraft.{pattern}"))
    } else if path == "netherite_upgrade_smithing_template" {
        Some("upgrade.minecraft.netherite_upgrade".to_owned())
    } else if path.ends_with("_banner_pattern") {
        // The item name component of the pattern items that have one.
        Some(format!("item.minecraft.{path}.new"))
    } else {
        None
    };
    key.and_then(|k| translate(&k))
        .or_else(|| translate(&format!("item.minecraft.{path}")))
        .or_else(|| translate(&format!("block.minecraft.{path}")))
        .unwrap_or_else(|| path.replace('_', " "))
}

/// ItemStack.getRarity: the ominous banner override, then the enchantment upgrade.
pub fn rarity<'a>(stack: &ItemStack, default: &'a str) -> &'a str {
    let rarity = if stack.data().is_some_and(|d| d.ominous_banner) {
        "uncommon"
    } else {
        default
    };
    if stack.enchantments().is_empty() {
        rarity
    } else {
        match rarity {
            "common" | "uncommon" => "rare",
            "rare" => "epic",
            r => r,
        }
    }
}

/// The lines of `stack` below its name that its components give, in
/// `ItemStack.addDetailsToTooltip` order.
pub fn detail_lines(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let path = stack.item.trim_start_matches("minecraft:");
    let mut lines = Vec::new();
    // SmithingTemplateItem.appendHoverText.
    if path.ends_with("_smithing_template") {
        let kind = if path == "netherite_upgrade_smithing_template" {
            "netherite_upgrade"
        } else {
            "armor_trim"
        };
        lines.push(text("item.minecraft.smithing_template"));
        lines.push(String::new());
        lines.push(text("item.minecraft.smithing_template.applies_to"));
        lines.push(format!(
            " {}",
            text(&format!(
                "item.minecraft.smithing_template.{kind}.applies_to"
            ))
        ));
        lines.push(text("item.minecraft.smithing_template.ingredients"));
        lines.push(format!(
            " {}",
            text(&format!(
                "item.minecraft.smithing_template.{kind}.ingredients"
            ))
        ));
    }
    // BeehiveBlock: the block state components of the item.
    if path == "beehive" {
        lines.push(
            text("container.beehive.bees")
                .replacen("%s", "0", 1)
                .replacen("%s", "3", 1),
        );
        lines.push(
            text("container.beehive.honey")
                .replacen("%s", "0", 1)
                .replacen("%s", "5", 1),
        );
    }
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
            crate::brewing::tooltip(stack.potion(), scale, translate)
                .into_iter()
                .map(|l| l.0),
        );
        let applied: Vec<String> = stack
            .potion()
            .and_then(crate::brewing::potion)
            .into_iter()
            .flat_map(|p| p.effects.iter())
            .filter_map(|(effect, _, amplifier)| {
                let (attribute, amount, operation) = effect_modifier(effect)?;
                modifier_line(
                    translate,
                    attribute,
                    amount * (*amplifier + 1) as f64,
                    operation,
                    false,
                )
            })
            .collect();
        if !applied.is_empty() {
            lines.push(String::new());
            lines.push(text("potion.whenDrank"));
            lines.extend(applied);
        }
    }
    // JukeboxPlayable.addToTooltip: the song's description.
    if path.starts_with("music_disc_") {
        lines.push(text(&format!("item.minecraft.{path}.desc")));
    }
    // Trim.addToTooltip.
    if let Some((material, pattern)) = data.and_then(|d| d.trim.as_ref()) {
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
    // DyedItemColor.addToTooltip (without advanced tooltips).
    if data.is_some_and(|d| d.dyed_color.is_some()) {
        lines.push(text("item.dyed"));
    }
    if let Some(info) = info {
        lines.extend(
            attribute_lines(&info.modifiers, None, translate)
                .into_iter()
                .map(|(line, _)| line),
        );
    }
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
    let display = TooltipDisplay::for_stack(stack);
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
        .map(|(_, s)| {
            count
                .replacen("%s", &s.count.to_string(), 1)
                .replacen("%s", &name(s), 1)
        })
        .collect();
    if slots.len() > 5 {
        let more =
            translate("item.container.more_items").unwrap_or_else(|| "and %s more...".into());
        lines.push(more.replacen("%s", &(slots.len() - 5).to_string(), 1));
    }
    lines
}

/// Every line of `stack`'s tooltip, name first.
pub fn lines(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut lines = vec![name_line(stack, translate)];
    if TooltipDisplay::for_stack(stack).shows("minecraft:lore") {
        lines.extend(lore_texts(stack).into_iter().map(|t| t.plain()));
    }
    lines.extend(detail_lines(stack, info, translate));
    lines
}

/// `ItemLore.addToTooltip`: each lore component under dark purple italic, with
/// the component's own styles (including explicit `false`) winning on children.
pub fn lore_texts(stack: &ItemStack) -> Vec<crate::text::Text> {
    let Some(Tag::List(entries)) = stack.data().and_then(|d| d.other("minecraft:lore")) else {
        return Vec::new();
    };
    let base = crate::text::Style {
        color: Some(crate::text::DARK_PURPLE),
        italic: Some(true),
        ..Default::default()
    };
    entries
        .iter()
        .map(|entry| {
            let mut text = crate::entity_nbt::name_text(entry).resolve_legacy();
            for span in &mut text.spans {
                span.style.inherit(&base);
            }
            text
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(TooltipDisplay::for_stack(&stack).shows("minecraft:lore"));
        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:lore".into())]),
            )])),
        );
        assert!(!TooltipDisplay::for_stack(&stack).shows("minecraft:lore"));
    }

    #[test]
    fn tooltip_display_controls_name_and_component_visibility() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        let display = TooltipDisplay::for_stack(&stack);
        assert!(display.name_visible(false));
        assert!(display.shows("minecraft:attribute_modifiers"));

        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
            )])),
        );
        let display = TooltipDisplay::for_stack(&stack);
        assert!(display.name_visible(false));
        assert!(!display.shows("minecraft:attribute_modifiers"));
        assert!(display.shows("minecraft:enchantments"));

        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([("hide_tooltip", Tag::Byte(1))])),
        );
        let display = TooltipDisplay::for_stack(&stack);
        assert!(!display.name_visible(false));
        assert!(display.name_visible(true));
        assert!(!display.shows("minecraft:attribute_modifiers"));
    }

    #[test]
    fn ominous_banner_keeps_its_default_hidden_pattern_tooltip() {
        let mut banner = crate::mobs::raider::ominous_banner();
        let display = TooltipDisplay::for_stack(&banner);
        assert!(!display.shows("minecraft:banner_patterns"));

        banner.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::Compound(Default::default())),
        );
        let display = TooltipDisplay::for_stack(&banner);
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
        banner.data_mut().custom_name = Some("Captain's Standard".into());
        assert_eq!(name_line(&banner, &translate), "Captain's Standard");
        banner.enchant("minecraft:unbreaking", 1);
        assert_eq!(rarity(&banner, "common"), "rare");
        let plain = ItemStack::new("minecraft:white_banner", 1);
        assert_eq!(name_line(&plain, &translate), "White Banner");
        assert_eq!(rarity(&plain, "common"), "common");
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
        let enchantments = crate::enchant::Enchantments::load(&root);
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
    fn equipment_attributes_use_player_totals_and_slot_group_colors() {
        let modifiers = [
            ("attack_damage", "base_attack_damage", 6.0, "mainhand"),
            ("attack_speed", "base_attack_speed", -2.4, "mainhand"),
            ("armor", "armor", 3.0, "head"),
        ]
        .map(|(attribute, id, amount, slot)| {
            (
                format!("minecraft:{attribute}"),
                format!("minecraft:{id}"),
                amount,
                "add_value".to_owned(),
                slot.to_owned(),
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
        let enchantments = crate::enchant::Enchantments::load(&root);
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
            modifier_line(&lang, "attack_damage", 0.0, 0, true).as_deref(),
            Some("0 Attack Damage")
        );
    }

    #[test]
    #[ignore = "requires prepared items/language in HORIZON_TEST_RESOURCES"]
    fn equipment_attributes_with_prepared_items_and_language() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let items = crate::items::Items::catalog(&root).unwrap();
        let enchantments = crate::enchant::Enchantments::load(&root);
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
            modifier_line(&lang, "movement_speed", 0.2, 2, false).as_deref(),
            Some("+20% Speed")
        );
        assert_eq!(
            modifier_line(&lang, "movement_speed", -0.6, 2, false).as_deref(),
            Some("-60% Speed")
        );
        assert_eq!(modifier_line(&lang, "movement_speed", 0.0, 2, false), None);
    }
}
