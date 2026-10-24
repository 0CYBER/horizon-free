//! Tooltip display visibility oracle companion for tools/test_tooltip_display_parity.py.
use horizon_client::{item_tooltip::TooltipDisplay, items::ItemStack};
use horizon_world::nbt::Tag;
use std::{error::Error, fmt::Write};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let mut output = String::new();
    for line in std::fs::read_to_string(&args[1])?.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let hide_tooltip: bool = fields[0].parse()?;
        let creative: bool = fields[1].parse()?;
        let hide_attributes: bool = fields[2].parse()?;
        let mut component = Vec::new();
        if hide_tooltip {
            component.push(("hide_tooltip", Tag::Byte(1)));
        }
        if hide_attributes {
            component.push((
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
            ));
        }
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        if !component.is_empty() {
            stack
                .data_mut()
                .set_other("minecraft:tooltip_display", Some(Tag::compound(component)));
        }
        let display = TooltipDisplay::for_stack(&stack);
        writeln!(
            output,
            "{}\t{}",
            display.name_visible(creative),
            display.shows("minecraft:attribute_modifiers")
        )?;
    }
    std::fs::write(&args[2], output)?;
    Ok(())
}
