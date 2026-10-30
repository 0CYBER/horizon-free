//! Lore tooltip companion for tools/test_lore_tooltip_parity.py: the spans
//! `item_tooltip::lore_texts` gives each stack (SNBT, one per line).
use horizon_client::{
    item_data::parse_stack,
    item_tooltip::{TooltipDisplay, lore_texts},
};
use std::{error::Error, fmt::Write};

pub fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = crate::args().collect();
    let input = std::fs::read_to_string(&args[1])?;
    let mut output = String::new();
    for line in input
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let stack = parse_stack(line)?.ok_or("empty stack")?;
        let display = TooltipDisplay::for_stack(&stack);
        if !display.shows("minecraft:lore") {
            writeln!(output, "-")?;
            continue;
        }
        let lines = lore_texts(&stack);
        if lines.is_empty() {
            writeln!(output, "-")?;
            continue;
        }
        let mut rendered = Vec::new();
        for text in lines {
            let mut spans = Vec::new();
            for span in &text.spans {
                if span.text.is_empty() {
                    continue;
                }
                let style = &span.style;
                spans.push(format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    span.text,
                    style.color.map(|c| c as i32).unwrap_or(-1),
                    style.bold == Some(true),
                    style.italic == Some(true),
                    style.underlined == Some(true),
                    style.strikethrough == Some(true),
                    style.obfuscated == Some(true),
                ));
            }
            rendered.push(spans.join(" | "));
        }
        writeln!(output, "{}", rendered.join(" || "))?;
    }
    std::fs::write(&args[2], output)?;
    Ok(())
}
