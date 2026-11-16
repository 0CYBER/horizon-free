//! Emit the fixture's selected catalog through the ordinary point scanner.
//! Run `cargo run -p horizon-mod-api-gen --features listener-fixture
//! --example listener-fixture-catalog -- crates/horizon-mod-wasm/tests/fixture/points.json`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let points = horizon_mod_api_gen::points::game_points(&root)?;
    let points: Vec<_> = points
        .into_iter()
        .filter(|point| {
            point.name.starts_with("listener_fixture_")
                || matches!(
                    point.name.as_str(),
                    "tick"
                        | "game_server_lifecycle_remember_shutdown"
                        | "game_beacon_average"
                        | "game_commands_wrap_degrees"
                )
        })
        .collect();
    let output = std::env::args_os().nth(1).ok_or("expected output path")?;
    std::fs::write(
        output,
        serde_json::to_vec(&serde_json::json!({
            "schema": point_shape::build_data::SCHEMA,
            "build": {"commit": "fixture"},
            "points": points,
        }))?,
    )?;
    Ok(())
}
