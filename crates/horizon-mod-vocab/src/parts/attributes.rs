use super::*;
use crate::{AttributeTooltip, Sentiment};

/// An attribute type a mod declares, as the game reads it when it grows the
/// attribute catalog (`Catalog::with_attributes`, ).
#[derive(Clone, Debug)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct AttributeDeclaration {
    /// The declaring mod's id: the first key of the catalog's sort order.
    pub mod_id: String,
    /// The attribute's namespaced id (`mymod:mana`).
    pub id: String,
    /// The attribute's default value, inside `[min, max]`.
    pub default: f64,
    /// The attribute's lower bound.
    pub min: f64,
    /// The attribute's upper bound.
    pub max: f64,
    /// Whether the attribute joins the sync set.
    pub syncable: bool,
    /// The tooltip sentiment of a rising or falling value.
    pub sentiment: Sentiment,
    /// How its tooltip lines read: display kind and base modifier.
    pub tooltip: AttributeTooltip,
}

/// The attribute types mods declared.
pub fn attribute_declarations() -> &'static [AttributeDeclaration] {
    installed().map_or(&[], |installed| &installed.attributes)
}

/// A mod's ask to widen an existing attribute's bounds, as the game reads it
/// when it grows the attribute catalog (arismoko/horizon#3728).
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct AttributeBoundsWidening {
    /// The asking mod's id, which a refusal names.
    pub mod_id: String,
    /// The attribute's namespaced id (`minecraft:max_health`).
    pub id: String,
    /// The lower bound the mod asks for.
    pub min: f64,
    /// The upper bound the mod asks for.
    pub max: f64,
}

/// The attribute bounds mods asked to widen.
pub fn attribute_bounds_widenings() -> &'static [AttributeBoundsWidening] {
    installed().map_or(&[], |installed| &installed.attribute_widenings)
}

/// Registration of new attribute types ([`Registrar::declare_attribute`](AttributesRegistrar::declare_attribute))
/// and wider bounds for existing ones ([`Registrar::widen_attribute_bounds`](AttributesRegistrar::widen_attribute_bounds)).
pub trait AttributesRegistrar {
    /// Declare an attribute type of the mod's own (vanilla's
    /// `RangedAttribute` in `BuiltInRegistries.ATTRIBUTE`, NeoForge's
    /// `RegisterEvent`): `path` in the mod's namespace (`mana` of `mymod` is
    /// `mymod:mana`), its `default` and `[min, max]` bounds, whether it
    /// joins the attribute sync (`Attribute.setSyncable`), and the sentiment
    /// that colors its tooltip lines (`Attribute.setSentiment`, POSITIVE by
    /// default; the translation key is `attribute.name.<ns>.<path>`), and how its
    /// tooltip lines read (`tooltip`: percentages, on/off, a base modifier; NeoForge's
    /// `PercentageAttribute`, `BooleanAttribute` and `IAttributeExtension.getBaseId`).
    ///
    /// Only both-side packages may declare attributes; other sides refuse because
    /// declarations assign network registry IDs.
    /// The declarations join the attribute catalog once, at load, appended
    /// after every vanilla attribute **sorted by declaring mod id, then by
    /// attribute id** — the order is the network id's, so every side running
    /// the same mods builds the same numbers and vanilla ids never shift —
    /// before entity attribute rows resolve, so a mod entity's declared set
    /// may name the mod's own new attribute. Sanitizing a value (NaN → min,
    /// else clamp) is the definition's own `sanitize`.
    ///
    /// An id outside the mod's manifest-owned namespaces (a vanilla attribute's included),
    /// a malformed id, a duplicate — of this mod's or another's, caught
    /// where the declarations join the catalog — a non-finite default or
    /// bound, bounds the default is not inside (`min > max` included), or a
    /// base modifier that is not a namespaced id refuses, naming the mod.
    fn declare_attribute(
        &mut self,
        path: &str,
        default: f64,
        min: f64,
        max: f64,
        syncable: bool,
        sentiment: Sentiment,
        tooltip: AttributeTooltip,
    ) -> Result<(), crate::AttributeRegistrationError>;

    /// Widen the `[min, max]` bounds of an attribute that already exists,
    /// vanilla's or another mod's (`id` in full: `minecraft:max_health`), as
    /// Java mods do with a mixin or an access transformer on
    /// `RangedAttribute.minValue` / `maxValue`: every clamp of the attribute's
    /// value, on both sides, then follows the wider bounds.
    ///
    /// The asks of every mod join the catalog once, at load, after the
    /// declared attributes and before entity attribute rows resolve. The
    /// result is their union, whatever the load order: the lowest `min` and
    /// the highest `max` asked, and never narrower than the attribute's own
    /// bounds, so an ask narrower than what another mod widened to changes
    /// nothing. The default value never moves. An attribute without bounds
    /// stays without them.
    ///
    /// Only both-side packages may widen bounds, so the client and the server
    /// build the same catalog; the mod-set check at join is all a client
    /// needs. A non-finite bound or `min > max` refuses here, naming the mod;
    /// an attribute the catalog does not have, or one whose default is not
    /// inside `[min, max]`, refuses the load where the asks join the catalog,
    /// naming the mod.
    fn widen_attribute_bounds(
        &mut self,
        id: &str,
        min: f64,
        max: f64,
    ) -> Result<(), crate::AttributeRegistrationError>;
}
