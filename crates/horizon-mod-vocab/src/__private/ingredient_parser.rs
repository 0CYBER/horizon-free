use crate::item::crafting::{IngredientRule, Shape};
use serde_json::Value;
use std::collections::HashSet;
/// Where `Ingredient::resolve` leaves the hook's answer inside the occurrence
/// it resolved. Written and read only here; the loads that own the sources
/// never send or save it.
const RESOLVED_ITEMS: &str = "horizon:resolved_items";

/// The kind an ingredient object names: `neoforge:ingredient_type` in
/// current NeoForge, `type` in older packs.
fn ingredient_kind(o: &serde_json::Map<String, Value>) -> Option<&str> {
    o.get("neoforge:ingredient_type")
        .or_else(|| o.get("type"))?
        .as_str()
}

/// A mod's own kind: a namespaced type that is not minecraft's.
fn is_custom(kind: &str) -> bool {
    kind.contains(':') && !kind.starts_with("minecraft:")
}

/// Takes the answer `resolve` left out of `data`.
// hidden: the game-owned ingredient adapter reads the answer a mod's `resolve` left, not a mod-facing call.
#[doc(hidden)]
pub fn resolved_items(data: &mut horizon_core::nbt::Compound) -> Option<Vec<String>> {
    let answer = data.remove(&RESOLVED_ITEMS.into())?;
    let horizon_core::nbt::Tag::List(answer) = answer else {
        return None;
    };
    answer
        .iter()
        .map(|i| match i {
            horizon_core::nbt::Tag::String(s) => s.to_string().ok(),
            _ => None,
        })
        .collect()
}
/// Keep first-seen order, drop the rest.
// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub fn dedup_keep_order(items: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.clone()));
}
// no wire: private parser working state includes engine NBT and unordered lookup sets; not an API transport value.
/// The walk both ingredient kinds share: a string (an id or `#tag`, as
/// `tags` resolves it), a list of them, or NeoForge's built-in kinds with
/// their children walked the same way, each answered by the flat list of
/// ids it matches in the order the sources named them. A kind outside the
/// shapes is answered by the wrapper's `custom` as the walk meets it, so a
/// shape sees its ids like any child's: the item ingredient gives the list
/// its hook left at load, a fluid one none, which fails the file. Existing
/// empty tags and custom answers retain their predicate but cannot be placed.
#[derive(Clone, Debug)]
// hidden: shared parser output for the game-owned ingredient adapter, not a mod-facing recipe.
#[doc(hidden)]
#[non_exhaustive]
pub struct WalkedIngredient {
    /// The ids it accepts, however the source shape named them.
    pub set: HashSet<String>,
    /// The same ids in the sources' order (`HolderSet`).
    pub order: Vec<String>,
    /// Compound, intersection, difference or custom display children.
    pub children: Option<(Shape, Vec<Self>)>,
    /// A kind outside the standard shapes: the kind and its NBT compound, as the
    /// file wrote it, without the wrapper's answer.
    pub custom: Option<(String, horizon_core::nbt::Compound)>,
    /// Component restrictions, independent of the enumerable item list.
    pub rule: Option<IngredientRule>,
    /// Slot display override, independent of matching.
    pub display: Option<horizon_core::nbt::Compound>,
}
// no wire: borrowed parser or registration callbacks execute only in their owning process.
/// The wrapper's answer for a kind outside the standard shapes: the ids it
/// matches, taken out of its metadata, or none to fail the file.
// hidden: the game-owned ingredient adapter supplies custom ids while walking JSON.
#[doc(hidden)]
pub type CustomIds<'a> = &'a dyn Fn(&mut horizon_core::nbt::Compound) -> Option<Vec<String>>;
// no wire: borrowed parser or registration callbacks execute only in their owning process.
/// Game-owned JSON-to-NBT boundary for the hidden recipe parser.
// hidden: supplied by the game's ingredient adapter to the parser, not a mod-facing type.
#[doc(hidden)]
pub type IngredientData<'a> =
    &'a dyn Fn(&serde_json::Map<String, Value>) -> Option<horizon_core::nbt::Compound>;
impl WalkedIngredient {
    /// Only component-independent ingredients can use item-list matching.
    pub fn is_simple(&self) -> bool {
        self.rule.is_none()
            && self.custom.is_none()
            && self
                .children
                .as_ref()
                .is_none_or(|(_, kids)| kids.iter().all(Self::is_simple))
    }

    /// The walk of `v`: the shapes' flat lists in the sources' order, their
    /// children kept for matching a concrete stack.
    pub fn walk(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_>,
        convert: IngredientData<'_>,
    ) -> Option<Self> {
        Self::walk_resolved(v, tags, block_tags, custom, convert, &|id| {
            Some(id.to_owned())
        })
    }

    // hidden: the game-owned adapter resolves ids using its own load-time registry.
    #[doc(hidden)]
    pub fn walk_resolved(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_>,
        convert: IngredientData<'_>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        let mut list: Vec<String> = Vec::new();
        let mut valid = true;
        let mut named = false;
        let mut add = |s: &str| {
            named = true;
            let more: Vec<String> = match s.strip_prefix('#') {
                Some(tag) => match tags.get(tag) {
                    Some(items) => items.clone(),
                    None => {
                        valid = false;
                        return;
                    }
                },
                None => vec![s.to_owned()],
            };
            for i in more {
                match resolve(&i) {
                    Some(i) if !list.contains(&i) => list.push(i),
                    Some(_) => {}
                    None => valid = false,
                }
            }
        };
        match v {
            Value::String(s) => add(s),
            Value::Array(a) => a.iter().filter_map(Value::as_str).for_each(&mut add),
            Value::Object(o) => {
                return Self::walk_object(o, tags, block_tags, custom, convert, resolve);
            }
            _ => return None,
        }
        (valid && named).then(|| Self::flat(list))
    }
    /// A resolved plain ingredient, including an existing empty tag.
    fn flat(list: Vec<String>) -> Self {
        Self {
            set: list.iter().cloned().collect(),
            order: list,
            children: None,
            custom: None,
            rule: None,
            display: None,
        }
    }
    fn allows_empty(&self) -> bool {
        self.custom.is_some()
            || (self.order.is_empty() && self.children.is_none())
            || self
                .children
                .as_ref()
                .is_some_and(|(_, children)| children.iter().any(Self::allows_empty))
    }

    /// NeoForge's ingredient types (`NeoForgeRegistries.Keys.INGREDIENT_TYPES`),
    /// the standard shapes mods' recipes carry, each answered by the flat id
    /// list it matches, with the children kept for matching a concrete
    /// stack. A kind outside these is the wrapper's `custom`; one of
    /// `minecraft:`'s the wrapper does not know still fails the file.
    fn walk_object(
        o: &serde_json::Map<String, Value>,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_>,
        convert: IngredientData<'_>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        // Current NeoForge marks a custom ingredient with
        // `neoforge:ingredient_type`; `type` is the older packs' key.
        let kind = ingredient_kind(o)?;
        let children = || {
            o.get("children")
                .or_else(|| o.get("ingredients"))
                .and_then(Value::as_array)
        };
        // The shapes' walked children, for matching at craft time: every
        // arm either fills it or returns.
        let nested;
        let kept = match kind {
            // `CompoundIngredient`: any of its children matching the stack
            // is enough — the union of their ids.
            "neoforge:compound" => {
                let subs: Vec<Self> = children()?
                    .iter()
                    .map(|s| Self::walk_resolved(s, tags, block_tags, custom, convert, resolve))
                    .collect::<Option<_>>()?;
                let mut seen = Vec::new();
                for sub in &subs {
                    for id in &sub.order {
                        if !seen.contains(id) {
                            seen.push(id.clone());
                        }
                    }
                }
                nested = Some((Shape::Compound, subs));
                seen
            }
            // `IntersectionIngredient`: every child must match the same
            // stack — the ids all of them name.
            "neoforge:intersection" => {
                let subs: Vec<Self> = children()?
                    .iter()
                    .map(|s| Self::walk_resolved(s, tags, block_tags, custom, convert, resolve))
                    .collect::<Option<_>>()?;
                nested = Some((Shape::Intersection, subs.clone()));
                subs.first()?
                    .order
                    .iter()
                    .filter(|i| subs[1..].iter().all(|s| s.set.contains(*i)))
                    .cloned()
                    .collect()
            }
            // The base's ids without the subtracted's.
            "neoforge:difference" => {
                let base = Self::walk_resolved(
                    o.get("base")?,
                    tags,
                    block_tags,
                    custom,
                    convert,
                    resolve,
                )?;
                let gone = Self::walk_resolved(
                    o.get("subtracted")?,
                    tags,
                    block_tags,
                    custom,
                    convert,
                    resolve,
                )?;
                nested = Some((Shape::Difference, vec![base.clone(), gone.clone()]));
                base.order
                    .iter()
                    .filter(|i| !gone.is_simple() || !gone.set.contains(*i))
                    .cloned()
                    .collect()
            }
            "neoforge:block_tag" => {
                let tag = o.get("tag")?.as_str()?;
                if tag.contains('#') {
                    return None;
                }
                let mut list = block_tags(tag)?;
                dedup_keep_order(&mut list);
                return Some(Self::flat(list));
            }
            "neoforge:components" => {
                if !matches!(o.get("items")?, Value::String(_) | Value::Array(_)) {
                    return None;
                }
                let mut base = Self::walk_resolved(
                    o.get("items")?,
                    tags,
                    block_tags,
                    custom,
                    convert,
                    resolve,
                )?;
                let mut patch = convert(o.get("components")?.as_object()?)?;
                let mut removed = Vec::new();
                patch.retain(|key, _| {
                    if let Some(id) = key.to_string_lossy().strip_prefix('!') {
                        removed.push(id.to_owned());
                        false
                    } else {
                        true
                    }
                });
                let strict = match o.get("strict") {
                    Some(v) => v.as_bool()?,
                    None => false,
                };
                base.rule = Some(IngredientRule::Components {
                    patch,
                    removed,
                    strict,
                });
                return Some(base);
            }
            "neoforge:custom_display" => {
                let base = Self::walk_resolved(
                    o.get("base")?,
                    tags,
                    block_tags,
                    custom,
                    convert,
                    resolve,
                )?;
                let mut shown = base.clone();
                shown.rule = None;
                shown.custom = None;
                shown.children = Some((Shape::CustomDisplay, vec![base]));
                shown.display = Some(convert(o.get("display")?.as_object()?)?);
                return Some(shown);
            }
            // A kind outside these shapes: the wrapper answers it — the
            // item ingredient asks its hook, a fluid one fails the file.
            k if is_custom(k) => {
                let mut data = convert(o)?;
                let list = custom(&mut data)?;
                return Some(Self {
                    set: list.iter().cloned().collect(),
                    order: list,
                    children: None,
                    custom: Some((k.to_owned(), data)),
                    rule: None,
                    display: None,
                });
            }
            _ => return None,
        };
        (!kept.is_empty()
            || nested
                .as_ref()
                .is_some_and(|(_, children)| children.iter().any(Self::allows_empty)))
        .then(|| Self {
            set: kept.iter().cloned().collect(),
            order: kept,
            children: nested,
            custom: None,
            rule: None,
            display: None,
        })
    }
}
