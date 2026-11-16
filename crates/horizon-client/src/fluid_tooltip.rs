//! Fluid tooltip lines: what a fluid's tooltip shows before `fluid_tooltip` listeners edit it,
//! for every place the game draws one. Names come from the lang (`fluid_type.<mod>.<name>`,
//! vanilla's by its block), so the lines stay translation-ready.
use crate::hud::rgba;
use crate::text::Text;
use mod_api::FluidStack;

/// `formic_acid` as "Formic Acid", for a fluid its lang doesn't name.
fn prettified(path: &str) -> String {
    path.split('_')
        .map(|w| {
            w.chars().next().map_or(String::new(), |c| {
                c.to_uppercase().chain(w.chars().skip(1)).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A fluid's name key and the words it falls back to: NeoForge's `FluidType` description id is
/// `fluid_type.<namespace>.<path>`; vanilla's own fluids are named by their block,
/// `block.<namespace>.<path>`, else the path in words.
fn name_parts(fluid: &str) -> (String, String) {
    let (ns, path) = fluid.split_once(':').unwrap_or(("minecraft", fluid));
    let fallback = crate::lang::get_or(&format!("block.{ns}.{path}"), &prettified(path));
    (format!("fluid_type.{ns}.{path}"), fallback)
}

/// Fluid `fluid`'s name as the lang names it, else its path in words.
pub fn fluid_name(fluid: &str) -> String {
    let (key, fallback) = name_parts(fluid);
    crate::lang::get_or(&key, &fallback)
}

/// The lines of `stack`'s tooltip: its name, its amount in millibuckets and, when `advanced`
/// (F3+H), its id in dark grey. A fluid no mod provides is named "Unknown fluid".
pub fn lines(stack: &FluidStack, advanced: bool) -> Vec<(Text, u32)> {
    let id = mod_api::__private::fluid_identity_name(stack.fluid);
    let name = if stack.key().is_some() {
        let (key, fallback) = name_parts(&id);
        Text::translate_with_fallback(&key, &fallback, &[])
    } else {
        Text::translate("horizon.fluid.unknown", &[])
    };
    let amount = Text::translate_with_fallback(
        "horizon.fluid.amount",
        "%s mB",
        &[Text::literal(stack.amount.to_string())],
    );
    let mut lines = vec![(name, u32::MAX), (amount, rgba(170, 170, 170, 255))];
    if advanced {
        lines.push((Text::literal(id), rgba(85, 85, 85, 255)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(id: &str, amount: u32) -> FluidStack {
        mod_platform::__private::part_tests::game_thread();
        FluidStack::new(mod_api::__private::fluid_key(id).unwrap(), amount)
    }
    fn plain(lines: &[(Text, u32)]) -> Vec<String> {
        lines.iter().map(|(text, _)| text.plain()).collect()
    }

    #[test]
    fn lines_name_the_fluid_then_its_amount() {
        let lines = lines(&stack("minecraft:water", 1250), false);
        assert_eq!(plain(&lines), ["Water", "1250 mB"]);
        assert_eq!(lines[0].1, u32::MAX);
        assert_eq!(lines[1].1, rgba(170, 170, 170, 255));
    }

    #[test]
    fn an_unnamed_mod_fluid_falls_back_to_its_path_in_words() {
        let lines = lines(&stack("minecraft:lava", 1), false);
        assert_eq!(plain(&lines), ["Lava", "1 mB"]);
        assert_eq!(fluid_name("mymod:formic_acid"), "Formic Acid");
    }

    #[test]
    fn advanced_adds_the_id() {
        let lines = lines(&stack("minecraft:water", 1000), true);
        assert_eq!(plain(&lines), ["Water", "1000 mB", "minecraft:water"]);
        assert_eq!(lines[2].1, rgba(85, 85, 85, 255));
    }

    #[test]
    fn lines_stay_translatable() {
        let lines = lines(&stack("minecraft:water", 5), false);
        assert_eq!(
            lines[0].0.translation_key(),
            Some("fluid_type.minecraft.water")
        );
        assert_eq!(lines[1].0.translation_key(), Some("horizon.fluid.amount"));
    }
}
