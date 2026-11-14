//! Special item models drawn by mods' renderers on the client:
//! `native_register_special_model_renderer` (NeoForge's `RegisterSpecialModelRendererEvent`) is
//! asked once at start, before resources load, and its types are what a `minecraft:special` model
//! may name besides vanilla's, or in place of one; a model resolves its type to the renderer's
//! slot when it decodes. When an item of such a model is resolved for drawing, on the main thread,
//! the type's renderer answers the resource models to draw (`ItemEnv::special_model_parts`),
//! checked here, then prepared by the item renderer, which bakes them once. The prepared answer
//! is kept by the stack for each model node for as long as that stack is drawn unchanged, so it
//! is asked once across frames, every display context draws the same answer, and a draw looks
//! nothing up by name.
use super::native_entity_renderers::Resolved;
use horizon_assets::{
    item_model::{PreparedParts, SpecialModelSlot, SpecialPart, StackView},
    math::Mat4,
};
use horizon_client::fast_hash::{FxHashMap, FxHasher};
use horizon_core::nbt::Tag;
use mod_api::{ClientHost, MAX_SPECIAL_MODEL_PARTS, SpecialModelPart, SpecialModelRenderer};
use serde_json::Value;
use std::{
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError, atomic::Ordering},
};

/// The registrations, by slot ([`SpecialModelSlot`]): each type and its renderer, kept once at
/// start.
static REGISTERED: OnceLock<Vec<(String, Resolved<dyn SpecialModelRenderer>)>> = OnceLock::new();

/// The prepared answers of the stacks drawn since the last frame began.
static ANSWERS: LazyLock<Mutex<Answers>> = LazyLock::new(Mutex::default);

fn answers() -> MutexGuard<'static, Answers> {
    ANSWERS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keeps the mods' registrations in slot order, once per process (false when they already were).
pub(super) fn register(renderers: Vec<(String, Arc<dyn SpecialModelRenderer>)>) -> bool {
    REGISTERED
        .set(
            renderers
                .into_iter()
                .map(|(kind, renderer)| (kind, Resolved::new(renderer)))
                .collect(),
        )
        .is_ok()
}

/// On every resource reload: the renderers that failed are asked again, and no answer is kept
/// (the model nodes and baked models they belonged to are gone).
pub(in crate::game) fn reload() {
    for (_, resolved) in REGISTERED.get().into_iter().flatten() {
        resolved.failed.store(false, Ordering::Relaxed);
    }
    let mut answers = answers();
    answers.kept.clear();
    answers.refused.clear();
}

/// Once per frame of the view `view` (its game's address), before it is drawn: the answers of
/// stacks no view's last frame drew are dropped, so what is kept is bounded by what is on screen.
/// Splitscreen's views draw one after another, each keeping what it drew.
pub(in crate::game) fn begin_frame(view: usize) {
    answers().begin_frame(view);
}

/// What the renderer of `slot` draws for `stack` in the model node of `config`: the kept answer
/// when this node drew an equal stack last frame or this one, else the renderer's, checked,
/// prepared by `prepare` and kept. `None` when no mod registered it, when it failed since
/// resources last loaded, or when it fails now.
pub(in crate::game) fn parts(
    host: &dyn ClientHost,
    slot: SpecialModelSlot,
    config: &Arc<Tag>,
    stack: &StackView,
    prepare: impl FnOnce(&[SpecialPart]) -> PreparedParts,
) -> Option<PreparedParts> {
    let (kind, resolved) = REGISTERED.get()?.get(slot.0 as usize)?;
    if resolved.failed.load(Ordering::Relaxed) {
        return None;
    }
    answer(&ANSWERS, Arc::as_ptr(config) as usize, stack, || {
        // A stack the API can't take is refused alone: the renderer isn't at fault.
        use super::api_values::TryIntoApi;
        let api_stack = stack
            .try_into_api()
            .map_err(|e| warn_unreadable_once(kind, &stack.item, || format!("{e:?}")))
            .ok()?;
        let parts = guarded(kind, resolved, || {
            resolved.inner.extract(host, &api_stack, config)
        })?;
        Some(prepare(&parts))
    })
}

/// Says once per type and item that a stack of the item couldn't be given to the type's
/// renderer, so it draws the missing model.
fn warn_unreadable_once(kind: &str, item: &str, error: impl FnOnce() -> String) {
    static SEEN: Mutex<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>> =
        Mutex::new(std::collections::BTreeMap::new());
    let mut seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    if seen.get(kind).is_some_and(|items| items.contains(item)) {
        return;
    }
    seen.entry(kind.to_owned())
        .or_default()
        .insert(item.to_owned());
    drop(seen);
    let error = error();
    log::warn!(
        "Horizon: a stack of {item} can't be read for the special model renderer of {kind} ({error}); it draws the missing model"
    );
}

/// The answer for `stack` in the model node `node`: the one kept in `answers` for an equal stack,
/// else what `ask` answers, kept. The lock isn't held while `ask` runs the mod's renderer.
fn answer(
    answers: &Mutex<Answers>,
    node: usize,
    stack: &StackView,
    ask: impl FnOnce() -> Option<PreparedParts>,
) -> Option<PreparedParts> {
    let lock = || answers.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (node, stack_hash(stack));
    if let Some(kept) = lock().drawn(key, stack) {
        return Some(kept);
    }
    if lock()
        .refused
        .get(&key)
        .is_some_and(|stacks| stacks.contains(stack))
    {
        return None;
    }
    let Some(prepared) = ask() else {
        lock().refused.entry(key).or_default().push(stack.clone());
        return None;
    };
    lock().keep(key, stack, prepared.clone());
    Some(prepared)
}

/// The prepared answers kept, by model node (its `config`'s address, stable until resources
/// reload) and the stack's [`stack_hash`].
#[derive(Default)]
struct Answers {
    /// Counts the frames begun, by any view.
    frame: u64,
    /// Each view's last two frames: when its previous one began and its current one.
    views: FxHashMap<usize, [u64; 2]>,
    kept: FxHashMap<(usize, u64), Vec<Kept>>,
    refused: FxHashMap<(usize, u64), Vec<StackView>>,
}
/// A view no frame of which began for this many frames (of all views) is gone.
const VIEW_GONE_AFTER: u64 = 256;
struct Kept {
    stack: StackView,
    prepared: PreparedParts,
    /// The frame (of any view) that last drew it.
    drawn: u64,
}
impl Answers {
    /// The answer kept for a stack equal to `stack`, now drawn in this frame.
    fn drawn(&mut self, key: (usize, u64), stack: &StackView) -> Option<PreparedParts> {
        let frame = self.frame;
        let kept = self
            .kept
            .get_mut(&key)?
            .iter_mut()
            .find(|k| k.stack == *stack)?;
        kept.drawn = frame;
        Some(kept.prepared.clone())
    }
    fn keep(&mut self, key: (usize, u64), stack: &StackView, prepared: PreparedParts) {
        self.kept.entry(key).or_default().push(Kept {
            stack: stack.clone(),
            prepared,
            drawn: self.frame,
        });
    }
    /// Starts the next frame of `view`, keeping only the answers drawn since the oldest of the
    /// views' previous frames began: what some view's last frame may have drawn.
    fn begin_frame(&mut self, view: usize) {
        self.refused.clear();
        self.frame += 1;
        let frame = self.frame;
        // A view's first frame keeps what was drawn before it began, as if its own.
        let starts = self.views.entry(view).or_insert([frame - 1; 2]);
        *starts = [starts[1], frame];
        self.views
            .retain(|_, [_, current]| frame - *current < VIEW_GONE_AFTER);
        let since = self.views.values().map(|[previous, _]| *previous).min();
        let since = since.unwrap_or(frame);
        self.kept.retain(|_, kept| {
            kept.retain(|k| k.drawn >= since);
            !kept.is_empty()
        });
    }
}

/// A hash of `stack` that equal stacks share, read in place.
fn stack_hash(stack: &StackView) -> u64 {
    let mut h = FxHasher::default();
    stack.item.hash(&mut h);
    stack.count.hash(&mut h);
    stack.bundle_selected.hash(&mut h);
    for (component, value) in &stack.patch {
        component.hash(&mut h);
        match value {
            Some(value) => value_hash(value).hash(&mut h),
            None => h.write_u8(0xFF),
        }
    }
    h.finish()
}
/// A hash of `value` that equal values share; an object's keys in any order.
fn value_hash(value: &Value) -> u64 {
    let mut h = FxHasher::default();
    std::mem::discriminant(value).hash(&mut h);
    match value {
        Value::Null => h.write_u8(0),
        Value::Bool(b) => b.hash(&mut h),
        Value::Number(n) => n.as_f64().map(f64::to_bits).hash(&mut h),
        Value::String(s) => s.hash(&mut h),
        Value::Array(items) => {
            for item in items {
                value_hash(item).hash(&mut h);
            }
        }
        Value::Object(fields) => fields
            .iter()
            .fold(0u64, |sum, (key, value)| {
                let mut field = FxHasher::default();
                key.hash(&mut field);
                value_hash(value).hash(&mut field);
                sum.wrapping_add(field.finish())
            })
            .hash(&mut h),
    }
    h.finish()
}

/// Runs `extract` for the renderer `resolved` of `kind` and checks its answer: `None`, with
/// `failed` set and logged once, when it returns an `Err`, panics or answers parts the resolver
/// refuses.
fn guarded(
    kind: &str,
    resolved: &Resolved<dyn SpecialModelRenderer>,
    extract: impl FnOnce() -> Result<Vec<SpecialModelPart>, String>,
) -> Option<Vec<SpecialPart>> {
    if resolved.failed.load(Ordering::Relaxed) {
        return None;
    }
    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(extract))
        .unwrap_or_else(|_| Err("it panicked".into()))
        .and_then(|parts| checked(&parts));
    match answer {
        Ok(parts) => Some(parts),
        Err(error) => {
            if !resolved.failed.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Horizon: the special model renderer of {kind} failed ({error}); its models draw the missing model until resources reload"
                );
            }
            None
        }
    }
}

/// `parts` as the resolver takes them, or why they are refused: more than
/// [`MAX_SPECIAL_MODEL_PARTS`], or a transform that isn't finite. Nothing is kept of a refusal.
fn checked(parts: &[SpecialModelPart]) -> Result<Vec<SpecialPart>, String> {
    if parts.len() > MAX_SPECIAL_MODEL_PARTS {
        return Err(format!(
            "{} parts, more than {MAX_SPECIAL_MODEL_PARTS}",
            parts.len()
        ));
    }
    parts
        .iter()
        .map(|part| {
            let [a, b, c, d, e, f, g, h, i, j, k, l] = part.transform;
            let pose =
                Mat4::from_row_major([a, b, c, d, e, f, g, h, i, j, k, l, 0.0, 0.0, 0.0, 1.0]);
            if !pose.is_finite() {
                return Err(format!("{}'s transform isn't finite", part.model));
            }
            Ok(SpecialPart {
                model: part.model.clone(),
                pose,
                tints: part.tints.iter().map(|&argb| argb as i32).collect(),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "native_special_renderers/frame_tests.rs"]
pub(in crate::game) mod frame_tests;

#[cfg(test)]
mod tests {
    use super::*;

    struct Unused;
    impl SpecialModelRenderer for Unused {
        fn extract(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStack,
            _: &Tag,
        ) -> Result<Vec<SpecialModelPart>, String> {
            unreachable!("the tests answer through `guarded`'s closure")
        }
    }

    fn renderer() -> Resolved<dyn SpecialModelRenderer> {
        Resolved::new(Arc::new(Unused))
    }

    /// A renderer's parts reach the resolver posed by their row-major transform and tinted.
    #[test]
    fn parts_reach_the_resolver_posed_and_tinted() {
        let parts = guarded("t:pan", &renderer(), || {
            Ok(vec![
                SpecialModelPart::new("t:item/a")
                    .translated([1.0, 2.0, 3.0])
                    .with_tints(vec![0xFF11_2233]),
            ])
        })
        .unwrap();
        assert_eq!(parts[0].model, "t:item/a");
        assert_eq!(parts[0].pose.0, Mat4::translation([1.0, 2.0, 3.0]).0);
        assert_eq!(parts[0].tints, [0xFF11_2233_u32 as i32]);
    }

    #[test]
    fn unreadable_answers_are_retained_only_for_the_frame() {
        let answers = Mutex::<Answers>::default();
        let stack = StackView::new("t:pan", 1);
        let asked = std::cell::Cell::new(0);
        let draw = || {
            answer(&answers, 1, &stack, || {
                asked.set(asked.get() + 1);
                None
            })
        };
        assert!(draw().is_none());
        assert!(draw().is_none());
        assert_eq!(asked.get(), 1);
        answers.lock().unwrap().begin_frame(1);
        assert!(draw().is_none());
        assert_eq!(asked.get(), 2);
    }

    /// An unchanged stack is asked once for as long as every frame draws it, however many stacks
    /// a model node draws; another stack, or the same stack in another node, is asked anew. A
    /// stack a frame didn't draw, or any after a reload, is asked again.
    #[test]
    fn an_unchanged_stack_is_extracted_once_across_frames() {
        const VIEW: usize = 1;
        let answers = Mutex::<Answers>::default();
        let asked = std::cell::Cell::new(0);
        let draw = |node: usize, stack: &StackView| {
            answer(&answers, node, stack, || {
                asked.set(asked.get() + 1);
                Some(PreparedParts(Arc::new(stack.count)))
            })
            .unwrap()
        };
        let count = |prepared: PreparedParts| *prepared.0.downcast_ref::<i32>().unwrap();
        // Nodes far from any real `config` address; 100 stacks of the pan in a stable order,
        // more than any fixed count would keep.
        let (pan, pot) = (1, 2);
        let stacks: Vec<StackView> = (1..=100).map(|n| StackView::new("t:pan", n)).collect();
        let first: Vec<PreparedParts> = stacks.iter().map(|s| draw(pan, s)).collect();
        for _frame in 0..10 {
            answers.lock().unwrap().begin_frame(VIEW);
            for (stack, first) in stacks.iter().zip(&first) {
                assert_eq!(&draw(pan, stack), first);
            }
        }
        assert_eq!(asked.get(), 100);
        // An equal stack read anew (the game builds its view every draw) is the same stack.
        let mut named = StackView::new("t:pan", 1);
        named.patch.insert(
            "minecraft:custom_name".into(),
            Some(serde_json::json!({"text": "Pan", "bold": true})),
        );
        let mut renamed = StackView::new("t:pan", 1);
        renamed.patch.insert(
            "minecraft:custom_name".into(),
            Some(serde_json::json!({"bold": true, "text": "Pan"})),
        );
        assert_eq!(draw(pan, &named), draw(pan, &renamed));
        assert_eq!(asked.get(), 101);
        assert_eq!(count(draw(pot, &stacks[0])), 1);
        assert_eq!(asked.get(), 102);
        // The next frame draws only the first stack: the others are asked again after it.
        answers.lock().unwrap().begin_frame(VIEW);
        assert_eq!(draw(pan, &stacks[0]), first[0]);
        answers.lock().unwrap().begin_frame(VIEW);
        assert_eq!(draw(pan, &stacks[0]), first[0]);
        assert_eq!(asked.get(), 102);
        assert_eq!(count(draw(pan, &stacks[1])), 2);
        assert_eq!(asked.get(), 103);
        assert_eq!(answers.lock().unwrap().kept.len(), 2);
        // Splitscreen: two views draw one after another, each its own stack, and neither's is
        // dropped by the other's frame.
        for _frame in 0..10 {
            answers.lock().unwrap().begin_frame(VIEW);
            assert_eq!(draw(pan, &stacks[0]), first[0]);
            answers.lock().unwrap().begin_frame(VIEW + 1);
            assert_eq!(count(draw(pan, &stacks[2])), 3);
        }
        assert_eq!(asked.get(), 104);
        answers.lock().unwrap().kept.clear();
        draw(pan, &stacks[0]);
        assert_eq!(asked.get(), 105);
    }

    /// An `Err`, a panic, 257 parts and a transform that isn't finite each refuse the answer
    /// whole, and the renderer isn't asked again until resources reload.
    #[test]
    fn a_failing_renderer_is_refused_whole_and_asked_no_more_until_reload() {
        let nan = SpecialModelPart::new("t:item/a").translated([f32::NAN, 0.0, 0.0]);
        let failures: [Box<dyn Fn() -> Result<Vec<SpecialModelPart>, String>>; 4] = [
            Box::new(|| Err("no pan".into())),
            Box::new(|| panic!("the pan fell")),
            Box::new(|| {
                Ok(vec![
                    SpecialModelPart::new("t:item/a");
                    MAX_SPECIAL_MODEL_PARTS + 1
                ])
            }),
            Box::new(move || Ok(vec![SpecialModelPart::new("t:item/b"), nan.clone()])),
        ];
        for fail in failures {
            let resolved = renderer();
            assert_eq!(guarded("t:pan", &resolved, fail), None);
            let mut asked = false;
            assert_eq!(
                guarded("t:pan", &resolved, || {
                    asked = true;
                    Ok(Vec::new())
                }),
                None
            );
            assert!(!asked, "a failed renderer is asked again before reload");
            resolved.failed.store(false, Ordering::Relaxed);
            assert_eq!(
                guarded("t:pan", &resolved, || Ok(Vec::new())),
                Some(Vec::new())
            );
        }
        let full = vec![SpecialModelPart::new("t:item/a"); MAX_SPECIAL_MODEL_PARTS];
        assert_eq!(
            guarded("t:pan", &renderer(), || Ok(full)).map(|parts| parts.len()),
            Some(MAX_SPECIAL_MODEL_PARTS)
        );
    }
}
