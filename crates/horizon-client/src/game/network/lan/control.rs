//! LAN 控制命令的可选端口和关键词。

use super::LanSettings;
use horizon_client::Result;

pub(super) fn open_settings(args: &[&str]) -> Result<LanSettings> {
    let mut ports = args
        .iter()
        .copied()
        .filter(|arg| !matches!(*arg, "commands" | "force"));
    let port = ports
        .next()
        .map(str::parse::<u16>)
        .transpose()?
        .unwrap_or(0);
    if ports.next().is_some() {
        return Err("lan open accepts at most one port".into());
    }
    Ok(LanSettings {
        port,
        guest_commands: args.contains(&"commands"),
        force_game_mode: args.contains(&"force"),
    })
}

#[cfg(test)]
mod tests {
    use super::open_settings;

    #[test]
    fn lan_open_keywords_do_not_require_a_port() {
        let cases: &[(&[&str], u16, bool, bool)] = &[
            (&[], 0, false, false),
            (&["commands"], 0, true, false),
            (&["force"], 0, false, true),
            (&["commands", "force"], 0, true, true),
            (&["force", "commands"], 0, true, true),
            (&["0"], 0, false, false),
            (&["0", "commands", "force"], 0, true, true),
            (&["25565"], 25565, false, false),
            (&["25565", "commands"], 25565, true, false),
            (&["25565", "force"], 25565, false, true),
            (&["25565", "commands", "force"], 25565, true, true),
            (&["65535", "force", "commands"], 65535, true, true),
            (&["commands", "56010"], 56010, true, false),
            (&["force", "56010"], 56010, false, true),
            (&["commands", "56010", "force"], 56010, true, true),
            (&["force", "56010", "commands"], 56010, true, true),
            (&["commands", "force", "56010"], 56010, true, true),
            (&["force", "commands", "56010"], 56010, true, true),
            (&["commands", "0", "force"], 0, true, true),
            (&["force", "commands", "0"], 0, true, true),
            (&["commands", "65535"], 65535, true, false),
        ];
        for &(args, port, commands, force) in cases {
            let settings = open_settings(args).unwrap();
            assert_eq!(
                (
                    settings.port,
                    settings.guest_commands,
                    settings.force_game_mode
                ),
                (port, commands, force),
                "{args:?}",
            );
        }
        for port in ["invalid", "-1", "65536"] {
            for args in [
                [port, "commands", "force"],
                ["commands", port, "force"],
                ["force", "commands", port],
            ] {
                assert!(open_settings(&args).is_err(), "{args:?}");
            }
        }
        let extra_ports: &[&[&str]] = &[
            &["56010", "56011"],
            &["commands", "56010", "56011"],
            &["56010", "force", "56011"],
            &["56010", "56010"],
            &["0", "commands", "56010"],
            &["commands", "0", "0"],
            &["56010", "unknown"],
        ];
        for args in extra_ports {
            assert!(open_settings(args).is_err(), "{args:?}");
        }
    }
}
