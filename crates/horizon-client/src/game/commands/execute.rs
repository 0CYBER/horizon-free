//! `/execute` (ExecuteCommand): walk the parsed redirect chain, forking and
//! filtering command sources, then run the final command once per source.
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::text::Text;

/// `CommandSourceStack` fields Horizon models: the executing entity (None for
/// the server), position, rotation and anchor.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct Source {
    pub entity: Option<Target>,
    pub pos: [f64; 3],
    pub rot: [f32; 2],
    /// EntityAnchorArgument.Anchor: eyes (true) or feet.
    pub eyes: bool,
}

impl Game<'_> {
    /// The source a command typed by the player starts from.
    pub(super) fn player_source(&self) -> Source {
        Source {
            entity: Some(Target::Player),
            pos: self.source_position(),
            rot: self.source_rotation(),
            eyes: false,
        }
    }
    fn target_rotation(&self, t: Target) -> [f32; 2] {
        match t {
            Target::Player => [self.yaw, self.pitch],
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or([0.0; 2], |m| [m.yaw, m.head_pitch]),
            _ => [0.0; 2],
        }
    }
    fn target_eye_height(&self, t: Target) -> f64 {
        match t {
            Target::Player => self
                .player
                .as_ref()
                .map_or(1.62, |p| f64::from(p.dimensions.eye_height)),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or(0.0, |m| m.eye_y() - m.position.y),
            _ => 0.0,
        }
    }
    /// EntityAnchorArgument.Anchor.apply.
    pub(super) fn anchor_position(&self, s: &Source) -> [f64; 3] {
        match s.entity {
            Some(t) if s.eyes => [s.pos[0], s.pos[1] + self.target_eye_height(t), s.pos[2]],
            _ => s.pos,
        }
    }
    /// Evaluate `f` with `source` as the command source.
    fn with_source<T>(&mut self, source: Source, f: impl FnOnce(&mut Self) -> T) -> T {
        let previous = self.command_source.replace(source);
        let out = f(self);
        self.command_source = previous;
        out
    }
    /// ExecuteCommand: `ctx` is the context holding the `execute` literal.
    pub(super) fn run_execute(&mut self, ctx: &Context) -> R<i32> {
        let mut sources = vec![self.command_source.unwrap_or_else(|| self.player_source())];
        let mut c = ctx;
        loop {
            let word = c
                .literals
                .iter()
                .map(String::as_str)
                .find(|w| *w != "execute")
                .unwrap_or("");
            match word {
                "run" => {
                    let Some(command) = c.child.as_deref() else {
                        return Err(err("command.unknown.command", &[]));
                    };
                    let last = command.last_child();
                    let Some(id) = last.command else {
                        return Err(err("command.unknown.command", &[]));
                    };
                    // A nested execute walks its own chain.
                    let nested = command.literals.first().is_some_and(|w| w == "execute");
                    let mut total = 0;
                    for s in sources {
                        let result = self.with_source(s, |g| {
                            if nested {
                                g.run_execute(command)
                            } else {
                                g.execute_command(id, last)
                            }
                        });
                        match result {
                            Ok(n) => total += n,
                            // Forked executions report failures and carry on.
                            Err(e) => self.command_failure(e.message),
                        }
                    }
                    return Ok(total);
                }
                "if" | "unless" => {
                    let want = word == "if";
                    let mut kept = Vec::new();
                    let mut count = 0;
                    for s in sources {
                        let n = self.with_source(s, |g| g.condition(c))?;
                        if (n > 0) == want {
                            kept.push(s);
                            count += if want { n } else { 1 };
                        }
                    }
                    sources = kept;
                    if c.child.is_none() {
                        // A condition ending the chain is a test.
                        return if sources.is_empty() {
                            Err(err("commands.execute.conditional.fail", &[]))
                        } else {
                            let entity = c.has("entity") && want;
                            self.feedback(if entity {
                                Text::translate(
                                    "commands.execute.conditional.pass_count",
                                    &[Text::literal(count.to_string())],
                                )
                            } else {
                                Text::translate("commands.execute.conditional.pass", &[])
                            });
                            Ok(if entity { count } else { 1 })
                        };
                    }
                }
                _ => {
                    let mut next = Vec::new();
                    for s in sources {
                        next.extend(self.with_source(s, |g| g.modify(word, c, s))?);
                    }
                    sources = next;
                }
            }
            match c.child.as_deref() {
                Some(child) => c = child,
                None => return Err(err("command.unknown.command", &[])),
            }
        }
    }
    /// The sources one modifier produces from `s`.
    fn modify(&mut self, word: &str, c: &Context, s: Source) -> R<Vec<Source>> {
        let targets = |g: &mut Self| -> R<Vec<Target>> {
            // An empty selection forks to nothing rather than failing.
            match c.get("targets") {
                Some(Arg::Entity(sel)) => {
                    let sel = sel.clone();
                    Ok(g.select(&sel))
                }
                _ => Ok(Vec::new()),
            }
        };
        Ok(match word {
            "as" => targets(self)?
                .into_iter()
                .map(|t| Source {
                    entity: Some(t),
                    ..s
                })
                .collect(),
            "at" => targets(self)?
                .into_iter()
                .filter_map(|t| {
                    Some(Source {
                        pos: self.target_position(t)?,
                        rot: self.target_rotation(t),
                        ..s
                    })
                })
                .collect(),
            "positioned" if c.has("as") => targets(self)?
                .into_iter()
                .filter_map(|t| {
                    Some(Source {
                        pos: self.target_position(t)?,
                        eyes: false,
                        ..s
                    })
                })
                .collect(),
            "positioned" => {
                let pos = self
                    .position_arg(c, "pos")
                    .ok_or_else(|| err("argument.pos.missing.double", &[]))?;
                vec![Source {
                    pos,
                    eyes: false,
                    ..s
                }]
            }
            "rotated" if c.has("as") => targets(self)?
                .into_iter()
                .map(|t| Source {
                    rot: self.target_rotation(t),
                    ..s
                })
                .collect(),
            "rotated" => {
                let rot = match c.get("rot") {
                    Some(Arg::Coords(r)) => r.rotation(s.rot),
                    _ => s.rot,
                };
                vec![Source { rot, ..s }]
            }
            "facing" if c.has("entity") => {
                let eyes = matches!(c.get("anchor"), Some(Arg::Anchor(true)));
                targets(self)?
                    .into_iter()
                    .filter_map(|t| {
                        let p = self.target_position(t)?;
                        let y = if eyes {
                            p[1] + self.target_eye_height(t)
                        } else {
                            p[1]
                        };
                        Some(self.facing(s, [p[0], y, p[2]]))
                    })
                    .collect()
            }
            "facing" => {
                let to = self
                    .position_arg(c, "pos")
                    .ok_or_else(|| err("argument.pos.missing.double", &[]))?;
                vec![self.facing(s, to)]
            }
            "align" => {
                let Some(Arg::Str(axes)) = c.get("axes") else {
                    return Ok(vec![s]);
                };
                let mut pos = s.pos;
                for (i, axis) in ['x', 'y', 'z'].into_iter().enumerate() {
                    if axes.contains(axis) {
                        pos[i] = pos[i].floor();
                    }
                }
                vec![Source { pos, ..s }]
            }
            "anchored" => vec![Source {
                eyes: matches!(c.get("anchor"), Some(Arg::Anchor(true))),
                ..s
            }],
            "in" => {
                // Horizon keeps one dimension loaded; others cannot be a source.
                let Some(Arg::Resource(dim)) = c.get("dimension") else {
                    return Ok(vec![s]);
                };
                if dim != self.dim.id() {
                    return Err(err("argument.dimension.invalid", &[dim]));
                }
                vec![s]
            }
            _ => return Err(err("command.unknown.command", &[])),
        })
    }
    /// CommandSourceStack.facing: rotation from the anchor point to `to`.
    fn facing(&self, s: Source, to: [f64; 3]) -> Source {
        let from = self.anchor_position(&s);
        let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
        let horizontal = (dx * dx + dz * dz).sqrt();
        let pitch = (-(dy.atan2(horizontal).to_degrees())) as f32;
        let yaw = (dz.atan2(dx).to_degrees() - 90.0) as f32;
        Source {
            rot: [yaw.rem_euclid(360.0), pitch],
            ..s
        }
    }
    /// The count an if/unless condition yields (0 = false).
    fn condition(&mut self, c: &Context) -> R<i32> {
        if c.has("entity") {
            let Some(Arg::Entity(sel)) = c.get("entities").cloned() else {
                return Ok(0);
            };
            return Ok(self.select(&sel).len() as i32);
        }
        if c.has("loaded") {
            let p = self
                .coords(c, "pos")
                .map(|co| co.block(self.source_position(), self.source_rotation()));
            return Ok(
                p.is_some_and(|p| self.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4))) as i32,
            );
        }
        if c.has("dimension") {
            return Ok(
                matches!(c.get("dimension"), Some(Arg::Resource(d)) if d == self.dim.id()) as i32,
            );
        }
        if c.has("block") {
            let p = self.block_pos_arg(c, "pos")?;
            let Some(Arg::Block(b)) = c.get("block").cloned() else {
                return Ok(0);
            };
            let state = self.world.block(p[0], p[1], p[2]);
            return Ok(self.block_matches(&b, state) as i32);
        }
        Ok(0)
    }
}
