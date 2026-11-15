//! Account and unsigned-developer policy for CLI Play.
use super::*;

/// [`client`], playing as `who` for this launch only (`play --account`).
pub fn client_as(tag: &str, who: Option<&str>) -> Result<Command, String> {
    if let Some(error) = game_architecture_launch_error(tag) {
        return Err(error);
    }
    play_step("reading the sign-in");
    let hint = |refused: String| format!("{refused} {}", crate::i18n::tr(SIGN_IN_HINT));
    let session = if crate::developer_offline::enabled(tag) && who.is_none() {
        crate::developer_offline::require_capability(tag)?;
        None
    } else {
        Some(match who {
            None => account::session_for_game(&game_home()).map_err(|refused| {
                // A keyring that didn't answer isn't a missing sign-in (#12179).
                match refused.keyring {
                    Some(why) => why,
                    None => hint(refused.to_string()),
                }
            }),
            Some(who) => account::session_for_game_as(&game_home(), who).map_err(hint),
        }?)
    };
    if let Err(e) = need_prepared(tag) {
        account::discard_unread_sessions();
        return Err(e);
    }
    play_step("checking the game's assets");
    complete_assets(tag);
    crate::browser::horizon::before_play(&game_home());
    let mut cmd = client_command(tag);
    if let Some(session) = session {
        if let Some(file) = session.file {
            cmd.arg("--session-file").arg(file);
        }
        cmd.envs(session.env);
        if let Some(e) = session.offline {
            eprintln!("Playing offline: {e}");
        }
        if let Some(e) = account::take_unsaved() {
            eprintln!("{e}");
        }
    } else {
        crate::developer_offline::mark(&mut cmd);
        eprintln!("{}", crate::i18n::tr(crate::developer_offline::NOTICE));
    }
    append_game_args(&mut cmd);
    Ok(cmd)
}
