//! Mod lines and progress bars on the startup loading screen: NeoForge's
//! `StartupNotificationManager` (`addModMessage`, `addProgressBar`, `popBar`) and `ProgressMeter`.
use crate::wire::{Reader, Wire, WireError, Writer};
use std::sync::{Arc, OnceLock};

/// A startup progress operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum StartupProgressError {
    /// The screen already has 64 open bars. Close a bar before retrying.
    TooManyBars,
    /// The game's screen has already been installed.
    AlreadyInstalled,
    /// The screen callback could not cross the mod boundary.
    Refused {
        /// The mod whose callback was refused.
        source: crate::Identifier,
        /// Why the boundary refused the call.
        reason: String,
    },
}
impl std::fmt::Display for StartupProgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyBars => f.write_str("the startup screen already has 64 open bars"),
            Self::AlreadyInstalled => f.write_str("the startup screen is already installed"),
            Self::Refused { source, reason } => write!(f, "{source}: {reason}"),
        }
    }
}
impl std::error::Error for StartupProgressError {}
impl From<(crate::Identifier, String)> for StartupProgressError {
    fn from((source, reason): (crate::Identifier, String)) -> Self {
        Self::Refused { source, reason }
    }
}

/// The game's side of the startup loading screen. A mod reaches it through
/// [`RegistrationContext::progress`](crate::RegistrationContext::progress) and
/// [`RegistrationContext::message`](crate::RegistrationContext::message), not directly. A wasm
/// mod holds the game's screen by id and each call crosses to the game.
pub trait StartupScreen: Send + Sync + 'static {
    /// Open a bar of `steps` steps under the stage bar; the answer names it.
    /// Returns `StartupProgressError::TooManyBars` if 64 bars are already open.
    fn open(&self, label: String, steps: u32) -> Result<u32, StartupProgressError>;
    /// One more of bar `bar`'s steps is done.
    fn step(&self, bar: u32);
    /// Bar `bar` has done `done` of its steps.
    fn set(&self, bar: u32, done: u32);
    /// Bar `bar` reads `label` from now on.
    fn label(&self, bar: u32, label: String);
    /// Bar `bar` is complete: it leaves the screen.
    fn close(&self, bar: u32);
    /// A line on the screen that fades after a few seconds.
    fn message(&self, text: String);
}
crate::wire::wire_handlers::handler!(dyn StartupScreen, "StartupScreen");

static SCREEN: OnceLock<Arc<dyn StartupScreen>> = OnceLock::new();

/// Install the game's startup screen, once, before mods register.
// hidden: engine plumbing, re-exported through __private for the game.
#[doc(hidden)]
pub fn install_startup_screen(screen: Arc<dyn StartupScreen>) -> Result<(), StartupProgressError> {
    SCREEN
        .set(screen)
        .map_err(|_| StartupProgressError::AlreadyInstalled)
}

pub(crate) fn installed() -> Option<Arc<dyn StartupScreen>> {
    SCREEN.get().cloned()
}

/// A mod's bar on the startup loading screen (NeoForge's `ProgressMeter`), from
/// [`RegistrationContext::progress`](crate::RegistrationContext::progress). The mod may keep it
/// past registration and step it or post messages from any thread, including `client_setup`.
/// Changes are drawn only while the loading screen is being drawn. Synchronous `client_setup`
/// work is not displayed live; updates become visible only after the hook returns and only if
/// a loading frame is drawn afterwards. Slow work may use a mod-owned worker, but that alone
/// does not keep the loading screen open. Dropping it completes the bar and removes it. With no game screen (a test, a tool), every call
/// does nothing.
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: a mod-side handle around the game's screen, which crosses as its own resource.
pub struct StartupProgress {
    screen: Option<Arc<dyn StartupScreen>>,
    bar: u32,
}

impl StartupProgress {
    /// Show a startup message through this retained handle, including during `client_setup`.
    /// Like bar updates, it appears only on a subsequent loading frame; a dedicated server logs it.
    pub fn message(&self, text: &str) {
        if let Some(screen) = &self.screen {
            screen.message(text.to_owned());
        }
    }

    /// One more step is done.
    pub fn step(&self) {
        if let Some(screen) = &self.screen {
            screen.step(self.bar);
        }
    }

    /// `done` of the bar's steps are done; more than its steps reads as all of them.
    pub fn set(&self, done: u32) {
        if let Some(screen) = &self.screen {
            screen.set(self.bar, done);
        }
    }

    /// The bar reads `text` from now on, cut to one line.
    pub fn label(&self, text: &str) {
        if let Some(screen) = &self.screen {
            screen.label(self.bar, text.to_owned());
        }
    }
}

impl Drop for StartupProgress {
    fn drop(&mut self) {
        if let Some(screen) = &self.screen {
            screen.close(self.bar);
        }
    }
}

impl crate::RegistrationContext {
    /// Open a bar of `steps` steps labelled `label` on the startup loading screen, under the
    /// game's own stage bar, the newest bar first (NeoForge's `addProgressBar`). On a dedicated
    /// server, which has no screen, the label and every tenth of the steps are logged.
    /// Returns `TooManyBars` when 64 bars are already open; no bar is created.
    pub fn progress(
        &self,
        label: &str,
        steps: u32,
    ) -> Result<StartupProgress, StartupProgressError> {
        let bar = self
            .startup
            .as_ref()
            .map_or(Ok(0), |screen| screen.open(label.to_owned(), steps))?;
        Ok(StartupProgress {
            screen: self.startup.clone(),
            bar,
        })
    }

    /// Show `text` on the startup loading screen for a few seconds (NeoForge's
    /// `addModMessage`); a dedicated server logs it.
    pub fn message(&self, text: &str) {
        if let Some(screen) = &self.startup {
            screen.message(text.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Record(Mutex<Vec<String>>);
    impl Record {
        fn push(&self, call: String) {
            self.0.lock().unwrap().push(call);
        }
    }
    impl StartupScreen for Record {
        fn open(&self, label: String, steps: u32) -> Result<u32, StartupProgressError> {
            self.push(format!("open {label} {steps}"));
            Ok(7)
        }
        fn step(&self, bar: u32) {
            self.push(format!("step {bar}"));
        }
        fn set(&self, bar: u32, done: u32) {
            self.push(format!("set {bar} {done}"));
        }
        fn label(&self, bar: u32, label: String) {
            self.push(format!("label {bar} {label}"));
        }
        fn close(&self, bar: u32) {
            self.push(format!("close {bar}"));
        }
        fn message(&self, text: String) {
            self.push(format!("message {text}"));
        }
    }

    #[test]
    fn a_bar_opens_steps_and_closes_on_the_game_screen() {
        let screen = Arc::new(Record::default());
        let mut context = crate::RegistrationContext::new();
        context.startup = Some(screen.clone());
        let bar = context.progress("Baking", 10).unwrap();
        for _ in 0..4 {
            bar.step();
        }
        bar.label("Baked");
        bar.set(10);
        bar.message("done");
        // A worker thread may step the bar.
        std::thread::scope(|scope| {
            scope.spawn(|| bar.step());
        });
        drop(bar);
        assert_eq!(
            *screen.0.lock().unwrap(),
            [
                "open Baking 10",
                "step 7",
                "step 7",
                "step 7",
                "step 7",
                "label 7 Baked",
                "set 7 10",
                "message done",
                "step 7",
                "close 7",
            ]
        );
        // Without a game screen, nothing happens.
        let none = crate::RegistrationContext::new();
        none.progress("idle", 1).unwrap().step();
        none.message("unheard");
    }
}
