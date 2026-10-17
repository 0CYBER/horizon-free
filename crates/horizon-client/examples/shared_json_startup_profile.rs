//! Profile the fact and behaviour table phase of client startup.
//! Run each mode in a separate process to capture CPU usage and peak RSS.
//! The deferred mode evaluates both phases contiguously for fair loader comparison;
//! actual client startup performs other initialization between these phases.
use horizon_client::{assets::Assets, blocks::BlockFacts};
use horizon_world::level::Behaviours;
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("expected resource directory")?;
    let mode = args.next().ok_or("expected baseline, pr607 or shared")?;
    if !["baseline", "pr607", "shared", "deferred"].contains(&mode.as_str()) {
        return Err("expected baseline, pr607 or shared".into());
    }
    let root = Path::new(&root);
    let total = Instant::now();
    let assets = Assets::load(root)?;
    let started = Instant::now();
    let (facts, behaviours) = if mode == "baseline" {
        (
            BlockFacts::load(root, assets.registry)?,
            Behaviours::load(assets.registry, root)?,
        )
    } else if mode == "pr607" {
        // Original PR #607 sequence at a01d47f2, retained as a same-build comparator.
        let json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(root.join("block-behaviours.json"))?,
        ))?;
        BlockFacts::load_with_behaviours(root, assets.registry, &json)?
    } else if mode == "shared" {
        BlockFacts::load_pair(root, assets.registry)?
    } else {
        let json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(root.join("block-behaviours.json"))?,
        ))?;
        let (facts, gameplay) = BlockFacts::load_with_projection(root, assets.registry, &json)?;
        let behaviours = Behaviours::load_with_gameplay(assets.registry, root, &json, &gameplay)?;
        (facts, behaviours)
    };
    println!("phase_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
    println!("total_ms={:.3}", total.elapsed().as_secs_f64() * 1000.0);
    println!("states={}", facts.states.len());
    std::hint::black_box((facts, behaviours));
    Ok(())
}
