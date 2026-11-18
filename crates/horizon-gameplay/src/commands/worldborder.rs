//! `/worldborder`.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context},
    text::Text,
    worldborder::{MAX_CENTER, MAX_SIZE},
};

/// What `/worldborder` needs from the game: the border in place and the hooks that tell
/// the mods and the guests of a change.
pub trait WorldborderHost: CommandHost {
    /// `WorldBorder.getSize`.
    fn border_size(&self) -> f64;
    /// `WorldBorder.getLerpTime`: the ticks left of a change in flight.
    fn border_remaining_ticks(&self) -> i64;
    /// `WorldBorder.lerpSizeBetween` from the current size to `to` over `ticks`.
    fn border_lerp_to(&mut self, to: f64, ticks: i64);
    /// `WorldBorder.setSize`.
    fn border_set_size(&mut self, size: f64);
    /// The bounds-changed hook of the border, after a size change.
    fn border_bounds_changed(&mut self);
    /// `WorldBorder.getCenterX` and `getCenterZ`.
    fn border_center(&self) -> [f64; 2];
    /// `WorldBorder.setCenter`.
    fn border_set_center(&mut self, center: [f64; 2]);
    /// The center-changed hook of the border, after `border_set_center`.
    fn border_center_changed(&mut self);
    /// A position argument of the command, or the source's own position when it has none.
    fn position_arg_or_source(&self, ctx: &Context, name: &str) -> [f64; 3];
    /// `getDamagePerBlock` (`amount`) or `getDamageSafeZone` (otherwise).
    fn border_damage_slot(&self, amount: bool) -> f64;
    /// `setDamagePerBlock` (`amount`) or `setDamageSafeZone` (otherwise).
    fn set_border_damage_slot(&mut self, amount: bool, value: f64);
    /// `WorldBorder.getWarningBlocks`.
    fn border_warning_distance(&self) -> i32;
    /// `WorldBorder.setWarningBlocks`.
    fn set_border_warning_distance(&mut self, distance: i32);
    /// `WorldBorder.getWarningTime`.
    fn border_warning_time(&self) -> i32;
    /// `WorldBorder.setWarningTime`.
    fn set_border_warning_time(&mut self, ticks: i32);
}

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
/// String.format(Locale.ROOT, "%.<decimals>f"): Java rounds the shortest
/// decimal text half-up, which `{:.N}` (exact value, half-even) does not.
fn percent_f(v: f64, decimals: usize) -> String {
    horizon_core::java_number::format_fixed(v, decimals)
}
fn ticks_to_seconds(ticks: i64) -> String {
    percent_f(ticks as f64 / 20.0, 2)
}
/// `WorldBorderCommand.ERROR_TOO_BIG`'s argument: vanilla hands the message the
/// double `5.9999968E7` (the value `setSize` compares against too), and a `%s`
/// substitution prints a double with `Double.toString`.
fn size_limit_text() -> String {
    horizon_core::java_number::double_to_string(MAX_SIZE)
}

/// `/worldborder get|set|add|center|damage|warning`.
pub fn worldborder(host: &mut dyn WorldborderHost, id: &str, ctx: &Context) -> R<i32> {
    let float2 = |v: f64| percent_f(v, 2);
    let double = match ctx.get("distance") {
        Some(Arg::Double(d)) => *d,
        _ => 0.0,
    };
    let time = match ctx.get("time") {
        Some(Arg::Int(t)) => Some(i64::from(*t)),
        _ => None,
    };
    match id {
        "worldborder.get" => {
            let size = host.border_size();
            host.feedback(Text::translate(
                "commands.worldborder.get",
                &[Text::literal(percent_f(size, 0))],
            ));
            Ok((size + 0.5).floor() as i32)
        }
        "worldborder.set" | "worldborder.add" => {
            let current = host.border_size();
            // `add` measures from the size the border has now and, given a
            // time, keeps a change in flight going for its remaining ticks
            // plus the argument (`getLerpTime() + time`); `set` replaces it.
            let (distance, ticks) = match (id, time) {
                ("worldborder.add", Some(t)) => {
                    (current + double, host.border_remaining_ticks() + t)
                }
                ("worldborder.add", None) => (current + double, 0),
                (_, t) => (double, t.unwrap_or(0)),
            };
            if current == distance {
                return Err(host.error("commands.worldborder.set.failed.nochange", &[]));
            }
            if distance < 1.0 {
                return Err(host.error("commands.worldborder.set.failed.small", &[]));
            }
            if distance > MAX_SIZE {
                return Err(
                    host.error("commands.worldborder.set.failed.big", &[&size_limit_text()])
                );
            }
            let shown = Text::literal(percent_f(distance, 1));
            if ticks > 0 {
                host.border_lerp_to(distance, ticks);
                let key = if distance > current {
                    "commands.worldborder.set.grow"
                } else {
                    "commands.worldborder.set.shrink"
                };
                host.feedback(Text::translate(
                    key,
                    &[shown, Text::literal(ticks_to_seconds(ticks))],
                ));
            } else {
                host.border_set_size(distance);
                host.feedback(Text::translate(
                    "commands.worldborder.set.immediate",
                    &[shown],
                ));
            }
            host.border_bounds_changed();
            Ok((distance - current) as i32)
        }
        "worldborder.center" => {
            let p = host.position_arg_or_source(ctx, "pos");
            let (x, z) = (p[0], p[2]);
            if host.border_center() == [x, z] {
                return Err(host.error("commands.worldborder.center.failed", &[]));
            }
            if x.abs() > MAX_CENTER || z.abs() > MAX_CENTER {
                return Err(host.error("commands.worldborder.set.failed.far", &[&"2.9999984E7"]));
            }
            host.border_set_center([x, z]);
            host.border_center_changed();
            host.feedback(Text::translate(
                "commands.worldborder.center.success",
                &[Text::literal(float2(x)), Text::literal(float2(z))],
            ));
            Ok(0)
        }
        "worldborder.damage.amount" | "worldborder.damage.buffer" => {
            let amount = id.ends_with("amount");
            let value = match ctx.get(if amount { "damagePerBlock" } else { "distance" }) {
                Some(Arg::Float(v)) => f64::from(*v),
                _ => 0.0,
            };
            if host.border_damage_slot(amount) == value {
                return Err(host.error(
                    if amount {
                        "commands.worldborder.damage.amount.failed"
                    } else {
                        "commands.worldborder.damage.buffer.failed"
                    },
                    &[],
                ));
            }
            host.set_border_damage_slot(amount, value);
            host.feedback(Text::translate(
                if amount {
                    "commands.worldborder.damage.amount.success"
                } else {
                    "commands.worldborder.damage.buffer.success"
                },
                &[Text::literal(float2(value))],
            ));
            Ok(value as i32)
        }
        "worldborder.warning.distance" => {
            let d = match ctx.get("distance") {
                Some(Arg::Int(v)) => *v,
                _ => 0,
            };
            if host.border_warning_distance() == d {
                return Err(host.error("commands.worldborder.warning.distance.failed", &[]));
            }
            host.set_border_warning_distance(d);
            host.feedback(Text::translate(
                "commands.worldborder.warning.distance.success",
                &[num(d)],
            ));
            Ok(d)
        }
        _ => {
            let ticks = match ctx.get("time") {
                Some(Arg::Int(v)) => *v,
                _ => 0,
            };
            if host.border_warning_time() == ticks {
                return Err(host.error("commands.worldborder.warning.time.failed", &[]));
            }
            host.set_border_warning_time(ticks);
            host.feedback(Text::translate(
                "commands.worldborder.warning.time.success",
                &[Text::literal(ticks_to_seconds(i64::from(ticks)))],
            ));
            Ok(ticks)
        }
    }
}

#[cfg(test)]
mod tests {
    /// `WorldBorderCommand.ERROR_TOO_BIG` is built with the double
    /// `5.9999968E7`, the same value `setSize` compares `distance` against, and
    /// the `%s` substitution of a translatable renders a double with
    /// `Double.toString`: the message names `5.9999968E7`, not a rounded
    /// `5.999997E7`.
    #[test]
    fn the_too_big_message_names_the_limit_as_vanilla_prints_it() {
        assert_eq!(super::MAX_SIZE, 5.9999968E7);
        assert_eq!(super::size_limit_text(), "5.9999968E7");
    }
}
