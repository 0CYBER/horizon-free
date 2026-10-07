//! Immutable item templates and strict stack validation. Template component
//! patches are interpreted against the current item prototype when used.
//! Component schemas supply typed size queries and contained-item validation.
use crate::{
    component_patch::{Patch, Prototype},
    item_stack_refs::{ItemStacks, StackRef},
    item_stack_storage::ItemStackData,
};
use std::{fmt, rc::Rc};

pub trait ValidationRules<I, K, V> {
    type ContentsError;
    fn has_max_damage(&self, stack: &ItemStackData<I, K, V>) -> bool;
    /// The component default is one, including on an empty stack.
    fn max_stack_size(&self, stack: &ItemStackData<I, K, V>) -> i32;
    /// Validate container sizes, then bundle sizes and weight, then charged
    /// projectile sizes. Only the immediate contained sizes are checked.
    fn validate_contents(&self, stack: &ItemStackData<I, K, V>) -> Result<(), Self::ContentsError>;
}
pub type Creation<I, K, V, E> = (StackRef<I, K, V>, Option<ValidationError<E>>);
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError<E> {
    StackableDamageable,
    Contents(E),
    TooManyItems { count: i32, maximum: i32 },
}
pub fn validate_strict<
    I: Copy + Eq,
    K: Clone + Eq,
    V: Clone + PartialEq,
    R: ValidationRules<I, K, V>,
>(
    stack: &ItemStackData<I, K, V>,
    rules: &R,
) -> Result<(), ValidationError<R::ContentsError>> {
    if rules.has_max_damage(stack) && rules.max_stack_size(stack) > 1 {
        return Err(ValidationError::StackableDamageable);
    }
    rules
        .validate_contents(stack)
        .map_err(ValidationError::Contents)?;
    let count = stack.count();
    let maximum = rules.max_stack_size(stack);
    if count > maximum {
        return Err(ValidationError::TooManyItems { count, maximum });
    }
    Ok(())
}
/// A rejected patch restores the original raw patch, including redundant entries.
/// The error is returned for the caller's diagnostic logger.
pub fn apply_validated<
    I: Copy + Eq,
    K: Clone + Eq,
    V: Clone + PartialEq,
    R: ValidationRules<I, K, V>,
>(
    stack: &mut ItemStackData<I, K, V>,
    patch: &Patch<K, V>,
    rules: &R,
) -> Result<(), ValidationError<R::ContentsError>> {
    let previous = stack.components.as_patch();
    stack.components.apply_patch(patch);
    let result = validate_strict(stack, rules);
    if result.is_err() {
        stack.components.restore_patch(&previous);
    }
    result
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateError {
    EmptyItem,
    EmptyStack,
}
impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyItem => "Item must be non-empty",
            Self::EmptyStack => "Stack must be non-empty",
        })
    }
}
impl std::error::Error for TemplateError {}
#[derive(Debug)]
struct Data<I, K, V> {
    item: I,
    air: I,
    count: i32,
    patch: Patch<K, V>,
}
#[derive(Debug)]
pub struct ItemTemplate<I, K, V>(Rc<Data<I, K, V>>);
impl<I, K, V> Clone for ItemTemplate<I, K, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq> ItemTemplate<I, K, V> {
    /// The native constructor rejects zero, but permits negative counts.
    pub fn new(item: I, air: I, count: i32, patch: Patch<K, V>) -> Result<Self, TemplateError> {
        if item == air || count == 0 {
            return Err(TemplateError::EmptyItem);
        }
        Ok(Self(Rc::new(Data {
            item,
            air,
            count,
            patch,
        })))
    }
    pub fn from_stack(stack: &ItemStackData<I, K, V>, air: I) -> Result<Self, TemplateError> {
        Self::new(stack.item(), air, stack.count(), stack.visible_patch())
    }
    pub fn from_non_empty_stack(
        stack: &ItemStackData<I, K, V>,
        air: I,
    ) -> Result<Self, TemplateError> {
        if stack.is_empty() {
            return Err(TemplateError::EmptyStack);
        }
        Self::from_stack(stack, air)
    }
    pub fn from_non_empty_stack_with_count(
        stack: &ItemStackData<I, K, V>,
        air: I,
        count: i32,
    ) -> Result<Self, TemplateError> {
        if stack.is_empty() || count <= 0 {
            return Err(TemplateError::EmptyStack);
        }
        Self::new(stack.item(), air, count, stack.visible_patch())
    }
    pub fn same_reference(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    pub fn item(&self) -> I {
        self.0.item
    }
    pub fn count(&self) -> i32 {
        self.0.count
    }
    pub fn patch(&self) -> &Patch<K, V> {
        &self.0.patch
    }
    pub fn with_count(&self, count: i32) -> Result<Self, TemplateError> {
        if count == self.count() {
            return Ok(self.clone());
        }
        Self::new(self.item(), self.0.air, count, self.0.patch.clone())
    }
    pub fn component<'a>(&'a self, prototype: &'a Prototype<K, V>, key: &K) -> Option<&'a V> {
        match self.0.patch.entries().iter().find(|(k, _)| k == key) {
            Some((_, value)) => value.as_ref(),
            None => prototype.get(key),
        }
    }
    /// Invalid creations return the store's canonical EMPTY. The optional error
    /// lets the caller log the rejected template without changing game behavior.
    pub fn create<R: ValidationRules<I, K, V>>(
        &self,
        stacks: &ItemStacks<I, K, V>,
        prototype: Prototype<K, V>,
        rules: &R,
    ) -> Creation<I, K, V, R::ContentsError> {
        self.apply(stacks, prototype, self.count(), &Patch::new([]), rules)
    }
    /// The supplied patch is the base; this template's patch takes precedence.
    pub fn apply<R: ValidationRules<I, K, V>>(
        &self,
        stacks: &ItemStacks<I, K, V>,
        prototype: Prototype<K, V>,
        count: i32,
        base: &Patch<K, V>,
        rules: &R,
    ) -> Creation<I, K, V, R::ContentsError> {
        let stack = stacks.create(self.item(), count, prototype);
        {
            let mut data = stack.write();
            data.components.apply_patch(base);
            data.components.apply_patch(self.patch());
        }
        let error = validate_strict(&stack.read(), rules).err();
        if error.is_some() {
            (stacks.empty(), error)
        } else {
            (stack, None)
        }
    }
}

impl<I: PartialEq, K: Eq, V: PartialEq> PartialEq for ItemTemplate<I, K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.0.item == other.0.item
            && self.0.count == other.0.count
            && self.0.patch == other.0.patch
    }
}
