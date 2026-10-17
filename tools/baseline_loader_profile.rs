//! Historical baseline comparator for PR #607; one normal independent loading sequence.
use horizon_client::{assets::Assets, blocks::BlockFacts};
use horizon_world::level::Behaviours;
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("expected prepared resource directory")?;
    if args.next().as_deref() != Some("baseline") {
        return Err("expected baseline".into());
    }
    let root = Path::new(&root);
    let total = Instant::now();
    let assets = Assets::load(root)?;
    let started = Instant::now();
    let facts = BlockFacts::load(root, assets.registry)?;
    let behaviours = Behaviours::load(assets.registry, root)?;
    println!("phase_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
    println!("total_ms={:.3}", total.elapsed().as_secs_f64() * 1000.0);
    println!("states={}", facts.states.len());
    std::hint::black_box((facts, behaviours));
    Ok(())
}
