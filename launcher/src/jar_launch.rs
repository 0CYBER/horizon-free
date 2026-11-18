//! The GUI bridge: preparation off the UI thread, one consent for the complete missing set.
use gpui::{AnyWindowHandle, AppContext, Context, PromptLevel};
use horizon_launcher::{
    backend::Msg,
    i18n::{tr, trf},
    jars::{self, Permission},
    launch::{self, Launch},
};
use horizon_platform::mod_package::manifest::JarRef;
use std::{
    process::Command,
    sync::mpsc::{self, Sender},
};

/// `profile` turns the command into the instance's first, on the thread too: a modpack's carried
/// mods are read whole for it, tens of thousands of files in a large pack (#18074).
pub fn client(
    mut command: Command,
    profile: impl FnOnce(&mut Command) -> Result<(), String> + Send + 'static,
    launch: Launch,
    run: Option<String>,
    tx: Sender<Msg>,
) {
    std::thread::spawn(move || {
        let prepared = profile(&mut command).and_then(|()| {
            with_consent(&tx, |permission| {
                jars::prepare_launch(&mut command, &launch, permission)
            })
            .map(|()| launch::wrap(command, &launch))
            .map_err(|error| error.to_string())
        });
        let _ = tx.send(Msg::JarClient {
            run,
            prepared: Box::new(prepared),
        });
    });
}

/// Used only from background jobs: the UI owns the one complete-set prompt.
pub fn with_consent<T>(tx: &Sender<Msg>, prepare: impl FnOnce(Permission<'_>) -> T) -> T {
    let ask = |missing: &[&JarRef]| {
        let (reply, answer) = mpsc::channel();
        let jars = missing.iter().map(|jar| (*jar).clone()).collect();
        tx.send(Msg::JarConsent { jars, reply }).is_ok() && answer.recv().unwrap_or(false)
    };
    prepare(Permission::Ask(&ask))
}

pub fn consent(
    window: AnyWindowHandle,
    jars: Vec<JarRef>,
    reply: Sender<bool>,
    cx: &mut Context<crate::Launcher>,
) {
    let message = trf(
        "Download {count} original Java jars from Modrinth?",
        &[("count", &jars.len().to_string())],
    );
    let mut detail = tr("Only assets and data are used; Java code is never run. Each file is checked against the manifest's SHA-512.").to_owned();
    for jar in jars {
        detail.push_str("\n\n");
        detail.push_str(&trf(
            "{id} {version}\nSHA-512: {hash}",
            &[
                ("id", jar.id()),
                ("version", jar.version()),
                ("hash", jar.sha512()),
            ],
        ));
    }
    match cx.update_window(window, |_, window, cx| {
        window.prompt(
            PromptLevel::Warning,
            &message,
            Some(&detail),
            &[tr("Cancel"), tr("Download jars")],
            cx,
        )
    }) {
        Ok(answer) => cx
            .spawn(async move |_, _| {
                let _ = reply.send(matches!(answer.await, Ok(1)));
            })
            .detach(),
        Err(_) => {
            let _ = reply.send(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Play turns the command into the instance's on the launch's thread, never the window's: a
    /// modpack's carried mods are read whole there (#18074). Its error ends the start like any
    /// other, as the message the window shows.
    #[test]
    fn the_profile_is_applied_off_the_calling_thread() {
        let (tx, rx) = mpsc::channel();
        let window = std::thread::current().id();
        let profile = move |_: &mut Command| -> Result<(), String> {
            match std::thread::current().id() == window {
                true => Err("on the window's thread".into()),
                false => Err("port the pack first".into()),
            }
        };
        let run = Some("port:big".to_owned());
        client(Command::new("game"), profile, Launch::default(), run, tx);
        let message = rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap();
        let Msg::JarClient { run, prepared } = message else {
            panic!("not the start's end");
        };
        assert_eq!(run.as_deref(), Some("port:big"));
        assert_eq!((*prepared).unwrap_err(), "port the pack first");
    }
}
