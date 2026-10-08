//! Control-socket commands for inspecting rendering: place blocks without
//! updates (`setblock`, `fill`) to build test scenes.
use super::Game;
use horizon_client::Result;
use horizon_world::registry::BlockStateId;
use serde_json::{Value, json};

pub const HELP: &[&str] = &[
    "setblock X Y Z STATE           set a block state (name[prop=value,...]) without updates",
    "fill X1 Y1 Z1 X2 Y2 Z2 STATE   fill a box with a block state without updates",
];

impl Game<'_> {
    /// Parse `name[k=v,...]` into a registry state (defaults for missing properties).
    fn parse_state(&self, text: &str) -> Result<BlockStateId> {
        let (name, props) = match text.split_once('[') {
            Some((n, rest)) => (n, rest.trim_end_matches(']')),
            None => (text, ""),
        };
        let name = if name.contains(':') {
            name.to_owned()
        } else {
            format!("minecraft:{name}")
        };
        let registry = &self.res.assets.registry;
        let default = registry
            .default_state(&name)
            .ok_or_else(|| format!("unknown block {name}"))?;
        let mut state = registry
            .state(default)
            .ok_or("missing default state")?
            .clone();
        for pair in props.split(',').filter(|p| !p.is_empty()) {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| format!("bad property {pair}"))?;
            if !state.properties.contains_key(k) {
                return Err(format!("{name} has no property {k}").into());
            }
            state.properties.insert(k.to_owned(), v.to_owned());
        }
        Ok(registry.resolve(&state)?)
    }
    pub(super) fn render_command(&mut self, words: &[&str]) -> Result<Option<Value>> {
        let int = |i: usize| -> Result<i32> {
            Ok(words
                .get(i)
                .ok_or_else(|| format!("missing argument {i}"))?
                .parse::<i32>()?)
        };
        match words[0] {
            "setblock" => {
                let state = self.parse_state(words.get(4).ok_or("missing state")?)?;
                self.world.set(int(1)?, int(2)?, int(3)?, state)?;
                Ok(Some(json!({ "ok": true, "state": state.0 })))
            }
            "fill" => {
                let state = self.parse_state(words.get(7).ok_or("missing state")?)?;
                let (a, b) = ([int(1)?, int(2)?, int(3)?], [int(4)?, int(5)?, int(6)?]);
                let mut count = 0;
                for x in a[0].min(b[0])..=a[0].max(b[0]) {
                    for y in a[1].min(b[1])..=a[1].max(b[1]) {
                        for z in a[2].min(b[2])..=a[2].max(b[2]) {
                            self.world.set(x, y, z, state)?;
                            count += 1;
                        }
                    }
                }
                Ok(Some(json!({ "ok": true, "blocks": count })))
            }
            other => Err(format!("unknown render command {other}").into()),
        }
    }
}
