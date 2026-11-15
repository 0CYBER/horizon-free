//! Special item model renderers, as a client mod registers them in
//! `register_special_model_renderer` (NeoForge's `RegisterSpecialModelRendererEvent` and
//! vanilla's `SpecialModelRenderer`).
//!
//! A renderer draws every `minecraft:special` item model whose `model` object has its type
//! (`mymod:skillet`). Its geometry is resource models: from the stack, the renderer answers which
//! models draw, each posed and tinted, and the game draws them as it draws any item model, with
//! the special model's `base` display transforms, particle and GUI light.
use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{ClientHost, ItemStack, Tag};
use std::{collections::BTreeMap, sync::Arc};

/// One resource model a special model renderer draws.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct SpecialModelPart {
    /// The model's id (`mymod:item/skillet_pan`), a model file of the packs.
    pub model: String,
    /// Where it is drawn: a row-major 3×4 affine transform in the special model's space, in
    /// blocks with the origin at the model's corner (a model element's coordinates over 16).
    pub transform: [f32; 12],
    /// ARGB per tint index, as an item model's `tints`: a quad with a tint index past them, or
    /// none, draws white.
    pub tints: Vec<u32>,
}

impl SpecialModelPart {
    /// The identity transform.
    pub const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    /// `model` where the special model is, untinted.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            transform: Self::IDENTITY,
            tints: Vec::new(),
        }
    }

    /// The part drawn under `transform` (row-major 3×4 affine).
    pub fn with_transform(mut self, transform: [f32; 12]) -> Self {
        self.transform = transform;
        self
    }

    /// The part moved by `by`, in blocks, after its transform.
    pub fn translated(mut self, by: [f32; 3]) -> Self {
        self.transform[3] += by[0];
        self.transform[7] += by[1];
        self.transform[11] += by[2];
        self
    }

    /// The part tinted per tint index (ARGB).
    pub fn with_tints(mut self, tints: Vec<u32>) -> Self {
        self.tints = tints;
        self
    }
}

/// The most parts one answer of a special model renderer may have: more refuse it.
pub const MAX_SPECIAL_MODEL_PARTS: usize = 256;

/// A mod's special model renderer: vanilla's `SpecialModelRenderer`, held by the game for every
/// special model of the type it is registered for. Models of a type share one renderer, so what
/// differs between them comes from the model object (`config`) and the stack.
/// Renderer answers depend only on stack and configuration. Host-state changes do not invalidate a continuously drawn unchanged stack. Answers drop after all views stop drawing the stack or resources reload.
pub trait SpecialModelRenderer: Send + Sync + 'static {
    /// `extractArgument` and `submit` in one: the models to draw for `stack`, in order, from the
    /// special model's `model` object (`config`, its JSON as NBT, converted once at load). Asked
    /// on the client's main thread when an item of such a model is drawn and its stack differs
    /// from the last one this model was asked about: an unchanged stack is asked once, however
    /// many frames draw it. Every display context (a hand, a screen, the ground, a frame, a head)
    /// draws the same answer, under the `base` model's display transforms. Diverges: vanilla's
    /// renderer submits geometry; Horizon's answers resource models, so geometry stays in the
    /// packs.
    ///
    /// An `Err`, a panic, a wasm call that is refused or traps, more than
    /// [`MAX_SPECIAL_MODEL_PARTS`] parts, and a transform that isn't finite all refuse the answer
    /// whole: the special model draws the missing model, the error is logged once with the type,
    /// and the renderer is asked no more until resources reload. A part naming a model no pack
    /// has refuses the answer too, logged once for the model.
    fn extract(
        &self,
        host: &dyn ClientHost,
        stack: &ItemStack,
        config: &Tag,
    ) -> Result<Vec<SpecialModelPart>, String>;
}

crate::wire::wire_handlers::handler!(dyn SpecialModelRenderer, "SpecialModelRenderer");

/// The special model renderer registry `register_special_model_renderer` hands out: NeoForge's
/// `RegisterSpecialModelRendererEvent`.
pub trait SpecialModelRenderers {
    /// `register(location, codec)`: `renderer` draws every special model of the type `kind`
    /// (`mymod:skillet`), vanilla's types included. A later registration of a type replaces the
    /// earlier one, whichever mod made it, as in NeoForge; the game logs both mods' ids. An id
    /// that is not `<namespace>:<path>` is logged and left out.
    fn register(&mut self, kind: String, renderer: Arc<dyn SpecialModelRenderer>);
}

// no wire: a report the game logs at registration; it holds `&'static` mod ids and never crosses to a guest.
/// A registration that replaced an earlier one of the same type.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplacedSpecialModelRenderer {
    /// The type.
    pub kind: String,
    /// The mod that registered it first, when a mod's dispatch did.
    pub earlier: Option<&'static str>,
    /// The mod whose registration stays.
    pub later: Option<&'static str>,
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`SpecialModelRenderers`]: the registrations by type.
#[derive(Default)]
// exhaustive: its fields are private; the game builds it
pub struct SpecialModelRendererRegistry {
    renderers: BTreeMap<String, (Option<&'static str>, Arc<dyn SpecialModelRenderer>)>,
    replaced: Vec<ReplacedSpecialModelRenderer>,
    initial: Vec<(String, Option<&'static str>)>,
}

impl SpecialModelRenderers for SpecialModelRendererRegistry {
    fn register(&mut self, kind: String, renderer: Arc<dyn SpecialModelRenderer>) {
        let later = crate::content::crash::current_native_mod();
        if let Some((earlier, _)) = self.renderers.insert(kind.clone(), (later, renderer)) {
            self.replaced.push(ReplacedSpecialModelRenderer {
                kind,
                earlier,
                later,
            });
        } else {
            self.initial.push((kind, later));
        }
    }
}

impl SpecialModelRendererRegistry {
    /// The first mod registration of each type. The engine uses its own builtin decoder
    /// to distinguish replacement of a game type from a new type, irrespective of namespace.
    pub fn initial_owners(&self) -> impl Iterator<Item = (&str, Option<&'static str>)> {
        self.initial
            .iter()
            .map(|(kind, owner)| (kind.as_str(), *owner))
    }

    /// Each type's renderer, the last registration of a type winning, and the registrations that
    /// replaced an earlier one.
    pub fn into_renderers(
        self,
    ) -> (
        BTreeMap<String, Arc<dyn SpecialModelRenderer>>,
        Vec<ReplacedSpecialModelRenderer>,
    ) {
        let renderers = self
            .renderers
            .into_iter()
            .map(|(kind, (_, renderer))| (kind, renderer))
            .collect();
        (renderers, self.replaced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;
    impl SpecialModelRenderer for Nothing {
        fn extract(
            &self,
            _: &dyn ClientHost,
            _: &ItemStack,
            _: &Tag,
        ) -> Result<Vec<SpecialModelPart>, String> {
            Ok(Vec::new())
        }
    }

    /// The last registration of a type wins, vanilla's types included, and each replacement is
    /// reported.
    #[test]
    fn the_last_registration_of_a_type_wins() {
        let last: Arc<dyn SpecialModelRenderer> = Arc::new(Nothing);
        let mut registry = SpecialModelRendererRegistry::default();
        registry.register("a:pan".into(), Arc::new(Nothing));
        registry.register("a:pan".into(), last.clone());
        registry.register("minecraft:shield".into(), Arc::new(Nothing));
        assert_eq!(
            registry.initial_owners().collect::<Vec<_>>(),
            vec![("a:pan", None), ("minecraft:shield", None)]
        );
        let (renderers, replaced) = registry.into_renderers();
        assert_eq!(renderers.len(), 2);
        assert!(Arc::ptr_eq(&renderers["a:pan"], &last));
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].kind, "a:pan");
    }

    /// A part's helpers move and tint it without turning it.
    #[test]
    fn a_part_moves_after_its_transform() {
        let part = SpecialModelPart::new("a:item/b")
            .with_transform([2.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0])
            .translated([0.5, 0.25, 0.0])
            .with_tints(vec![0xFF00_FF00]);
        assert_eq!(
            part.transform,
            [2.0, 0.0, 0.0, 1.5, 0.0, 2.0, 0.0, 0.25, 0.0, 0.0, 2.0, 0.0]
        );
        assert_eq!(part.tints, [0xFF00_FF00]);
    }
}
