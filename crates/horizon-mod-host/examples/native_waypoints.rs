//! Playable native API example. Run `cargo run -p horizon-mod-host --example native_waypoints --`
//! with prepared resources and a disposable game directory. In an owned single-player save,
//! use `/waypoint_set` to place a cyan marker. The most recent death appears in red after respawn.
use mod_api::{
    ClientHost, ClientTickPostEvent, CommandEvent, ConfigFileError, ConfigPath, Flow, Hooks,
    ModEntry, RegisterClientCommandsEvent, RenderLevelStage, RenderLevelStageEvent, WorldDepth,
    WorldLine,
};
use std::sync::Mutex;

#[derive(Default)]
struct Markers {
    path: Option<ConfigPath>,
    manual: Option<[i32; 3]>,
    death: Option<[i32; 3]>,
}

#[derive(Default)]
struct Waypoints(Mutex<Markers>);

/// `ConfigPath` rejects `:` and `\`, which an imported save folder can hold on Unix.
fn escape(part: &str) -> String {
    part.replace('%', "%25")
        .replace('/', "%2F")
        .replace(':', "%3A")
        .replace('\\', "%5C")
}

fn waypoint_path(save: &str, dimension: &str) -> Option<ConfigPath> {
    let (save, dimension) = (escape(save), escape(dimension));
    ConfigPath::new(&format!("{save}/{dimension}.json")).ok()
}

fn read_waypoint(host: &dyn ClientHost, path: &ConfigPath) -> Option<[i32; 3]> {
    match host.data_files().read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).ok(),
        Err(ConfigFileError::NotFound) => None,
        Err(error) => {
            log::warn!("Could not load the saved waypoint: {error}");
            None
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for Waypoints {
    fn register_client_commands(
        &self,
        _: &mut dyn ClientHost,
        event: &mut RegisterClientCommandsEvent<'_>,
    ) {
        event.commands.push(mod_api::Command::new("waypoint_set"));
    }

    fn command(&self, host: &mut dyn ClientHost, event: &mut CommandEvent<'_>) -> Flow {
        if event.command != "waypoint_set" {
            return Flow::Continue;
        }
        if let Some((path, position)) = host
            .client_session()
            .client_server()
            .and_then(|server| waypoint_path(&server.save_folder?, host.dimension()))
            .zip(host.local_player().and_then(|player| {
                player
                    .entity()
                    .position(host)
                    .map(|pos| pos.map(|axis| axis.floor() as i32))
            }))
        {
            let bytes = serde_json::to_vec(&position).expect("integer coordinates serialize");
            match host.data_files().write(&path, &bytes) {
                Ok(()) => {
                    let mut markers = self.0.lock().unwrap();
                    markers.path = Some(path);
                    markers.manual = Some(position);
                }
                Err(error) => log::warn!("Could not save the waypoint: {error}"),
            }
        }
        Flow::Cancel
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        let dimension = host.dimension();
        let path = host
            .client_session()
            .client_server()
            .and_then(|server| waypoint_path(&server.save_folder?, dimension));
        let mut markers = self.0.lock().unwrap();
        if markers.path != path {
            markers.manual = path.as_ref().and_then(|path| read_waypoint(host, path));
            markers.path = path;
        }
        markers.death = host
            .local_player()
            .and_then(|player| player.last_death_location(host))
            .filter(|death| death.dimension().to_string() == dimension)
            .map(|death| {
                let pos = death.pos();
                [pos.x, pos.y, pos.z]
            });
    }

    fn client_level_unload(
        &self,
        _: &mut dyn ClientHost,
        _: &mut mod_api::ClientLevelUnloadEvent<'_>,
    ) {
        *self.0.lock().unwrap() = Markers::default();
    }

    fn render_level_stage(&self, _: &dyn ClientHost, event: &mut RenderLevelStageEvent<'_>) {
        if event.stage != RenderLevelStage::AfterLevel {
            return;
        }
        let markers = self.0.lock().unwrap();
        for (point, color) in [(markers.manual, 0xff00ffff), (markers.death, 0xffff3030)] {
            if let Some([x, y, z]) = point {
                let from = [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5];
                let to = [from[0], from[1] + 8.0, from[2]];
                let _ = event.draw.lines(&[WorldLine::new(
                    from,
                    to,
                    color,
                    3.0,
                    WorldDepth::ThroughWalls,
                )]);
            }
        }
    }
}

fn main() {
    horizon_mod_host::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    horizon_mod_host::main_with(
        vec![ModEntry::new("waypoints", Box::<Waypoints>::default())],
        &[],
        &[],
        startup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waypoint_files_are_scoped_to_the_save_and_dimension() {
        let overworld = waypoint_path("Test World", "minecraft:overworld").unwrap();
        assert_eq!(overworld.as_str(), "Test World/minecraft%3Aoverworld.json");
        assert_ne!(
            Some(overworld.clone()),
            waypoint_path("Test World", "minecraft:the_nether")
        );
        assert_ne!(
            Some(overworld),
            waypoint_path("Test World (2)", "minecraft:overworld")
        );
        assert_ne!(
            waypoint_path("Test World", "example:foo/bar"),
            waypoint_path("Test World", "example:foo%2Fbar")
        );
        assert_eq!(waypoint_path("..", "minecraft:overworld"), None);
        assert!(waypoint_path("a:b\\c", "minecraft:overworld").is_some());
        assert_ne!(
            waypoint_path("a:b", "minecraft:overworld"),
            waypoint_path("a%3Ab", "minecraft:overworld")
        );
    }
}
