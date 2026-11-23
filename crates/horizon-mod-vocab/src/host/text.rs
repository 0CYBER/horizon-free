//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The methods of the `text` area on both sides, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait TextHost {
    /// Side: both.
    /// Reads a text component as `/tellraw` and `ComponentSerialization.CODEC` take it: one
    /// JSON or SNBT value (`"plain"`, `{"text":"Hi","color":"gold"}`, `["a",{"text":"b"}]`,
    /// `translate`, `extra`, click and hover events), with nothing but whitespace after it
    /// (Paper's `GsonComponentSerializer.deserialize`). None when it does not parse as one component:
    /// an object needs one content key (a `type` binds alone, a null `type` counts as absent),
    /// `text` must be a string, `extra` a non-empty list, and a style field that is present and
    /// wrong refuses the whole component. An object whose only content key is `index` is refused,
    /// as in vanilla's codec; Horizon follows vanilla there.
    #[access(Metadata)]
    fn text_from_json(&self, json: &str) -> Result<Option<Text>, TextFormError>;
    /// Side: both.
    /// Reads legacy formatting codes (`§a`, `§l`, `§r`...) as the font draws them
    /// (`Style.applyLegacyFormat`; Bukkit's `ChatColor` strings): a colour code resets the
    /// decorations, `§r` resets everything, an unknown code is dropped. `&` codes are not
    /// read; a mod that takes them replaces `&` with `§` first, as
    /// `ChatColor.translateAlternateColorCodes` does.
    #[access(Metadata)]
    fn text_from_legacy(&self, legacy: &str) -> Result<Text, TextFormError>;
    /// Side: both.
    /// The text as item components and block entities store it (`ComponentSerialization`
    /// to NBT): what `minecraft:custom_name` and each `minecraft:lore` line hold. See
    /// `ItemStack::set_display`.
    #[access(Metadata)]
    #[raw_text]
    fn text_to_nbt(&self, text: &Text) -> Result<Tag, TextFormError>;
    /// Side: both.
    /// The text an item component or a block entity stores (`ComponentSerialization` from
    /// NBT): what `text_to_nbt` writes, read back, so a `minecraft:custom_name` or a
    /// `minecraft:lore` line comes out as a `Text` with its colours and styles. See
    /// `ItemStack::display_name` and `ItemStack::lore`. A plain string is a literal text;
    /// anything that is not one component (an empty list, an object with no content key, a
    /// number) is `Ok(None)`; nesting past the game's limit is `TooDeep`.
    #[access(Metadata)]
    fn text_from_nbt(&self, tag: &Tag) -> Result<Option<Text>, TextFormError>;
}
