//! Server mods' typed commands: an argument line parsed as the command's
//! arguments say, the way vanilla's Brigadier parsers read them, with
//! vanilla's error messages. The game sends the same arguments to players'
//! command trees, so their chat box checks them before the server does.
use crate::runtime::{Argument, ArgumentKind, ArgumentValue, BlockPos, ServerPlayer};

/// The values of `args`, typed by `player`, for `arguments` (in order;
/// optional ones left out aren't there), or the error the player reads.
pub fn parse(
    arguments: &[Argument],
    args: &str,
    player: &ServerPlayer,
    online: &[ServerPlayer],
) -> Result<Vec<(String, ArgumentValue)>, String> {
    let mut values = Vec::new();
    let mut rest = args;
    for (i, argument) in arguments.iter().enumerate() {
        if i > 0 && !rest.is_empty() {
            rest = rest.strip_prefix(' ').ok_or(
                "Expected whitespace to end one argument, but found trailing data".to_string(),
            )?;
        }
        if rest.is_empty() {
            if argument.optional {
                break;
            }
            return Err("Unknown or incomplete command, see below for error".into());
        }
        let (value, after) = one(&argument.kind, rest, player, online)?;
        values.push((argument.name.clone(), value));
        rest = after;
    }
    if !rest.is_empty() {
        return Err("Incorrect argument for command".into());
    }
    Ok(values)
}

/// StringReader.isAllowedInUnquotedString.
fn unquoted(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

/// The unquoted string at the start of `s`, and what follows it.
fn word(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !unquoted(c)).unwrap_or(s.len());
    s.split_at(end)
}

/// One argument from the start of `s`, and what follows it.
fn one<'a>(
    kind: &ArgumentKind,
    s: &'a str,
    player: &ServerPlayer,
    online: &[ServerPlayer],
) -> Result<(ArgumentValue, &'a str), String> {
    Ok(match kind {
        ArgumentKind::Word => {
            let (w, rest) = word(s);
            if w.is_empty() {
                return Err("Expected string".into());
            }
            (ArgumentValue::Word(w.into()), rest)
        }
        ArgumentKind::Text => (ArgumentValue::Text(s.into()), ""),
        ArgumentKind::Integer((min, max)) => {
            let (n, rest) = integer(s)?;
            if let Some(min) = min.filter(|m| n < *m) {
                return Err(format!("Integer must not be less than {min}, found {n}"));
            }
            if let Some(max) = max.filter(|m| n > *m) {
                return Err(format!("Integer must not be more than {max}, found {n}"));
            }
            (ArgumentValue::Integer(n), rest)
        }
        ArgumentKind::Number => {
            let (n, rest) = double(s)?;
            (ArgumentValue::Number(n), rest)
        }
        ArgumentKind::Boolean => {
            let (w, rest) = word(s);
            let b = match w {
                "true" => true,
                "false" => false,
                "" => return Err("Expected bool".into()),
                _ => {
                    return Err(format!(
                        "Invalid boolean, expected 'true' or 'false' but found '{w}'"
                    ));
                }
            };
            (ArgumentValue::Boolean(b), rest)
        }
        ArgumentKind::Player => {
            if let Some(selector) = s.strip_prefix('@') {
                let (kind, rest) = word(selector);
                return match kind {
                    // A player's @p is themselves (the nearest to where they are).
                    "s" | "p" => Ok((ArgumentValue::Player(player.clone()), rest)),
                    "a" | "e" | "r" | "n" => {
                        Err("Only one player is allowed, but the provided selector allows more than one".into())
                    }
                    _ => Err(format!("Unknown selector type '@{kind}'")),
                };
            }
            let (name, rest) = word(s);
            if name.is_empty() {
                return Err("Invalid name or UUID".into());
            }
            let found = online
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(name))
                .ok_or("No player was found")?;
            (ArgumentValue::Player(found.clone()), rest)
        }
        ArgumentKind::BlockPos => {
            let (x, rest) = coordinate(s, player.x)?;
            let rest = rest
                .strip_prefix(' ')
                .ok_or("Incomplete (expected 3 coordinates)")?;
            let (y, rest) = coordinate(rest, player.y)?;
            let rest = rest
                .strip_prefix(' ')
                .ok_or("Incomplete (expected 3 coordinates)")?;
            let (z, rest) = coordinate(rest, player.z)?;
            (ArgumentValue::BlockPos(BlockPos { x, y, z }), rest)
        }
    })
}

/// StringReader.readInt.
fn integer(s: &str) -> Result<(i32, &str), String> {
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.'))
        .unwrap_or(s.len());
    let (n, rest) = s.split_at(end);
    if n.is_empty() {
        return Err("Expected integer".into());
    }
    let n = n.parse().map_err(|_| format!("Invalid integer '{n}'"))?;
    Ok((n, rest))
}

/// StringReader.readDouble.
fn double(s: &str) -> Result<(f64, &str), String> {
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.'))
        .unwrap_or(s.len());
    let (n, rest) = s.split_at(end);
    if n.is_empty() {
        return Err("Expected double".into());
    }
    let n = n.parse().map_err(|_| format!("Invalid double '{n}'"))?;
    Ok((n, rest))
}

/// WorldCoordinate.parseInt, then BlockPos.containing: `~` and `~n` from
/// `base`, a whole number as it is.
fn coordinate(s: &str, base: f64) -> Result<(i32, &str), String> {
    if s.starts_with('^') {
        return Err(
            "Cannot mix world & local coordinates (everything must either use ^ or not)".into(),
        );
    }
    if let Some(offset) = s.strip_prefix('~') {
        if offset.is_empty() || offset.starts_with(' ') {
            return Ok((base.floor() as i32, offset));
        }
        let (n, rest) = double(offset)?;
        return Ok(((base + n).floor() as i32, rest));
    }
    integer(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::GameMode;

    fn player(name: &str) -> ServerPlayer {
        ServerPlayer {
            id: format!("id-{name}"),
            name: name.into(),
            x: 10.7,
            y: 64.0,
            z: -3.2,
            dimension: "minecraft:overworld".into(),
            game_mode: GameMode::Survival,
            health: 20.0,
        }
    }
    fn arg(name: &str, kind: ArgumentKind, optional: bool) -> Argument {
        Argument {
            name: name.into(),
            kind,
            optional,
        }
    }

    #[test]
    fn values_in_order_and_optional_ones_left_out() {
        let alex = player("Alex");
        let online = [alex.clone(), player("Steve")];
        let args = [
            arg("who", ArgumentKind::Player, false),
            arg("height", ArgumentKind::Integer((Some(1), Some(64))), true),
            arg("note", ArgumentKind::Text, true),
        ];
        let v = parse(&args, "steve 5 hello there", &alex, &online).unwrap();
        assert_eq!(v[0].0, "who");
        assert!(matches!(&v[0].1, ArgumentValue::Player(p) if p.name == "Steve"));
        assert_eq!(v[1], ("height".into(), ArgumentValue::Integer(5)));
        assert_eq!(
            v[2],
            ("note".into(), ArgumentValue::Text("hello there".into()))
        );
        let v = parse(&args, "@s", &alex, &online).unwrap();
        assert_eq!(v.len(), 1);
        assert!(matches!(&v[0].1, ArgumentValue::Player(p) if p.name == "Alex"));
    }

    #[test]
    fn vanilla_errors() {
        let alex = player("Alex");
        let online = [alex.clone()];
        let height = [arg(
            "height",
            ArgumentKind::Integer((Some(1), Some(64))),
            false,
        )];
        let err = |a: &[Argument], s| parse(a, s, &alex, &online).unwrap_err();
        assert_eq!(
            err(&height, "0"),
            "Integer must not be less than 1, found 0"
        );
        assert_eq!(
            err(&height, "65"),
            "Integer must not be more than 64, found 65"
        );
        assert_eq!(err(&height, "x"), "Expected integer");
        assert_eq!(err(&height, "1.5"), "Invalid integer '1.5'");
        assert_eq!(
            err(&height, ""),
            "Unknown or incomplete command, see below for error"
        );
        assert_eq!(err(&height, "3 4"), "Incorrect argument for command");
        let who = [arg("who", ArgumentKind::Player, false)];
        assert_eq!(err(&who, "Herobrine"), "No player was found");
        assert_eq!(err(&who, "@q"), "Unknown selector type '@q'");
        let flag = [arg("on", ArgumentKind::Boolean, false)];
        assert_eq!(
            err(&flag, "yes"),
            "Invalid boolean, expected 'true' or 'false' but found 'yes'"
        );
        let pos = [arg("at", ArgumentKind::BlockPos, false)];
        assert_eq!(err(&pos, "1 2"), "Incomplete (expected 3 coordinates)");
    }

    #[test]
    fn block_positions_relative_to_the_player() {
        let alex = player("Alex");
        let pos = [arg("at", ArgumentKind::BlockPos, false)];
        let v = parse(&pos, "~ ~1 ~-0.5", &alex, &[]).unwrap();
        assert_eq!(
            v[0].1,
            ArgumentValue::BlockPos(BlockPos {
                x: 10,
                y: 65,
                z: -4
            })
        );
        let v = parse(&pos, "-5 70 ~", &alex, &[]).unwrap();
        assert_eq!(
            v[0].1,
            ArgumentValue::BlockPos(BlockPos {
                x: -5,
                y: 70,
                z: -4
            })
        );
    }

    #[test]
    fn free_text_is_an_optional_text_argument() {
        let alex = player("Alex");
        let free = [arg("args", ArgumentKind::Text, true)];
        assert_eq!(parse(&free, "", &alex, &[]), Ok(Vec::new()));
        assert_eq!(
            parse(&free, "anything at all", &alex, &[]),
            Ok(vec![(
                "args".into(),
                ArgumentValue::Text("anything at all".into())
            )])
        );
        assert_eq!(
            parse(&[], "more", &alex, &[]),
            Err("Incorrect argument for command".into())
        );
        assert_eq!(parse(&[], "", &alex, &[]), Ok(Vec::new()));
    }
}
