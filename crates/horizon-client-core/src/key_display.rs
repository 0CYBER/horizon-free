//! Key display names (`InputConstants.Key.getDisplayName`), shared by key
//! mappings and `keybind` text components.
use crate::text::Text;

/// InputConstants.Key.getDisplayName: one-character keys as the upper-case
/// character, others by translation.
pub fn display_name(key: &str) -> Text {
    if let Some(rest) = key.strip_prefix("key.keyboard.")
        && rest.chars().count() == 1
    {
        let c = rest.to_uppercase();
        return Text::literal(c);
    }
    let symbol = match key {
        "key.keyboard.minus" => Some("-"),
        "key.keyboard.equal" => Some("="),
        "key.keyboard.left.bracket" => Some("["),
        "key.keyboard.right.bracket" => Some("]"),
        "key.keyboard.backslash" => Some("\\"),
        "key.keyboard.semicolon" => Some(";"),
        "key.keyboard.apostrophe" => Some("'"),
        "key.keyboard.grave.accent" => Some("`"),
        "key.keyboard.comma" => Some(","),
        "key.keyboard.period" => Some("."),
        "key.keyboard.slash" => Some("/"),
        _ => None,
    };
    if let Some(s) = symbol {
        return Text::literal(s);
    }
    if let Some(n) = key.strip_prefix("key.mouse.")
        && n.parse::<u32>().is_ok()
        && !crate::lang::has(key)
    {
        return Text::translate("key.mouse", &[Text::literal(n)]);
    }
    Text::translate(key, &[])
}
