//! Held-live entity attribute allocation workload for external RSS measurement.
use horizon_world::attributes::entity::Catalog;
use std::{
    hint::black_box,
    io::{self, Write},
    sync::Arc,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &std::fs::read_to_string(&args[1])?,
    )?)?);
    let count: usize = args[2].parse()?;
    let active = args[3] == "active";
    if count > 100_000 {
        return Err("allocation workload exceeds limit".into());
    }
    let keys = [
        "minecraft:max_health",
        "minecraft:max_absorption",
        "minecraft:movement_speed",
        "minecraft:armor",
    ]
    .map(|s| catalog.id(s).unwrap());
    let mut entities = Vec::with_capacity(count);
    let mut sum = 0.0;
    for _ in 0..count {
        let mut a = catalog.create("minecraft:zombie").unwrap();
        if active {
            for id in keys {
                assert!(a.ensure(id));
            }
        }
        sum += a.value(keys[0]).unwrap();
        entities.push(a);
    }
    black_box(&entities);
    println!(
        "{}",
        serde_json::json!({"entities":entities.len(),"active_per_entity":if active{4}else{0},"health_sum":sum})
    );
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    black_box(&entities);
    Ok(())
}
