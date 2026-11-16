// exports: none (reached inside the crate only)
/// The package inventory carries the same namespace set as content files and
/// the package loader's collision fence. A package always owns its own ID.
pub fn owns_namespace(owner: &str, namespace: &str) -> bool {
    owner == namespace
        || crate::LoadedMods::new()
            .get(owner)
            .is_some_and(|package| package.namespaces().any(|owned| owned == namespace))
}

/// `name` with its namespace: a bare name is in mod `owner`'s own, for a declared id and
/// for the block and item references a declaration names (`block_item`, `pick_item`,
/// `wall_block`, `base_block`, `crafting_remainder`, `Item::block`) alike.
pub fn qualified(owner: &str, name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("{owner}:{name}")
    }
}

/// Validate and qualify a declaration using the package namespace inventory.
pub fn owned_declaration_id(owner: &str, name: &str) -> Result<String, String> {
    let id = qualified(owner, name);
    let parsed = crate::Identifier::parse_namespaced(&id)
        .ok_or_else(|| format!("mod {owner} declares invalid identifier {id:?}"))?;
    let namespace = parsed.namespace();
    if !owns_namespace(owner, namespace) {
        return Err(format!(
            "mod {owner} declares {id:?} in unowned namespace {namespace:?}"
        ));
    }
    Ok(id)
}

/// A declaration a mod's `blocks` or `items` hook adds: its id, and the names of other
/// content it references.
pub(crate) trait Declaration {
    /// The declared id and its optional references.
    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>);
}

impl Declaration for crate::Block {
    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>) {
        (
            &mut self.id,
            [
                &mut self.block_item,
                &mut self.pick_item,
                &mut self.wall_block,
                &mut self.base_block,
            ],
        )
    }
}

impl Declaration for crate::Item {
    fn names(&mut self) -> (&mut String, impl IntoIterator<Item = &mut Option<String>>) {
        (
            &mut self.id,
            [&mut self.crafting_remainder, &mut self.block],
        )
    }
}

/// Make `declaration` mod `owner`'s: each bare reference names its own content, and
/// the id must be in a namespace it owns. On an error the id is left as given.
pub(crate) fn own_declaration(
    owner: &str,
    declaration: &mut impl Declaration,
) -> Result<(), String> {
    let (id, references) = declaration.names();
    for reference in references.into_iter().flatten() {
        *reference = qualified(owner, reference);
    }
    *id = owned_declaration_id(owner, id)?;
    Ok(())
}

impl crate::RegistrationContext {
    /// Refuse this mod's registration with a diagnostic, without panicking.
    /// The first reason wins. Reasons exceeding 4096 UTF-8 bytes are truncated
    /// at a character boundary; shorter reasons are preserved verbatim.
    /// All this mod's declarations from this registration are discarded, including
    /// later declarations, without adding their validation errors to the report.
    /// Other mods' declarations are unchanged. Native and wasm use the same
    /// admission path. Returning from `Hooks::register` immediately is recommended.
    pub fn refuse(&self, reason: &str) {
        let mut reason_slot = self.self_refusal.lock().unwrap_or_else(|p| p.into_inner());
        if reason_slot.is_none() {
            *reason_slot = Some(super::registration_refusal::bounded_text(reason).to_owned());
        }
    }

    /// Report a loading problem that doesn't stop this mod: a missing optional integration,
    /// a setting it corrected. `key` is a translation key in the mod's own `lang/` files and
    /// `args` its `%s` arguments; the Mods screen shows it in the player's language, and the
    /// log names the key. The first 16 warnings are kept, each cut as `refuse` cuts.
    pub fn warn(&self, key: &str, args: &[&str]) {
        let mut warnings = self.warnings.lock().unwrap_or_else(|p| p.into_inner());
        if warnings.len() < super::registration_refusal::MAX_LOADING_WARNINGS {
            warnings.push(super::registration_refusal::LoadingMessage::new(key, args));
        }
    }

    pub(crate) fn refuse_declaration(&self, reason: String) {
        self.declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(reason);
    }
}

/// The engine consumes block/item declaration ownership refusals before loading
/// resources. Guest dispatch retains invalid output so the owning host boundary
/// sees and refuses it as well, rather than silently dropping a declaration.
// hidden: engine admission plumbing, re-exported through __private for the loader.
#[doc(hidden)]
pub fn take_declaration_refusals(context: &crate::RegistrationContext) -> Vec<String> {
    std::mem::take(
        &mut *context
            .declaration_refusals
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    )
}
