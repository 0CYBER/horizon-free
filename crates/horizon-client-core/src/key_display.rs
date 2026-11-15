//! Key display names (`InputConstants.Key.getDisplayName`), shared by key
//! mappings and `keybind` text components (`KeybindContents.getNestedComponent`).
use crate::text::Text;
use std::{collections::BTreeMap, sync::RwLock};

/// The keys that write a symbol, with what they write on a US layout.
const SYMBOLS: [(&str, &str); 11] = [
    ("key.keyboard.minus", "-"),
    ("key.keyboard.equal", "="),
    ("key.keyboard.left.bracket", "["),
    ("key.keyboard.right.bracket", "]"),
    ("key.keyboard.backslash", "\\"),
    ("key.keyboard.semicolon", ";"),
    ("key.keyboard.apostrophe", "'"),
    ("key.keyboard.grave.accent", "`"),
    ("key.keyboard.comma", ","),
    ("key.keyboard.period", "."),
    ("key.keyboard.slash", "/"),
];

/// What the player's keyboard layout writes on each key that writes
/// something, by key name, as the window reported it (`learn_layout`).
static LAYOUT: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// Current action bindings (`KeyMapping` assigned keys): action name (`key.jump`)
/// -> bound key name (`key.keyboard.space`, or `key.keyboard.unknown` if unbound).
static ACTIONS: RwLock<BTreeMap<String, String>> = RwLock::new(BTreeMap::new());

/// Optional dynamic action resolver callback (`KeybindResolver`).
type ActionResolver = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
static RESOLVER: RwLock<Option<ActionResolver>> = RwLock::new(None);

/// Serializes tests that read or change the process-wide binding state.
#[cfg(test)]
pub(crate) static TEST_BINDINGS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Register or update an action key binding (`KeyMapping` assigned key).
pub fn set_action_binding(action: &str, bound: &str) {
    if let Ok(mut actions) = ACTIONS.write() {
        actions.insert(action.to_owned(), bound.to_owned());
    }
}

/// Replace every action binding at once (the active `KeyMapping` set), under one
/// lock so a reader never sees a half-written table.
pub fn replace_action_bindings<I, K, V>(bindings: I)
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    if let Ok(mut actions) = ACTIONS.write() {
        actions.clear();
        for (a, b) in bindings {
            actions.insert(a.as_ref().to_owned(), b.as_ref().to_owned());
        }
    }
}

/// Install a custom action resolver closure (`KeybindResolver.setKeyResolver`).
pub fn set_action_resolver<F>(resolver: F)
where
    F: Fn(&str) -> Option<String> + Send + Sync + 'static,
{
    if let Ok(mut r) = RESOLVER.write() {
        *r = Some(Box::new(resolver));
    }
}

/// Clear any custom action resolver.
pub fn clear_action_resolver() {
    if let Ok(mut r) = RESOLVER.write() {
        *r = None;
    }
}

/// Clear registered action bindings.
pub fn clear_action_bindings() {
    if let Ok(mut actions) = ACTIONS.write() {
        actions.clear();
    }
}

/// Reset action bindings and custom resolver.
pub fn reset_action_bindings() {
    clear_action_bindings();
    clear_action_resolver();
}

/// Resolve an action name to its bound key, if known.
fn resolve_action(key: &str) -> Option<String> {
    if let Ok(resolver) = RESOLVER.read()
        && let Some(ref resolve) = *resolver
        && let Some(bound) = resolve(key)
    {
        return Some(bound);
    }
    if let Ok(actions) = ACTIONS.read()
        && let Some(bound) = actions.get(key)
    {
        return Some(bound.clone());
    }
    None
}

/// The keyboard layout writes `text` on the key `key` (`key.keyboard.q`):
/// its name from now on, as `SDL_GetKeyFromScancode` names it with SDL's
/// default keycode options. Only letter and symbol keys take it; a letter
/// key keeps its Latin letter on a non-Latin layout (`latin_letters`), and
/// the number row keeps its digits (`french_numbers`).
pub fn learn_layout(key: &str, text: &str) {
    let Some(rest) = key.strip_prefix("key.keyboard.") else {
        return;
    };
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return;
    };
    let letter_key = rest.len() == 1 && rest.as_bytes()[0].is_ascii_lowercase();
    let latin = c.is_ascii() || ('\u{C0}'..='\u{24F}').contains(&c);
    if c.is_control()
        || c.is_whitespace()
        || !(letter_key || SYMBOLS.iter().any(|(k, _)| *k == key))
        || (letter_key && c.is_alphabetic() && !latin)
    {
        return;
    }
    if let Ok(mut layout) = LAYOUT.write() {
        layout.insert(key.to_owned(), c.to_uppercase().collect());
    }
}

/// InputConstants.Key.getDisplayName and KeybindContents.getNestedComponent:
/// resolves action keybinds (`key.jump`) through current client bindings,
/// then formats physical keys using keyboard layout, symbols, or translations.
pub fn display_name(key: &str) -> Text {
    let resolved = resolve_action(key);
    let target = resolved.as_deref().unwrap_or(key);

    if let Some(label) = LAYOUT.read().ok().and_then(|l| l.get(target).cloned()) {
        return Text::literal(label);
    }
    if let Some(rest) = target.strip_prefix("key.keyboard.")
        && rest.chars().count() == 1
    {
        let c = rest.to_uppercase();
        return Text::literal(c);
    }
    if let Some((_, s)) = SYMBOLS.iter().find(|(k, _)| *k == target) {
        return Text::literal(*s);
    }
    if let Some(n) = target.strip_prefix("key.mouse.")
        && n.parse::<u32>().is_ok()
        && !crate::lang::has(target)
    {
        return Text::translate("key.mouse", &[Text::literal(n)]);
    }
    Text::translate(target, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_show_what_the_layout_writes() {
        let _guard = TEST_BINDINGS.lock().unwrap_or_else(|e| e.into_inner());
        let prior_layout = std::mem::take(&mut *LAYOUT.write().unwrap());
        assert_eq!(display_name("key.keyboard.q").plain(), "Q");
        // AZERTY: A on the Q key, M on the semicolon key, a dead ^ on the
        // left bracket, a comma on the M key.
        learn_layout("key.keyboard.q", "a");
        learn_layout("key.keyboard.semicolon", "m");
        learn_layout("key.keyboard.left.bracket", "^");
        learn_layout("key.keyboard.m", ",");
        assert_eq!(display_name("key.keyboard.q").plain(), "A");
        assert_eq!(display_name("key.keyboard.semicolon").plain(), "M");
        assert_eq!(display_name("key.keyboard.left.bracket").plain(), "^");
        assert_eq!(display_name("key.keyboard.m").plain(), ",");
        // The number row keeps its digits; a Cyrillic letter, nothing
        // printable and keys that write nothing are left alone.
        learn_layout("key.keyboard.1", "&");
        learn_layout("key.keyboard.f", "а");
        learn_layout("key.keyboard.g", "\u{8}");
        learn_layout("key.keyboard.tab", "\t");
        learn_layout("key.keyboard.h", "ab");
        assert_eq!(display_name("key.keyboard.1").plain(), "1");
        assert_eq!(display_name("key.keyboard.f").plain(), "F");
        assert_eq!(display_name("key.keyboard.g").plain(), "G");
        assert_eq!(display_name("key.keyboard.h").plain(), "H");
        assert_eq!(display_name("key.keyboard.slash").plain(), "/");
        *LAYOUT.write().unwrap() = prior_layout;
    }

    #[test]
    fn action_keybinds_resolve_through_client_bindings() {
        let _guard = TEST_BINDINGS.lock().unwrap_or_else(|e| e.into_inner());
        let prior_layout = std::mem::take(&mut *LAYOUT.write().unwrap());
        replace_action_bindings([
            ("key.jump", "key.keyboard.space"),
            ("key.forward", "key.keyboard.w"),
            ("key.back", "key.keyboard.s"),
            ("key.right", "key.keyboard.d"),
        ]);
        let ((), _) = crate::lang::scoped(
            crate::lang::Language::of(
                "en_us",
                [
                    ("key.jump".into(), "Jump".into()),
                    ("key.keyboard.space".into(), "Space".into()),
                    ("key.keyboard.up".into(), "Up Arrow".into()),
                    ("key.keyboard.unknown".into(), "None".into()),
                ],
            ),
            || {
                // 1. A bound action shows its assigned key (Space, not Jump)
                assert_eq!(display_name("key.jump").plain(), "Space");
                assert_eq!(display_name("key.forward").plain(), "W");
                assert_eq!(display_name("key.back").plain(), "S");
                assert_eq!(display_name("key.right").plain(), "D");

                // 2. Action rebinding
                set_action_binding("key.jump", "key.keyboard.up");
                assert_eq!(display_name("key.jump").plain(), "Up Arrow");

                // 3. Unbound display (key.keyboard.unknown -> "None")
                set_action_binding("key.jump", "key.keyboard.unknown");
                assert_eq!(display_name("key.jump").plain(), "None");

                // 4. Keyboard layout changes (AZERTY layout maps key.keyboard.q to A)
                learn_layout("key.keyboard.q", "a");
                set_action_binding("key.drop", "key.keyboard.q");
                assert_eq!(display_name("key.drop").plain(), "A");

                // 5. An action with no binding falls back to translating its ID
                assert_eq!(
                    display_name("key.unregistered_custom_action").plain(),
                    "key.unregistered_custom_action"
                );

                // 6. Custom resolver
                set_action_resolver(|action| {
                    if action == "key.custom_action" {
                        Some("key.keyboard.h".to_owned())
                    } else {
                        None
                    }
                });
                assert_eq!(display_name("key.custom_action").plain(), "H");
                clear_action_resolver();

                // 7. French translations
                let ((), _) = crate::lang::scoped(
                    crate::lang::Language::of(
                        "fr_fr",
                        [
                            ("key.jump".into(), "Sauter".into()),
                            ("key.keyboard.space".into(), "Espace".into()),
                            ("key.keyboard.unknown".into(), "Aucun".into()),
                        ],
                    ),
                    || {
                        set_action_binding("key.jump", "key.keyboard.space");
                        assert_eq!(display_name("key.jump").plain(), "Espace");
                        set_action_binding("key.jump", "key.keyboard.unknown");
                        assert_eq!(display_name("key.jump").plain(), "Aucun");
                    },
                );

                // 8. With no bindings, the action translates (vanilla's default resolver)
                reset_action_bindings();
                assert_eq!(display_name("key.jump").plain(), "Jump");
            },
        );
        *LAYOUT.write().unwrap() = prior_layout;
    }
}
