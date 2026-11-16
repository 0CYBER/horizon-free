//! The worker pauses before Cargo can execute dependency build code.
use gpui::{AnyWindowHandle, AppContext, Context, PromptLevel};
use horizon_launcher::{i18n::tr, native_mods::RegistryInventory};
use std::sync::mpsc::Sender;

pub fn show(
    window: AnyWindowHandle,
    inventory: RegistryInventory,
    reply: Sender<bool>,
    cx: &mut Context<crate::Launcher>,
) {
    match cx.update_window(window, |_, window, cx| {
        window.prompt(
            PromptLevel::Warning,
            &inventory.label(),
            Some(&inventory.detail()),
            &[tr("Cancel"), tr("Allow native mods")],
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
