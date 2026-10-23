//! Firework tooltip companion for tools/test_firework_tooltip_parity.py: the
//! lines `item_tooltip::firework_lines` gives each stack (SNBT, one per line;
//! the `#resources` line names the prepared resources for en_us).
use horizon_client::{enchant::Enchantments, item_data::parse_stack, item_tooltip::firework_lines};
use std::{error::Error, fmt::Write, path::PathBuf};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let input = std::fs::read_to_string(&args[1])?;
    let root = input
        .lines()
        .find_map(|l| l.strip_prefix("#resources "))
        .map(PathBuf::from)
        .ok_or("no #resources line")?;
    // Use the inventory's filtered translations so missing tooltip keys are caught.
    let enchantments = Enchantments::load(&root);
    let translate = |key: &str| enchantments.translate(key).map(str::to_owned);
    let mut output = String::new();
    for line in input.lines().filter(|l| !l.starts_with('#')) {
        let stack = parse_stack(line)?.ok_or("empty stack")?;
        let lines = firework_lines(&stack, &translate);
        if lines.is_empty() {
            writeln!(output, "-")?;
        } else {
            writeln!(output, "{}", lines.join(" | "))?;
        }
    }
    std::fs::write(&args[2], output)?;
    Ok(())
}
