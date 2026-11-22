//! The window of a data move the launcher asked for (`--relocate-data <target> --restart`): the
//! step it is on and the files copied so far, until the launcher opens again (#19607).
//!
//! It reads nothing from the folders being moved: the built-in look, the bundled fonts and the
//! strings in the code. The move and the reopening run on a thread of their own that starts
//! before the window and never depends on it; GPUI keeps this thread, which AppKit needs, and
//! only shows what the move sends. Closing the window is refused for the whole move, so it never
//! runs on unseen; the window goes when the move has ended.
use crate::shell::{c, text};
use crate::theme::{self, night, Theme};
use gpui::{
    div, prelude::*, px, size, App, Bounds, Context, QuitMode, Render, Window, WindowBounds,
    WindowHandle, WindowOptions,
};
use horizon_launcher::backend::Appearance;
use horizon_launcher::i18n::{tr, trf};
use horizon_launcher::relocation::{Progress, Step, Worker};
use horizon_launcher::themes::Accent;
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

struct MoveWindow {
    progress: Rc<RefCell<Progress>>,
    target: Option<PathBuf>,
    theme: Theme,
}

/// Runs the move behind its window. The exit code, once the move and the reopening are done.
pub fn run(worker: Worker) -> i32 {
    let worker = Arc::new(worker);
    let target = worker.target().map(PathBuf::from);
    let (tx, rx) = channel();
    let code = Arc::new(OnceLock::new());
    let stop = Arc::new(AtomicBool::new(false));
    let mover = {
        let (worker, done, stop) = (worker.clone(), code.clone(), stop.clone());
        std::thread::Builder::new()
            .name("relocation".into())
            .spawn(move || {
                let code = worker.run_and_finish(
                    |update| {
                        let _ = tx.send(update);
                    },
                    &stop,
                );
                let _ = done.set(code);
                // `tx` goes with this closure: the window sees the move end.
                code
            })
    };
    let mover = match mover {
        Ok(mover) => mover,
        Err(error) => {
            let _ = writeln!(
                std::io::stderr(),
                "Could not start the move's thread: {error}"
            );
            // The launcher has quit already: the move, and its reopening, run here instead.
            return worker.run_and_finish(|_| {}, &AtomicBool::new(false));
        }
    };
    let shown = Rc::new(Cell::new(false));
    let seen = shown.clone();
    let lost = stop.clone();
    gpui_platform::application()
        .with_assets(crate::blocks::Assets)
        .run(move |cx: &mut App| {
            // Windows and Linux would otherwise quit with the last window, before the timer
            // below can open it again.
            cx.set_quit_mode(QuitMode::Explicit);
            crate::startup::add_fonts(cx);
            let progress = Rc::new(RefCell::new(Progress::default()));
            let mut window = open(&progress, &target, cx);
            // A window that never opened (no display) is not retried; one the platform took
            // away is, for as long as the move runs.
            seen.set(window.is_some());
            let shown = window.is_some();
            // Ticks in a row a lost window has failed to come back.
            let mut missing = 0;
            cx.spawn(async move |cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                loop {
                    match rx.try_recv() {
                        Ok(update) => progress.borrow_mut().apply(update),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            // The move and the reopening are done. `run` may not return on
                            // every platform once asked to quit, so the code goes from here.
                            std::process::exit(code.get().copied().unwrap_or(1));
                        }
                    }
                }
                let alive = window
                    .as_ref()
                    .is_some_and(|handle| handle.update(cx, |_, _, cx| cx.notify()).is_ok());
                if shown && !alive {
                    window = cx.update(|cx| open(&progress, &target, cx));
                    missing = if window.is_some() { 0 } else { missing + 1 };
                    // About 5 s without a window: the move stops at its next safe point rather
                    // than go on unseen.
                    if missing >= 50 {
                        lost.store(true, Ordering::Relaxed);
                    }
                }
            })
            .detach();
            cx.activate(true);
        });
    // The event loop ended with the move still running: the session ending (Windows), the
    // display server gone (Linux), the app told to quit. A move the player saw must not go on
    // unseen, so it stops before its next file and reports that to the reopened launcher. Only
    // one that never had a window finishes here.
    if shown.get() {
        stop.store(true, Ordering::Relaxed);
    }
    mover.join().unwrap_or(1)
}

fn open(
    progress: &Rc<RefCell<Progress>>,
    target: &Option<PathBuf>,
    cx: &mut App,
) -> Option<WindowHandle<MoveWindow>> {
    let bounds = Bounds::centered(None, size(px(520.), px(220.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            is_resizable: false,
            app_id: Some("horizon-launcher".into()),
            ..Default::default()
        },
        |window, cx| {
            window.on_window_should_close(cx, |_, _| false);
            let theme = Theme::for_appearance(
                Appearance::default(),
                Accent::default(),
                window.appearance(),
            );
            cx.new(|_| MoveWindow {
                progress: progress.clone(),
                target: target.clone(),
                theme,
            })
        },
    )
    .ok()
}

impl Render for MoveWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (t, n) = (self.theme, self.theme.night);
        let (step, files) = {
            let progress = self.progress.borrow();
            (progress.step.unwrap_or(Step::Checking), progress.files)
        };
        // No animated bar: the step and the count show that the move goes on.
        div()
            .size_full()
            .bg(t.page)
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(text(night::TITLE, c(n.ink)).child(tr("Moving Horizon's data").to_string()))
            .children(self.target.as_ref().map(|target| {
                text(theme::MICRO, c(n.muted))
                    .font_family(theme::MONO)
                    .truncate()
                    .child(target.display().to_string())
            }))
            .child(text(night::UI, c(n.ink)).child(tr(step.text()).to_string()))
            .child(
                text(night::CAPTION, c(n.ink_soft))
                    .font_family(theme::MONO)
                    .child(trf("{count} files copied", &[("count", &files.to_string())])),
            )
            .child(text(night::CAPTION, c(n.muted)).child(
                tr("Your data stays where it is until the copy is verified. The launcher opens again when the move ends.")
                    .to_string(),
            ))
    }
}
