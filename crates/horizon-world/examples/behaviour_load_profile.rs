//! Measure registry and behaviour load times using prepared client resources.
//! `behaviour_load_profile RESOURCES [CACHE_DIR]`: with CACHE_DIR, loads use and
//! refresh the snapshot there, as the client does with its `.horizon-cache`.
use horizon_world::level::{Behaviours, data::load_registry};
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .expect("resource directory argument");
    let root = Path::new(&root);
    if let Some(dir) = std::env::args().nth(2) {
        Behaviours::set_cache_dir(dir.into());
    }
    let started = Instant::now();
    let registry = load_registry(root)?;
    eprintln!(
        "registry_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    for iteration in 0..3 {
        let started = Instant::now();
        let behaviours = Behaviours::load(&registry, root)?;
        eprintln!(
            "behaviours_{}_ms={:.3}",
            iteration,
            started.elapsed().as_secs_f64() * 1000.0
        );
        std::hint::black_box(behaviours);
    }
    Ok(())
}
