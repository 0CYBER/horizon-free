//! Persistent enchantment components and their displayed names.
use horizon_core::nbt::parse_snbt;
use horizon_items::{enchant::Enchantments, item_nbt::item_from_nbt};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Arc,
};

fn describe(enchantments: &Enchantments, levels: &[(Arc<str>, u32)]) -> String {
    let mut rows: Vec<_> = levels
        .iter()
        .map(|(id, level)| {
            format!(
                "{id}={level}={}",
                enchantments.full_name_in(None, id, *level).0
            )
        })
        .collect();
    rows.sort();
    rows.join("|")
}

fn main() -> horizon_items::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let defaults = horizon_world::item_defaults::read(root)?;
    let enchantments = Enchantments::load(root, &defaults)?;
    let mut out = BufWriter::new(File::create(&args[3])?);
    for line in BufReader::new(File::open(&args[2])?).lines() {
        let line = line?;
        let (label, snbt) = line.split_once('\t').ok_or("fixture separator")?;
        let (tag, _) = parse_snbt(&format!(
            "{{id:'minecraft:diamond_sword',count:1,components:{snbt}}}"
        ))?;
        let stack = item_from_nbt(&tag).ok_or("decoded item")?;
        writeln!(
            out,
            "{label}\t{}\t{}\t{}",
            stack.damage,
            describe(&enchantments, stack.enchantments()),
            describe(&enchantments, stack.stored_enchantments()),
        )?;
    }
    Ok(())
}
