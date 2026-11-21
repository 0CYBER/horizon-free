//! The launcher's own update from its window (#9287). The window downloads and checks the new
//! launcher in the background ([`backend::stage_update`]) and says "A new version is available".
//! Update starts this launcher's file again as the installer and quits:
//! `horizon-launcher apply-update <tag> <staged file> [--restart [-- <the window's arguments>]]`.
//! The installer waits until the window's process has exited (its stdin is a pipe only that
//! process holds, which closes with it). If the launcher in place is already that version or newer
//! it changes nothing. Otherwise it checks the signed SHA-256 again and puts the new launcher in
//! place ([`backend::place_checked`], the same code `horizon-launcher update` uses), then starts
//! it. When that fails it starts the old one again, which says why ([`take_failure`]). A window
//! closed with an update staged hands it over the same way without the restart ([`on_close`]), and
//! says so while it waits, so the next start is the new launcher; a failure there is said at that
//! start. Only quitting for a data move drops it ([`forget`]), and the next start checks again.
//!
//! `main` handles the command before the CLI parser, so MCP and the completions never list it.
//! Every launcher keeps it: the installer is whatever launcher file is in place when Update is
//! pressed, so a window and its installer can be different versions and this command line can't
//! change. The SHA-256 travels in [`SHA256_ENV`], which an older installer ignores; an older window
//! gives none, and a file that can't be checked doesn't go in.
use crate::backend::{self, Staged};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

pub const COMMAND: &str = "apply-update";
const RESTART: &str = "--restart";
const SHA256_ENV: &str = "HORIZON_LAUNCHER_UPDATE_SHA256";
/// On a launcher started by a restart for links.json's `min_launcher` (`min_launcher`): the new
/// one, or the old one again after a failure, or this one when the new one was already in place.
/// That launcher doesn't restart for the minimum again. The window gives it to its installer,
/// which gives it to the launcher it starts; read once at startup and removed
/// ([`take_min_restarted_env`]), so no game or other program inherits it.
pub const MIN_RESTARTED_ENV: &str = "HORIZON_LAUNCHER_MIN_RESTARTED";
static MIN_RESTARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Notes [`MIN_RESTARTED_ENV`] and removes it from this process's environment. Called first at
/// startup, before other threads run.
pub fn take_min_restarted_env() {
    if std::env::var_os(MIN_RESTARTED_ENV).is_some() {
        MIN_RESTARTED.store(true, std::sync::atomic::Ordering::Relaxed);
        std::env::remove_var(MIN_RESTARTED_ENV);
    }
}

/// Whether a restart for the minimum started this launcher ([`MIN_RESTARTED_ENV`]).
pub fn min_restarted() -> bool {
    MIN_RESTARTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// How long the installer waits for the window to exit before it goes on: the replacement works
/// under a running launcher too (the old file is moved aside, not overwritten).
const WAIT: Duration = Duration::from_secs(60);

/// The update Update ([`start_now`]) or a closing window ([`on_close`]) hands over.
static PENDING: Mutex<Option<Staged>> = Mutex::new(None);

pub fn set_pending(staged: Option<Staged>) {
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = staged;
}

/// Drops a staged update without installing it, and its stage: the window quits for the data move,
/// whose worker copies the data folder the stage is in. The launcher it restarts checks again.
pub fn forget() {
    let staged = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(staged) = staged {
        backend::remove_stage(&backend::update_work_dir(), &staged.payload);
    }
}

/// What a closing window does with its staged update.
#[derive(Debug, PartialEq)]
enum Close {
    Nothing,
    /// Drop it with its stage, as [`forget`] does.
    Drop(Staged),
    /// Hand it to the installer, which doesn't start the launcher again.
    Install(Staged),
}

/// Whether closing the window installs a staged update: not in a copy that doesn't replace itself
/// (a development build, `HORIZON_LAUNCHER_NO_SELF_UPDATE`) or that a package manager owns.
pub fn installs_on_close() -> bool {
    installs_when(
        backend::self_update_allowed(),
        backend::package_managed_launcher(),
    )
}

fn installs_when(self_update_allowed: bool, package_managed: bool) -> bool {
    self_update_allowed && !package_managed
}

fn close_with(staged: Option<Staged>, installs: bool) -> Close {
    match staged {
        None => Close::Nothing,
        Some(staged) if installs => Close::Install(staged),
        Some(staged) => Close::Drop(staged),
    }
}

/// The window is closing: a staged update goes in once this process has exited, and the next start
/// is the new launcher. A game started from the window is a process of its own, which the
/// installer neither waits for nor touches. An installer that doesn't start is said at the next
/// start, as one that fails is ([`take_failure`]).
pub fn on_close() {
    let staged = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    match close_with(staged, installs_on_close()) {
        Close::Nothing => {}
        Close::Drop(staged) => backend::remove_stage(&backend::update_work_dir(), &staged.payload),
        Close::Install(staged) => {
            if let Err(e) = hand_over(&staged, false) {
                record_failure(&staged.tag, &e);
            }
        }
    }
}

/// Update was pressed: starts the installer for the staged update with a restart (true; the
/// caller quits right after), or reports that none is staged (false: the file in place already is
/// the new launcher, another window or `horizon-launcher update` put it there). `for_minimum`:
/// the restart is the one for links.json's `min_launcher` ([`MIN_RESTARTED_ENV`]).
pub fn start_now(for_minimum: bool) -> Result<bool, String> {
    let staged = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    match staged {
        Some(staged) => hand_over_with(&staged, true, for_minimum).map(|()| true),
        None => Ok(false),
    }
}

/// Starts the installer for `staged`; the caller quits right after. With `restart` the installer
/// starts the new launcher with this one's arguments.
pub fn hand_over(staged: &Staged, restart: bool) -> Result<(), String> {
    hand_over_with(staged, restart, false)
}

fn hand_over_with(staged: &Staged, restart: bool, for_minimum: bool) -> Result<(), String> {
    backend::mark_handed_over(staged);
    let installer = backend::update_target();
    let mut cmd = installer_command(
        &installer,
        staged,
        restart,
        for_minimum,
        std::env::args_os().skip(1),
    );
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Not in the terminal's process group: closing the window's terminal doesn't stop it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| {
        backend::remove_stage(&backend::update_work_dir(), &staged.payload);
        format!("Could not start {}: {e}", installer.display())
    })?;
    // The pipe's other end stays open until this process exits: the installer's sign to go on.
    std::mem::forget(child.stdin.take());
    Ok(())
}

fn installer_command(
    installer: &Path,
    staged: &Staged,
    restart: bool,
    for_minimum: bool,
    window_args: impl IntoIterator<Item = OsString>,
) -> Command {
    let mut cmd = crate::process::command(installer);
    cmd.args(installer_args(staged, restart, window_args))
        .env(SHA256_ENV, &staged.sha256);
    if for_minimum {
        cmd.env(MIN_RESTARTED_ENV, "1");
    }
    cmd
}

/// What the installer starts after it ran: the launcher in place (the new one, or the old one
/// after a failure) with the window's `args`, and [`MIN_RESTARTED_ENV`] when it was a restart
/// for the minimum.
fn restart_command(target: &Path, args: Vec<OsString>, for_minimum: bool) -> Command {
    let mut cmd = crate::process::command(target);
    cmd.args(args).env_remove(SHA256_ENV).stdin(Stdio::null());
    if for_minimum {
        cmd.env(MIN_RESTARTED_ENV, "1");
    }
    cmd
}

fn installer_args(
    staged: &Staged,
    restart: bool,
    window_args: impl IntoIterator<Item = OsString>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        COMMAND.into(),
        staged.tag.clone().into(),
        staged.payload.clone().into(),
    ];
    if restart {
        args.push(RESTART.into());
        args.push("--".into());
        args.extend(window_args);
    }
    args
}

/// What the installer was asked: the update, and the arguments to start the new launcher with.
#[derive(Debug, PartialEq)]
struct Request {
    staged: Staged,
    restart: Option<Vec<OsString>>,
}

/// `args` after the program's name and [`SHA256_ENV`]'s value (empty from an older window, which
/// no file matches); None unless they are an installer's whole command line.
fn parse(args: impl IntoIterator<Item = OsString>, sha256: String) -> Option<Request> {
    let mut args = args.into_iter();
    if args.next()? != COMMAND {
        return None;
    }
    let tag = args.next()?.into_string().ok()?;
    let payload = PathBuf::from(args.next()?);
    let restart = match args.next() {
        None => None,
        Some(flag) if flag == RESTART => match args.next() {
            None => Some(Vec::new()),
            Some(dashes) if dashes == "--" => Some(args.collect()),
            Some(_) => return None,
        },
        Some(_) => return None,
    };
    Some(Request {
        staged: Staged {
            tag,
            payload,
            sha256,
        },
        restart,
    })
}

/// The installer, when this process was started as one (Some(exit code)); None otherwise.
pub fn run() -> Option<i32> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first()? != COMMAND {
        return None;
    }
    let sha256 = std::env::var(SHA256_ENV).unwrap_or_default();
    let Some(request) = parse(args, sha256) else {
        eprintln!(
            "usage: horizon-launcher {COMMAND} <tag> <staged file> [{RESTART} [-- <arguments>]]"
        );
        return Some(2);
    };
    wait_for_window(WAIT);
    let result = install(&request.staged);
    // Gone either way: a failed update is downloaded again at the next try. Other launchers' stages
    // are theirs.
    backend::remove_stage(&backend::update_work_dir(), &request.staged.payload);
    if let Err(e) = &result {
        record_failure(&request.staged.tag, e);
    }
    if let Some(args) = request.restart {
        let mut cmd = restart_command(&backend::update_target(), args, min_restarted());
        if let Err(e) = cmd.spawn() {
            record_failure(&request.staged.tag, &format!("{e}"));
        }
    }
    Some(i32::from(result.is_err()))
}

fn wait_for_window(limit: Duration) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(limit);
}

fn install(staged: &Staged) -> Result<(), String> {
    let target = backend::update_target();
    install_locked(
        &backend::update_lock_file(&target),
        || backend::installed_launcher_version(&target),
        staged,
        &backend::update_work_dir(),
        backend::place_staged,
    )
}

/// [`install_over`] under the update lock ([`backend::lock_update`]), with the version in place
/// read only once it is held: an installer that waited for another one sees what that one put in.
fn install_locked(
    lock: &Path,
    installed: impl FnOnce() -> Option<String>,
    staged: &Staged,
    work: &Path,
    place: impl FnOnce(&Staged) -> Result<(), String>,
) -> Result<(), String> {
    let _lock = backend::lock_update(lock)?;
    install_over(installed().as_deref(), staged, work, place)
}

/// `installed` is the version of the launcher in place, which may have changed since this update
/// was staged: `horizon-launcher update` or another window can have put this version there, or a
/// newer one this older stage must not replace. Only what a launcher downloaded and whose signed
/// SHA-256 still holds goes in, never a file from elsewhere.
fn install_over(
    installed: Option<&str>,
    staged: &Staged,
    work: &Path,
    place: impl FnOnce(&Staged) -> Result<(), String>,
) -> Result<(), String> {
    if backend::already_in_place(installed, &staged.tag) {
        return Ok(());
    }
    backend::place_checked(work, staged, place)
}

fn failure_file() -> PathBuf {
    backend::data_dir().join(".launcher-update-failed")
}

/// The release that didn't go in on the first line, then why.
fn record_failure(tag: &str, error: &str) {
    let _ = fs::create_dir_all(backend::data_dir());
    let _ = fs::write(failure_file(), format!("{tag}\n{error}"));
}

/// Why the last update from the window didn't go in, once: the file is removed as it is read.
/// Nothing once this launcher is that release or a newer one (another copy, the command line or
/// the site put it in since), so a current launcher never says its update failed.
pub fn take_failure() -> Option<String> {
    let text = fs::read_to_string(failure_file()).ok()?;
    let _ = fs::remove_file(failure_file());
    failure_to_show(&text, backend::VERSION)
}

/// What [`take_failure`] shows of the failure file's `text` to launcher `version`. A file from a
/// launcher before the tag line is all error.
fn failure_to_show(text: &str, version: &str) -> Option<String> {
    let (tag, error) = match text.split_once('\n') {
        Some((tag, error)) if backend::launcher_version(tag.trim()).is_some() => {
            (Some(tag.trim()), error)
        }
        _ => (None, text),
    };
    if tag.is_some_and(|tag| backend::already_in_place(Some(version), tag)) {
        return None;
    }
    Some(error.trim().to_owned()).filter(|e| !e.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failure is shown to the launcher it failed to replace, and not to one already at that
    /// release or past it; a file without the tag line, from an older installer, still shows.
    #[test]
    fn a_failure_is_not_shown_once_its_release_is_in() {
        let file = "launcher@0.2.41\nCould not replace the launcher: denied";
        assert_eq!(
            failure_to_show(file, "0.2.40").as_deref(),
            Some("Could not replace the launcher: denied")
        );
        assert_eq!(failure_to_show(file, "0.2.41"), None);
        assert_eq!(failure_to_show(file, "0.3.0"), None);
        assert_eq!(
            failure_to_show("Could not start it\nsecond line", "9.9.9").as_deref(),
            Some("Could not start it\nsecond line")
        );
        assert_eq!(failure_to_show("launcher@0.2.41\n  ", "0.2.40"), None);
    }

    fn staged() -> Staged {
        Staged {
            tag: "launcher@0.2.41".into(),
            payload: PathBuf::from("/data/.launcher-update/1-ab/unpacked/Horizon.app"),
            sha256: "abc".into(),
        }
    }

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    /// What the window starts is what the installer reads back, the SHA-256 and the window's own
    /// arguments included (even one that looks like the installer's flags).
    #[test]
    fn the_installer_reads_the_command_line_the_window_gives_it() {
        let window = os(&["--restart", "play"]);
        let cmd = installer_command(
            Path::new("horizon-launcher"),
            &staged(),
            true,
            false,
            window.clone(),
        );
        let sha256 = cmd
            .get_envs()
            .find(|(name, _)| name.to_str() == Some(SHA256_ENV))
            .and_then(|(_, value)| value?.to_str())
            .unwrap()
            .to_owned();
        assert_eq!(
            parse(cmd.get_args().map(OsString::from), sha256),
            Some(Request {
                staged: staged(),
                restart: Some(window),
            })
        );
        // Closing the window: no restart.
        let args = installer_args(&staged(), false, os(&["play"]));
        assert_eq!(
            parse(args, "abc".into()),
            Some(Request {
                staged: staged(),
                restart: None,
            })
        );
    }

    /// Closing installs a staged update, without the restart, only where this copy replaces
    /// itself; elsewhere it is dropped. A data move drops it before the window quits.
    #[test]
    fn closing_installs_a_staged_update_only_where_the_launcher_updates_itself() {
        assert_eq!(close_with(None, true), Close::Nothing);
        assert_eq!(close_with(Some(staged()), true), Close::Install(staged()));
        let args = installer_args(&staged(), false, os(&["play"]));
        assert!(!args.iter().any(|a| a == RESTART));
        assert_eq!(parse(args, "abc".into()).unwrap().restart, None);
        assert_eq!(close_with(Some(staged()), false), Close::Drop(staged()));
        assert!(installs_when(true, false));
        for (allowed, package_managed) in [(false, false), (true, true), (false, true)] {
            assert!(!installs_when(allowed, package_managed));
        }
        // The data move: `forget` empties what the close would hand over.
        set_pending(Some(Staged {
            payload: std::env::temp_dir().join("horizon-no-stage"),
            ..staged()
        }));
        forget();
        let left = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
        assert_eq!(close_with(left, true), Close::Nothing);
    }

    fn env_of(cmd: &Command, name: &str) -> Option<Option<String>> {
        cmd.get_envs()
            .find(|(key, _)| key.to_str() == Some(name))
            .map(|(_, value)| value.and_then(|v| v.to_str()).map(str::to_owned))
    }

    /// A restart for links.json's minimum is marked from the window to its installer and from the
    /// installer to the launcher it starts, the new one or the old one after a failure; any other
    /// restart is not.
    #[test]
    fn a_restart_for_the_minimum_is_marked_through_the_installer() {
        let marked = Some(Some("1".to_owned()));
        let installer = |for_minimum| {
            installer_command(Path::new("i"), &staged(), true, for_minimum, os(&["play"]))
        };
        assert_eq!(env_of(&installer(true), MIN_RESTARTED_ENV), marked);
        assert_eq!(env_of(&installer(false), MIN_RESTARTED_ENV), None);

        let restart = restart_command(Path::new("horizon-launcher"), os(&["play"]), true);
        assert_eq!(env_of(&restart, MIN_RESTARTED_ENV), marked);
        assert_eq!(env_of(&restart, SHA256_ENV), Some(None));
        assert_eq!(
            restart.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("play")]
        );
        let plain = restart_command(Path::new("horizon-launcher"), os(&["play"]), false);
        assert_eq!(env_of(&plain, MIN_RESTARTED_ENV), None);
    }

    #[test]
    fn other_command_lines_are_not_the_installer() {
        let parse = |args: Vec<OsString>| parse(args, String::new());
        assert_eq!(parse(os(&["update"])), None);
        assert_eq!(parse(os(&[COMMAND, "launcher@0.2.41"])), None);
        assert_eq!(parse(os(&[COMMAND, "t", "/p", "--force"])), None);
        assert_eq!(parse(os(&[COMMAND, "t", "/p", RESTART, "x"])), None);
    }

    const OLD: Option<&str> = Some("0.2.40");
    const TAG: &str = "launcher@0.2.41";
    const NEW_LAUNCHER: &[u8] = b"the new launcher";

    /// A work folder under the temporary folder holding one stage: `horizon-launcher` packed in a
    /// `.tar.gz` and unpacked beside it, as a window leaves it.
    fn stage(name: &str) -> (PathBuf, PathBuf, Staged) {
        let work = std::env::temp_dir().join(format!("horizon-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let stage = backend::new_stage_folder(&work).unwrap();

        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(NEW_LAUNCHER.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "top/horizon-launcher", NEW_LAUNCHER)
            .unwrap();
        let archive = tar.into_inner().unwrap().finish().unwrap();
        fs::write(stage.join("download.tar.gz"), &archive).unwrap();

        let payload = stage.join("unpacked").join("top").join("horizon-launcher");
        fs::create_dir_all(payload.parent().unwrap()).unwrap();
        fs::write(&payload, NEW_LAUNCHER).unwrap();
        let staged = Staged {
            tag: TAG.into(),
            payload,
            sha256: horizon_release_sign::sha256_hex(&archive),
        };
        (work, stage, staged)
    }

    /// Runs the installer on a stage with a stand-in for placing it: what would have gone in.
    fn placed_by(
        installed: Option<&str>,
        staged: &Staged,
        work: &Path,
    ) -> (Result<(), String>, Option<Vec<u8>>) {
        let placed = std::cell::RefCell::new(None);
        let result = install_over(installed, staged, work, |new| {
            *placed.borrow_mut() = Some(fs::read(&new.payload).unwrap());
            Ok(())
        });
        (result, placed.into_inner())
    }

    /// The installer puts in only what a launcher staged in its own update folder: a command line
    /// naming any other file, or leaving its stage through `..`, changes nothing.
    #[test]
    fn the_installer_takes_only_a_staged_update() {
        let (work, stage, staged) = stage("not-staged");
        let elsewhere = work.with_extension("elsewhere");
        fs::write(&elsewhere, b"not a launcher").unwrap();
        for payload in [elsewhere.clone(), stage.join("..").join("z")] {
            let (result, placed) = placed_by(
                OLD,
                &Staged {
                    payload,
                    ..staged.clone()
                },
                &work,
            );
            assert!(result.is_err_and(|e| e.contains("not a launcher update")));
            assert_eq!(placed, None);
        }
        assert!(elsewhere.exists());
        let _ = fs::remove_file(elsewhere);
        let _ = fs::remove_dir_all(work);
    }

    /// The window can sit on a stage for days. An archive that no longer has its signed SHA-256 (a
    /// partial file, say) never goes in, and neither does one with no SHA-256 to check, as from
    /// a window older than this check.
    #[test]
    fn bytes_that_do_not_match_are_not_placed() {
        let (work, stage, staged) = stage("tampered");
        let archive = stage.join("download.tar.gz");

        let no_hash = Staged {
            sha256: String::new(),
            ..staged.clone()
        };
        let (result, placed) = placed_by(OLD, &no_hash, &work);
        assert!(result.is_err());
        assert_eq!(placed, None);

        let whole = fs::read(&archive).unwrap();
        fs::write(&archive, &whole[..whole.len() / 2]).unwrap();
        let (result, placed) = placed_by(OLD, &staged, &work);
        assert!(result.is_err_and(|e| e.contains("signed checksum")));
        assert_eq!(placed, None);
        let _ = fs::remove_dir_all(work);
    }

    /// What goes in comes from the bytes just checked: a damaged unpacked copy doesn't.
    #[test]
    fn the_new_launcher_comes_from_the_checked_archive() {
        let (work, _, staged) = stage("damaged-copy");
        fs::write(&staged.payload, b"half a launc").unwrap();
        let (result, placed) = placed_by(OLD, &staged, &work);
        assert_eq!(result, Ok(()));
        assert_eq!(placed.as_deref(), Some(NEW_LAUNCHER));
        let _ = fs::remove_dir_all(work);
    }

    /// An AppImage is the archive itself, with nothing to unpack.
    #[test]
    fn an_appimage_is_checked_as_it_is() {
        let (work, stage, staged) = stage("appimage");
        let image = stage.join("download.AppImage");
        fs::remove_file(stage.join("download.tar.gz")).unwrap();
        fs::write(&image, NEW_LAUNCHER).unwrap();
        let mut appimage = Staged {
            payload: image,
            sha256: horizon_release_sign::sha256_hex(NEW_LAUNCHER),
            ..staged
        };
        let (result, placed) = placed_by(OLD, &appimage, &work);
        assert_eq!(result, Ok(()));
        assert_eq!(placed.as_deref(), Some(NEW_LAUNCHER));

        appimage.sha256 = horizon_release_sign::sha256_hex(b"another AppImage");
        let (result, placed) = placed_by(OLD, &appimage, &work);
        assert!(result.is_err());
        assert_eq!(placed, None);
        let _ = fs::remove_dir_all(work);
    }

    /// Closing a window after `horizon-launcher update`: no failure and no downgrade, even when
    /// the stage is unusable by now. A launcher with no readable version gets the update.
    #[test]
    fn a_launcher_already_current_is_left_alone() {
        let (work, _, staged) = stage("already-current");
        let gone = Staged {
            payload: PathBuf::from("/nowhere"),
            ..staged.clone()
        };
        for current in ["0.2.41", "0.2.42", "0.3.0"] {
            assert_eq!(placed_by(Some(current), &gone, &work), (Ok(()), None));
        }
        let (result, placed) = placed_by(None, &staged, &work);
        assert_eq!(result, Ok(()));
        assert_eq!(placed.as_deref(), Some(NEW_LAUNCHER));
        let _ = fs::remove_dir_all(work);
    }

    /// Two windows staged the same release and closed: their installers go one after the other,
    /// and the second, which reads nothing before it has the lock, sees the first one's launcher
    /// and changes nothing.
    #[test]
    fn a_second_installer_waits_and_then_changes_nothing() {
        let (work, _, staged) = stage("two-installers");
        let lock = work.join("horizon-launcher.lock");
        let in_place = std::sync::Arc::new(Mutex::new(OLD.map(str::to_owned)));
        let held = backend::lock_update(&lock).unwrap();
        let second = std::thread::spawn({
            let (lock, work, staged, in_place) =
                (lock.clone(), work.clone(), staged.clone(), in_place.clone());
            move || {
                let placed = std::cell::Cell::new(false);
                let result = install_locked(
                    &lock,
                    || in_place.lock().unwrap().clone(),
                    &staged,
                    &work,
                    |_| {
                        placed.set(true);
                        Ok(())
                    },
                );
                (result, placed.get())
            }
        });
        // The first installer, holding the lock, puts the release in.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !second.is_finished(),
            "the second installer waits for the lock"
        );
        *in_place.lock().unwrap() = Some("0.2.41".into());
        drop(held);
        assert_eq!(second.join().unwrap(), (Ok(()), false));
        let _ = fs::remove_dir_all(work);
    }

    /// The startup cleanup leaves an installer's files alone while it holds the lock, and removes
    /// what is left once no installer does.
    #[test]
    fn the_cleanup_waits_for_an_installer() {
        let dir = std::env::temp_dir().join(format!("horizon-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let name = "horizon-launcher.exe";
        let lock = backend::update_lock_file(&dir.join(name));
        let aside = [format!("{name}.old-7-0"), format!("{name}.update-7")];
        for file in &aside {
            fs::write(dir.join(file), b"x").unwrap();
        }
        fs::write(dir.join(name), b"the launcher").unwrap();
        let held = backend::lock_update(&lock).unwrap();
        assert!(!backend::remove_replaced_once(&dir, name, 1, &lock));
        assert!(aside.iter().all(|file| dir.join(file).exists()));
        // Unlocked, not only closed: a process another test is starting right now can still hold
        // a copy of the descriptor until its exec, and on Unix that copy keeps the lock.
        held.unlock().unwrap();
        drop(held);
        assert!(backend::remove_replaced_once(&dir, name, 1, &lock));
        assert!(aside.iter().all(|file| !dir.join(file).exists()));
        assert!(dir.join(name).exists() && lock.exists());
        let _ = fs::remove_dir_all(dir);
    }

    /// A window from before stage folders hands the installer its old command line, with no SHA-256
    /// and a path in the old layout. Its restart still works, whatever becomes of the update.
    #[test]
    fn an_older_windows_command_line_is_still_understood() {
        let request = parse(
            os(&[
                COMMAND,
                TAG,
                "/data/.launcher-update/unpacked/x",
                RESTART,
                "--",
                "play",
            ]),
            String::new(),
        )
        .unwrap();
        assert_eq!(request.restart, Some(os(&["play"])));
        let work = Path::new("/data/.launcher-update");
        let (result, placed) = placed_by(OLD, &request.staged, work);
        assert!(result.is_err());
        assert_eq!(placed, None);
    }
}
