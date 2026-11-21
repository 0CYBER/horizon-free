//! LAN 控制命令的可选端口和关键词。

use super::LanSettings;
use horizon_client::Result;

pub(super) fn open_settings(args: &[&str]) -> Result<LanSettings> {
    let port = match args.first().copied() {
        None | Some("commands" | "force") => 0,
        Some(port) => port.parse::<u16>()?,
    };
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
            assert!(
                open_settings(&[port, "commands", "force"]).is_err(),
                "{port}"
            );
        }
    }
}
