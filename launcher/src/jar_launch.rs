//! The GUI bridge: preparation off the UI thread, one consent for the complete missing set.
use gpui::{AnyWindowHandle, AppContext, Context, PromptLevel};
use horizon_launcher::{
    backend::Msg,
    i18n::{tr, trf},
    jars::{self, Permission},
    launch::{self, Launch},
};
use horizon_mod_package::manifest::JarRef;
use std::{
    process::Command,
    sync::mpsc::{self, Sender},
};

pub fn client(mut command: Command, launch: Launch, run: Option<String>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let prepared = with_consent(&tx, |permission| {
            jars::prepare_launch(&mut command, &launch, permission)
        })
        .map(|()| launch::wrap(command, &launch))
        .map_err(|error| error.to_string());
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
