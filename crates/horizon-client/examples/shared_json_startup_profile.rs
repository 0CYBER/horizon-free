//! Profile the fact and behaviour table phase of client startup.
//! Run in separate processes for baseline and shared JSON modes to capture peak RSS.
use horizon_client::{assets::Assets, blocks::BlockFacts};
use horizon_world::level::Behaviours;
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("expected resource directory")?;
    let mode = args.next().ok_or("expected baseline or shared")?;
    if mode != "baseline" && mode != "shared" {
        return Err("expected baseline or shared".into());
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
    } else {
        let json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(root.join("block-behaviours.json"))?,
        ))?;
        BlockFacts::load_with_behaviours(root, assets.registry, &json)?
    };
    println!("phase_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
    println!("total_ms={:.3}", total.elapsed().as_secs_f64() * 1000.0);
    println!("states={}", facts.states.len());
    std::hint::black_box((facts, behaviours));
    Ok(())
}
