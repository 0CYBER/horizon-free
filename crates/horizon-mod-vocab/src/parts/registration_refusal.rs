// exports: none (reached inside the crate only)
//! Self-refusal transport and the registration admission savepoint.

/// Engine admission plumbing; mods call `RegistrationContext::refuse` instead.
// hidden: the generated registrar call table transports self-refusals.
#[doc(hidden)]
pub trait RegistrationRefusalRegistrar {
    /// Deliver the current mod's refusal, without accepting an owner argument.
    fn registration_refusal(&mut self, reason: String);
    /// Deliver one of the current mod's loading warnings.
    fn registration_warning(&mut self, warning: LoadingMessage);
}

/// The most loading warnings one mod keeps; later ones are dropped.
// hidden: the host re-applies the cap to what a guest sends.
#[doc(hidden)]
pub const MAX_LOADING_WARNINGS: usize = 16;

/// A loading message as a translation key and its arguments, resolved against the
/// selected language when the game shows it (NeoForge's `ModLoadingIssue`).
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadingMessage {
    pub(crate) key: String,
    pub(crate) args: Vec<String>,
}

impl LoadingMessage {
    /// A message from `key` and its `%s` arguments, each cut to 4096 bytes as `refuse` cuts.
    pub fn new(key: &str, args: &[&str]) -> Self {
        Self {
            key: bounded_text(key).to_owned(),
            args: args
                .iter()
                .map(|arg| bounded_text(arg).to_owned())
                .collect(),
        }
    }
    /// The translation key; also the text shown when no language has it.
    pub fn key(&self) -> &str {
        &self.key
    }
    /// The arguments, in `%s` order.
    pub fn args(&self) -> &[String] {
        &self.args
    }
}

/// `text` cut to at most 4096 UTF-8 bytes, at a character boundary.
pub(crate) fn bounded_text(text: &str) -> &str {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Engine transport plumbing for the generated guest dispatch arm.
// hidden: drains the guest-local context's warnings after the registration callback.
#[doc(hidden)]
pub fn take_registration_warnings(cx: &crate::RegistrationContext) -> Vec<LoadingMessage> {
    std::mem::take(&mut *cx.warnings.lock().unwrap_or_else(|p| p.into_inner()))
}

/// The loading warnings mod `id` gave during registration, in order, from the installed Parts.
// hidden: read by the game's Mods screen.
#[doc(hidden)]
pub fn registration_warnings(id: &str) -> Vec<LoadingMessage> {
    super::installed().map_or_else(Vec::new, |installed| {
        installed
            .warnings
            .iter()
            .filter(|(owner, _)| owner == id)
            .map(|(_, warning)| warning.clone())
            .collect()
    })
}

/// Engine transport plumbing for the generated guest dispatch arm.
// hidden: drains the guest-local context after the registration callback.
#[doc(hidden)]
pub fn take_registration_refusal(cx: &crate::RegistrationContext) -> Option<String> {
    cx.self_refusal
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
}

#[cfg(test)]
mod tests {
    use crate::RegistrationContext;
    #[test]
    fn bounded_reasons_preserve_utf8_and_empty_is_a_refusal() {
        for (reason, expected) in [
            ("a".repeat(4095) + "é!", "a".repeat(4095)),
            ("é".repeat(2048), "é".repeat(2048)),
            (String::new(), String::new()),
        ] {
            let cx = RegistrationContext::new();
            cx.refuse(&reason);
            cx.refuse("not the first reason");
            assert_eq!(*cx.self_refusal.lock().unwrap(), Some(expected));
        }
    }

    #[test]
    fn warnings_keep_key_and_arguments_up_to_the_cap() {
        let cx = RegistrationContext::new();
        for i in 0..=super::MAX_LOADING_WARNINGS {
            cx.warn("test.warn.optional", &["jei", &i.to_string()]);
        }
        let warnings = super::take_registration_warnings(&cx);
        assert_eq!(warnings.len(), super::MAX_LOADING_WARNINGS);
        assert_eq!(warnings[0].key(), "test.warn.optional");
        assert_eq!(warnings[0].args(), ["jei", "0"]);
        assert_eq!(warnings[15].args(), ["jei", "15"]);
        assert_eq!(*cx.self_refusal.lock().unwrap(), None);
    }

    #[test]
    fn a_warning_cuts_key_and_arguments_at_a_character_boundary_and_keeps_every_argument() {
        let fits = "a".repeat(4096);
        let over = format!("{}é", "a".repeat(4095));
        let many: Vec<String> = (0..20).map(|i| i.to_string()).collect();
        let mut args: Vec<&str> = vec![&fits, &over];
        args.extend(many.iter().map(String::as_str));
        let warning = super::LoadingMessage::new(&format!("{fits}b"), &args);
        assert_eq!(warning.key(), fits);
        assert_eq!(warning.args()[0], fits);
        assert_eq!(warning.args()[1], "a".repeat(4095));
        assert_eq!(warning.args().len(), 22);
        assert_eq!(warning.args()[21], "19");
    }
}
