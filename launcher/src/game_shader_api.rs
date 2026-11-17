//! The shader API lines the installed game serves, as the game says itself: `horizon-client
//! --launch-api`, `content.shader.api` (docs/api/launch.md). The launcher and the game it
//! installed are separate builds, and instances can play different games, so a registry shader
//! pack is offered, checked and marked by the lines of the game it is for, never by the line
//! this launcher was built with (#17527).
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// A game's answer by its version folder and the time its install finished (a developer build
/// is installed again under the same name); None when it gave no answer.
type Asked = BTreeMap<(PathBuf, Option<SystemTime>), Option<Vec<String>>>;
static ASKED: Mutex<Asked> = Mutex::new(BTreeMap::new());

/// How long a game may take to answer; the Nightly of 2026-11-13 takes a tenth of a second.
const PATIENCE: Duration = Duration::from_secs(5);

/// The game folder an operation is on and the lines of that folder's game. No folder: every
/// folder's operations go by these lines (a test's game).
type Current = (Option<PathBuf>, Vec<String>);

thread_local! {
    /// What the operation running on this thread is for ([`for_home`]).
    static OPERATION: RefCell<Option<Current>> = const { RefCell::new(None) };
}

/// One operation on a game folder (a search, an install, an update, a list of its packs): the
/// lines of the game that folder's instance plays, found once and the same for everything the
/// operation picks, checks and marks, whatever is selected or installed meanwhile.
#[must_use = "the operation lasts as long as this value"]
pub struct Operation {
    /// What ran around this operation, back when it ends; None when it changed nothing.
    outer: Option<Option<Current>>,
}

impl Drop for Operation {
    fn drop(&mut self) {
        if let Some(outer) = self.outer.take() {
            OPERATION.with(|current| *current.borrow_mut() = outer);
        }
    }
}

/// Begins an operation on the game folder `home`. Inside an operation on the same folder it
/// changes nothing. Inside one on another folder (an instance's command that installs a
/// modpack into its own profile) it goes by its own folder's game until it ends, and the outer
/// one's game comes back.
pub fn for_home(home: &Path) -> Operation {
    let same = OPERATION.with(|current| {
        let current = current.borrow();
        current
            .as_ref()
            .is_some_and(|(folder, _)| folder.as_deref().is_none_or(|folder| folder == home))
    });
    if same {
        return Operation { outer: None };
    }
    begin((Some(home.to_owned()), of_game(version_of(home))))
}

fn begin(operation: Current) -> Operation {
    let outer = OPERATION.with(|current| current.borrow_mut().replace(operation));
    Operation { outer: Some(outer) }
}

/// The shader API lines served by the game the running operation is for ([`for_home`]); outside
/// one, by the game the selected instance plays. With no game installed yet, or one that
/// doesn't say, the line this launcher's own build serves.
pub fn served() -> Vec<String> {
    if let Some((_, lines)) = OPERATION.with(|current| current.borrow().clone()) {
        return lines;
    }
    of_game(crate::instances::selected().resolve_version(None).ok())
}

/// The installed version the instance playing in `home` plays. A folder no instance plays in
/// yet (a modpack's profile being installed) gets the launcher's version, as a new instance
/// does.
fn version_of(home: &Path) -> Option<String> {
    match crate::instances::list()
        .into_iter()
        .find(|i| i.home == home)
    {
        Some(instance) => instance.resolve_version(None).ok(),
        None => crate::backend::resolve_version(None).ok(),
    }
}

fn of_game(tag: Option<String>) -> Vec<String> {
    tag.and_then(|tag| asked(&tag))
        .unwrap_or_else(|| vec![horizon_shader_api::API.to_owned()])
}

fn asked(tag: &str) -> Option<Vec<String>> {
    let dir = crate::backend::version_dir(tag);
    let installed = std::fs::metadata(dir.join(".installed")).and_then(|m| m.modified());
    let key = (dir, installed.ok());
    let known = || ASKED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(answer) = known().get(&key) {
        return answer.clone();
    }
    // A test never starts a game: it says what one answers (`tests::answers`).
    if cfg!(test) {
        return None;
    }
    // Not held while the game answers: a game that hangs holds up nobody else's question.
    let mut game = crate::process::command(crate::backend::client_program(tag));
    game.arg("--launch-api");
    let answer = reply(game, PATIENCE).and_then(|reply| lines_of(&reply));
    known().entry(key).or_insert(answer).clone()
}

/// What `program` prints when it ends well within `patience`. One that takes longer is stopped
/// and gives no answer.
fn reply(mut program: Command, patience: Duration) -> Option<Vec<u8>> {
    let mut child = program
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // Read beside the wait: a reply larger than the pipe would otherwise never end.
    let read = std::thread::spawn(move || {
        let mut reply = Vec::new();
        stdout.read_to_end(&mut reply).map(|_| reply)
    });
    let deadline = Instant::now() + patience;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let reply = read.join().ok()?.ok()?;
    status?.success().then_some(reply)
}

/// `content.shader.api` of a `--launch-api` reply.
fn lines_of(reply: &[u8]) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Reply {
        content: Content,
    }
    #[derive(serde::Deserialize)]
    struct Content {
        shader: Shader,
    }
    #[derive(serde::Deserialize)]
    struct Shader {
        api: Vec<String>,
    }
    let reply: Reply = serde_json::from_slice(reply).ok()?;
    Some(reply.content.shader.api)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Runs `run` as an operation on a game that serves the shader API lines `lines`.
    pub fn with_game<T>(lines: &[&str], run: impl FnOnce() -> T) -> T {
        let _game = begin((None, lines.iter().map(|line| (*line).to_owned()).collect()));
        run()
    }

    /// Installs a game as version `tag` that answers `--launch-api` with the lines `lines`.
    pub fn answers(tag: &str, lines: &[&str]) {
        let dir = crate::backend::version_dir(tag);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".installed"), "").unwrap();
        let installed = std::fs::metadata(dir.join(".installed")).and_then(|m| m.modified());
        let lines = lines.iter().map(|line| (*line).to_owned()).collect();
        ASKED
            .lock()
            .unwrap()
            .insert((dir, installed.ok()), Some(lines));
    }

    #[test]
    fn reads_the_lines_a_game_says_it_serves() {
        // The Nightly of 2026-11-13 (9f42064c11) and a game from main after #15886.
        let nightly =
            br#"{"launch_api": "1.3", "content": {"shader": {"schema": 1, "api": ["0.1"]}}}"#;
        assert_eq!(lines_of(nightly).unwrap(), ["0.1"]);
        let main =
            br#"{"content": {"mod": {"api": ["0.2"]}, "shader": {"schema": 1, "api": ["0.2"]}}}"#;
        assert_eq!(lines_of(main).unwrap(), ["0.2"]);
        let both = br#"{"content": {"shader": {"api": ["0.1", "0.2"]}}}"#;
        assert_eq!(lines_of(both).unwrap(), ["0.1", "0.2"]);
        // A game from before `content`, or one that says something else: no answer.
        for reply in [
            b"not json".as_slice(),
            br#"{"launch_api": "1.0", "options": []}"#,
            br#"{"content": {"shader": {"schema": 1}}}"#,
            br#"{"content": {"shader": {"api": "0.1"}}}"#,
        ] {
            assert_eq!(lines_of(reply), None);
        }
        // Without an installed game that answers, the launcher's own line stands in.
        assert_eq!(served(), [horizon_shader_api::API]);
        assert_eq!(with_game(&["0.1"], served), ["0.1"]);
        // A test's game stands for every folder's operation inside it.
        let inner = with_game(&["0.1"], || {
            let _inner = for_home(Path::new("another folder"));
            served()
        });
        assert_eq!(inner, ["0.1"]);
        assert_eq!(served(), [horizon_shader_api::API]);
    }

    /// A game that hangs is stopped after the wait and gives no answer; one that answers is
    /// read whole.
    #[test]
    fn a_game_that_does_not_answer_in_time_is_stopped() {
        const HANG: &str = "HORIZON_TEST_GAME_HANGS";
        match std::env::var(HANG).as_deref() {
            Ok("hang") => std::thread::sleep(Duration::from_secs(120)),
            Ok(_) => print!("{}", "reply ".repeat(20_000)),
            Err(_) => {
                let game = |hangs: &str| {
                    let mut game = Command::new(std::env::current_exe().unwrap());
                    game.args([
                        "--exact",
                        "game_shader_api::tests::a_game_that_does_not_answer_in_time_is_stopped",
                        "--nocapture",
                    ])
                    .env(HANG, hangs);
                    game
                };
                let started = Instant::now();
                assert_eq!(reply(game("hang"), Duration::from_millis(300)), None);
                assert!(started.elapsed() < Duration::from_secs(30));
                let whole = reply(game("answer"), Duration::from_secs(60)).unwrap();
                assert!(whole.len() > 120_000, "{}", whole.len());
            }
        }
    }
}
