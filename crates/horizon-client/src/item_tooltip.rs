//! The lines of an item's tooltip as `ItemStack.getTooltipLines` writes them
//! without a player and without advanced tooltips, for stacks as recipes
//! produce them. The recipe book's search tree matches them, so recipes of
//! tools and armour answer to "attack speed" or "when on head".
use crate::items::{ItemInfo, ItemStack};

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
) -> Option<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let scale = match operation {
        0 if attribute == "knockback_resistance" => 10.0,
        0 => 1.0,
        _ => 100.0,
    };
    let (kind, shown) = if amount > 0.0 {
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

/// `ItemStack.addAttributeTooltips` for the item's own modifiers: per slot
/// group an empty line, the group's heading and a line per modifier.
fn attribute_lines(info: &ItemInfo, translate: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut lines = Vec::new();
    for group in SLOT_GROUPS {
        let mut first = true;
        for (attribute, _, amount, operation, slot) in &info.modifiers {
            if slot != group {
                continue;
            }
            if std::mem::take(&mut first) {
                lines.push(String::new());
                lines.push(
                    translate(&format!("item.modifiers.{group}"))
                        .unwrap_or_else(|| format!("item.modifiers.{group}")),
                );
            }
            let id = match operation.as_str() {
                "add_multiplied_base" => 1,
                "add_multiplied_total" => 2,
                _ => 0,
            };
            let path = attribute.strip_prefix("minecraft:").unwrap_or(attribute);
            lines.extend(modifier_line(translate, path, *amount, id));
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
    // FireworkRocketItem: the flight duration (an unset component is 1).
    if path == "firework_rocket" {
        let flight = data.and_then(|d| d.fireworks.as_ref()).map_or(1, |f| f.0);
        lines.push(format!(
            "{} {flight}",
            text("item.minecraft.firework_rocket.flight")
        ));
    }
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
        lines.extend(attribute_lines(info, translate));
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
    lines.extend(detail_lines(stack, info, translate));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_use_two_decimals_at_most() {
        assert_eq!(format_amount(8.0), "8");
        assert_eq!(format_amount(3.2000000953674316), "3.2");
        assert_eq!(format_amount(2.8200000524520874), "2.82");
        assert_eq!(format_amount(0.125), "0.12");
        assert_eq!(format_amount(0.375), "0.38");
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
            modifier_line(&lang, "movement_speed", 0.2, 2).as_deref(),
            Some("+20% Speed")
        );
        assert_eq!(
            modifier_line(&lang, "movement_speed", -0.6, 2).as_deref(),
            Some("-60% Speed")
        );
        assert_eq!(modifier_line(&lang, "movement_speed", 0.0, 2), None);
    }
}
