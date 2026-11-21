//! Plain words for what stops a game from starting or installing.
use crate::i18n::{tr, trf};
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// The game's exit code when it can't mark its folders as in use (`holder::acquire` in
/// horizon-mod-package).
const LEASES_REFUSED: i32 = 78;

/// Whether `command` runs Horizon's own game with nothing in front of it. Only then is its exit
/// code the game's: a Java instance, a wrapper or a Unix pre-launch script can end with 78 too.
/// The launch plan's record of the game's command before wrappers (`run_leases::snapshot`)
/// decides, not the program's name: a wrapper may be called like the client.
pub(crate) fn runs_horizon_directly(command: &Command) -> bool {
    if !crate::run_leases::is_horizon(command) {
        return false;
    }
    let Ok(Some(game)) = crate::run_leases::game_snapshot(command) else {
        return false;
    };
    direct(
        command.get_program(),
        game.get_program(),
        staged(command).as_deref(),
    )
}

/// On Windows the game runs from its staged copy (`windows_runtime::redirect`).
#[cfg(windows)]
fn staged(command: &Command) -> Option<std::path::PathBuf> {
    let slot = command
        .get_envs()
        .find(|(key, _)| *key == crate::windows_runtime::SLOT)?
        .1?;
    Some(Path::new(slot).join("game").join(crate::backend::CLIENT))
}

#[cfg(not(windows))]
fn staged(_: &Command) -> Option<std::path::PathBuf> {
    None
}

fn direct(program: &OsStr, game: &OsStr, staged: Option<&Path>) -> bool {
    program == game || staged.is_some_and(|staged| staged.as_os_str() == program)
}

/// How a run ended badly: its words, and whether the game refused its folder leases. The kind
/// travels beside the words so the window never reads it back from translated text.
#[derive(Debug, PartialEq, Eq)]
pub struct ExitError {
    pub text: String,
    pub leases_refused: bool,
}

/// How a run that ended with `status` reads. A directly started game's lease refusal says
/// what to do; the crash card adds where to report it ([`crash_help`]).
pub fn exit_error(direct: bool, status: std::process::ExitStatus) -> ExitError {
    exit_text(direct, status.code(), &status.to_string())
}

fn exit_text(direct: bool, code: Option<i32>, status: &str) -> ExitError {
    let leases_refused = direct && code == Some(LEASES_REFUSED);
    let text = match leases_refused {
        true => tr("Horizon stopped before starting because its folders weren't marked as in use. Press Play again.").into(),
        false => format!("Exited with {status}"),
    };
    ExitError {
        text,
        leases_refused,
    }
}

/// The crash report a run that ended left, looked up by time with `lookup`. After a lease
/// refusal the game never started, so a report the lookup finds is an earlier run's.
pub fn crash_report<T>(leases_refused: bool, lookup: impl FnOnce() -> Option<T>) -> Option<T> {
    if leases_refused {
        return None;
    }
    lookup()
}

/// The crash card's line on what to report: with a crash report, without one, or after a lease
/// refusal. The game refuses before its log starts, so then the launcher's log is the one to
/// share; `glass` picks the label the look gives its tab (`home_tabs`, `glass_home_tabs`).
pub fn crash_help(report: bool, refused: bool, glass: bool) -> String {
    if refused {
        let tab = if glass { tr("Log") } else { tr("Console") };
        return trf(
            "If this repeats, open the {tab} tab, press Share log… and post the link on Discord.",
            &[("tab", tab)],
        );
    }
    tr(if report {
        "When you report it on Discord, attach the crash report file (or paste it with Copy report) and say what you were doing."
    } else {
        "The game ended without a crash report. When you report it on Discord, attach latest.log from the logs folder and say what you were doing."
    })
    .into()
}

fn shown(path: &Path) -> String {
    crate::backend::plain(path.to_owned()).display().to_string()
}

/// Whether the data folder comes from the environment: Settings can't move it then
/// (`location_page::environment_blocked`).
fn data_from_environment() -> bool {
    ["HORIZON_LAUNCHER_HOME", "HORIZON_HOME"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

/// A refused file system: at `folder` when the error knows it, against the data folder `data`.
pub(crate) fn unsupported_filesystem(folder: Option<&Path>, data: &Path) -> String {
    unsupported_text(folder, data, data_from_environment())
}

fn unsupported_text(folder: Option<&Path>, data: &Path, environment: bool) -> String {
    match folder {
        Some(folder) if folder == data && environment => trf(
            "Horizon can't use {folder}: its file system isn't supported. Set HORIZON_LAUNCHER_HOME to a folder on a local drive.",
            &[("folder", &shown(folder))],
        ),
        Some(folder) if folder == data => trf(
            "Horizon can't use {folder}: its file system isn't supported. Move the launcher's data to a local drive in Settings > About > Data folder.",
            &[("folder", &shown(folder))],
        ),
        Some(folder) => trf(
            "Horizon can't use {folder}: its file system isn't supported. It must be on a local drive.",
            &[("folder", &shown(folder))],
        ),
        None => tr("A folder Horizon uses is on a file system it doesn't support. Keep the launcher's data and game folders on a local drive.").into(),
    }
}

/// A refused entry at `path`: a link, or not the kind of entry expected there.
pub(crate) fn unexpected_entry(path: &Path, data: &Path) -> String {
    let link = std::fs::symlink_metadata(path).is_ok_and(|meta| crate::native_cache::linked(&meta));
    unexpected_text(path, data, link)
}

fn unexpected_text(path: &Path, data: &Path, link: bool) -> String {
    match path == data && link {
        // Settings can't move a linked data folder either: relocation refuses a linked source
        // (`validate_roots` in `data_location::migrate_locked`), so only the environment can
        // point past it.
        true => trf(
            "{folder} is a link. Set HORIZON_LAUNCHER_HOME to the real folder.",
            &[("folder", &shown(path))],
        ),
        false => trf(
            "Refusing native cache cleanup at an unexpected file or link: {path}",
            &[("path", &path.display().to_string())],
        ),
    }
}

/// A Play refused because a folder couldn't be opened or written.
pub(crate) fn unwritable(error: &str, game: Option<&Path>, data: &Path) -> String {
    match game {
        Some(game) => trf(
            "{error}. Make sure your account can write to {game} and {data}, then try again.",
            &[
                ("error", error),
                ("game", &shown(game)),
                ("data", &shown(data)),
            ],
        ),
        None => trf(
            "{error}. Make sure your account can write to {data}, then try again.",
            &[("error", error), ("data", &shown(data))],
        ),
    }
}

/// Where a release without a build for this computer leaves the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Host {
    /// Nightlies since nightly-2026-11-18 ship only the Apple Silicon game.
    IntelMac,
    /// The x86_64 launcher under Rosetta, which looks for the Intel game that no longer ships.
    TranslatedMac,
    Other,
}

fn host_for(os: &str, arch: &str, translated: bool) -> Host {
    match (os, arch) {
        ("macos", "x86_64") if translated => Host::TranslatedMac,
        ("macos", "x86_64") => Host::IntelMac,
        _ => Host::Other,
    }
}

fn host() -> Host {
    static HOST: std::sync::OnceLock<Host> = std::sync::OnceLock::new();
    *HOST.get_or_init(|| host_for(std::env::consts::OS, std::env::consts::ARCH, translated()))
}

#[cfg(target_os = "macos")]
fn translated() -> bool {
    let mut value: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated, and value and size describe one writable c_int.
    let result = unsafe {
        libc::sysctlbyname(
            c"sysctl.proc_translated".as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    // Macs without Rosetta don't know the name: not translated.
    result == 0 && value == 1
}

#[cfg(not(target_os = "macos"))]
fn translated() -> bool {
    false
}

fn launcher_download() -> String {
    crate::backend::download_page(&crate::backend::Links::default().website)
}

/// The version list's line for a release with no build here, and the page a click opens.
pub fn no_build_card() -> (String, Option<String>) {
    card_for(host(), launcher_download())
}

fn card_for(host: Host, download: String) -> (String, Option<String>) {
    match host {
        Host::Other => (tr("No build for this platform").into(), None),
        Host::IntelMac => (
            tr("This version has no Intel Mac build. A version you already have keeps working.")
                .into(),
            None,
        ),
        Host::TranslatedMac => (
            tr("You're running the Intel launcher on an Apple Silicon Mac. Download the Apple Silicon launcher.")
                .into(),
            Some(download),
        ),
    }
}

/// Why `tag` (or, with `None`, the newest release) can't be installed here.
pub fn no_build(tag: Option<&str>) -> String {
    no_build_for(host(), tag, &launcher_download())
}

fn no_build_for(host: Host, tag: Option<&str>, download: &str) -> String {
    match (host, tag) {
        (Host::Other, Some(tag)) => trf("{tag} has no build for this platform.", &[("tag", tag)]),
        (Host::Other, None) => tr("No release has a build for this platform yet.").into(),
        (Host::IntelMac, _) => {
            tr("This version has no Intel Mac build. A version you already have keeps working.")
                .into()
        }
        (Host::TranslatedMac, _) => trf(
            "You're running the Intel launcher on an Apple Silicon Mac. Download the Apple Silicon launcher: {url}",
            &[("url", download)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_directly_started_horizon_game_owns_its_exit_code() {
        let installed = Path::new("versions")
            .join("nightly")
            .join(crate::backend::CLIENT);
        let staged = Path::new("run")
            .join("slot")
            .join("game")
            .join(crate::backend::CLIENT);
        assert!(direct(installed.as_os_str(), installed.as_os_str(), None));
        assert!(direct(
            staged.as_os_str(),
            installed.as_os_str(),
            Some(&staged)
        ));
        // A wrapper named like the client, elsewhere, is still a wrapper.
        let lookalike = Path::new("tools").join(crate::backend::CLIENT);
        assert!(!direct(
            lookalike.as_os_str(),
            installed.as_os_str(),
            Some(&staged)
        ));

        // As the launch plan leaves them: the game alone, a lookalike wrapper, the Unix
        // pre-launch script, and a Java instance (never marked, so never snapshotted).
        let mut game = Command::new(&installed);
        crate::run_leases::mark(&mut game, Path::new("versions/nightly"));
        crate::run_leases::snapshot(&mut game);
        assert!(runs_horizon_directly(&game));
        let around = |program: &Path| {
            let mut out = Command::new(program);
            for (key, value) in game.get_envs() {
                if let Some(value) = value {
                    out.env(key, value);
                }
            }
            out
        };
        assert!(!runs_horizon_directly(&around(&lookalike)));
        assert!(!runs_horizon_directly(&around(Path::new("/bin/sh"))));
        let mut marked_only = Command::new(&installed);
        crate::run_leases::mark(&mut marked_only, Path::new("versions/nightly"));
        assert!(!runs_horizon_directly(&marked_only));
        assert!(!runs_horizon_directly(&Command::new(&installed)));
    }

    #[test]
    fn the_crash_card_names_the_tab_the_look_has() {
        assert_eq!(
            crash_help(false, true, true),
            "If this repeats, open the Log tab, press Share log… and post the link on Discord."
        );
        assert_eq!(
            crash_help(false, true, false),
            "If this repeats, open the Console tab, press Share log… and post the link on Discord."
        );
        assert!(crash_help(false, false, true).contains("attach latest.log"));
        assert!(crash_help(true, false, false).contains("crash report file"));
    }

    #[test]
    fn only_the_games_code_78_reads_as_a_lease_refusal() {
        let generic = |text: &str| ExitError {
            text: text.into(),
            leases_refused: false,
        };
        assert_eq!(
            exit_text(true, Some(78), "exit code: 78"),
            ExitError {
                text: "Horizon stopped before starting because its folders weren't marked as in use. Press Play again.".into(),
                leases_refused: true,
            }
        );
        assert_eq!(
            exit_text(false, Some(78), "exit code: 78"),
            generic("Exited with exit code: 78")
        );
        assert_eq!(
            exit_text(true, Some(1), "exit code: 1"),
            generic("Exited with exit code: 1")
        );
        assert_eq!(
            exit_text(true, None, "signal: 9"),
            generic("Exited with signal: 9")
        );
    }

    #[test]
    fn a_lease_refusal_never_shows_an_earlier_runs_crash_report() {
        let earlier = Some(std::path::PathBuf::from("crash-reports/crash-earlier.txt"));
        assert_eq!(
            crash_report(true, || -> Option<std::path::PathBuf> {
                panic!("looked up a report after a lease refusal")
            }),
            None
        );
        assert_eq!(crash_report(false, || earlier.clone()), earlier);
    }

    #[test]
    fn storage_refusals_name_the_folder() {
        let data = Path::new("/home/player/.local/share/horizon-launcher");
        let game = Path::new("/mnt/nfs/survival");
        assert_eq!(
            unsupported_text(Some(data), data, false),
            "Horizon can't use /home/player/.local/share/horizon-launcher: its file system isn't supported. Move the launcher's data to a local drive in Settings > About > Data folder."
        );
        assert_eq!(
            unsupported_text(Some(data), data, true),
            "Horizon can't use /home/player/.local/share/horizon-launcher: its file system isn't supported. Set HORIZON_LAUNCHER_HOME to a folder on a local drive."
        );
        // An instance on NFS, with the data folder local: the instance is named, not the data.
        assert_eq!(
            unsupported_text(Some(game), data, false),
            "Horizon can't use /mnt/nfs/survival: its file system isn't supported. It must be on a local drive."
        );
        assert_eq!(
            unsupported_text(None, data, false),
            "A folder Horizon uses is on a file system it doesn't support. Keep the launcher's data and game folders on a local drive."
        );

        // Settings can't move a linked data folder, with or without the override.
        assert_eq!(
            unexpected_text(data, data, true),
            "/home/player/.local/share/horizon-launcher is a link. Set HORIZON_LAUNCHER_HOME to the real folder."
        );
        let generic = "Refusing native cache cleanup at an unexpected file or link: /home/player/.local/share/horizon-launcher";
        assert_eq!(unexpected_text(data, data, false), generic);
        let inner = data.join("native");
        assert!(unexpected_text(&inner, data, true).starts_with("Refusing native cache"));

        let game = Path::new("/home/player/games/survival");
        assert_eq!(
            unwritable("Permission denied", Some(game), data),
            "Permission denied. Make sure your account can write to /home/player/games/survival and /home/player/.local/share/horizon-launcher, then try again."
        );
        assert_eq!(
            unwritable("Permission denied", None, data),
            "Permission denied. Make sure your account can write to /home/player/.local/share/horizon-launcher, then try again."
        );
    }

    #[test]
    fn a_file_at_the_data_path_is_not_called_a_link() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("horizon-launcher");
        std::fs::write(&data, b"").unwrap();
        assert!(
            unexpected_entry(&data, &data).starts_with("Refusing native cache cleanup"),
            "{}",
            unexpected_entry(&data, &data)
        );
    }

    #[test]
    fn a_refused_file_system_keeps_the_first_folder_named() {
        use crate::native_cache::Error;
        let (game, data) = (Path::new("/mnt/nfs/survival"), Path::new("/data"));
        assert!(matches!(
            Error::UnsupportedFilesystem(None).on(game).on(data),
            Error::UnsupportedFilesystem(Some(folder)) if folder == game
        ));
        assert!(matches!(Error::Busy.on(game), Error::Busy));
    }

    #[test]
    fn a_mac_without_its_build_says_which_launcher_or_version_to_use() {
        assert_eq!(host_for("macos", "x86_64", false), Host::IntelMac);
        assert_eq!(host_for("macos", "x86_64", true), Host::TranslatedMac);
        assert_eq!(host_for("macos", "aarch64", false), Host::Other);
        assert_eq!(host_for("linux", "x86_64", false), Host::Other);
        // The flag only exists for macOS processes.
        assert_eq!(host_for("windows", "x86_64", true), Host::Other);

        let url = "https://playhorizon.dev/docs/install";
        assert_eq!(
            no_build_for(Host::Other, Some("nightly-2026-11-20"), url),
            "nightly-2026-11-20 has no build for this platform."
        );
        assert_eq!(
            no_build_for(Host::Other, None, url),
            "No release has a build for this platform yet."
        );
        let intel =
            "This version has no Intel Mac build. A version you already have keeps working.";
        assert_eq!(
            no_build_for(Host::IntelMac, Some("nightly-2026-11-20"), url),
            intel
        );
        assert_eq!(no_build_for(Host::IntelMac, None, url), intel);
        assert_eq!(
            no_build_for(Host::TranslatedMac, None, url),
            "You're running the Intel launcher on an Apple Silicon Mac. Download the Apple Silicon launcher: https://playhorizon.dev/docs/install"
        );

        assert_eq!(
            card_for(Host::Other, url.into()),
            ("No build for this platform".into(), None)
        );
        assert_eq!(card_for(Host::IntelMac, url.into()), (intel.into(), None));
        assert_eq!(
            card_for(Host::TranslatedMac, url.into()).1.as_deref(),
            Some(url)
        );
    }
}
