//! Concrete stack matching, shared by ordinary crafting and placement.
#[cfg(test)]
#[path = "matching_tests.rs"]
mod tests;
use super::*;
impl Ingredient {
    /// `test` with the loop context the game already holds.
    pub fn test_in(
        &self,
        context: &horizon_world::native_load_context::LoopContext,
        stack: &ItemStack,
    ) -> bool {
        self.test_in_guarded(context, stack, &|| true)
            .unwrap_or(false)
    }
    /// Stop before another custom leaf runs if a policy changed the caller's
    /// context. `None` is interruption, never a failed ingredient predicate.
    pub fn test_in_guarded(
        &self,
        context: &horizon_world::native_load_context::LoopContext,
        stack: &ItemStack,
        guard: &impl Fn() -> bool,
    ) -> Option<bool> {
        match &self.children {
            Some((Shape::CustomDisplay, kids)) => match kids.first() {
                Some(base) => base.test_in_guarded(context, stack, guard),
                None => Some(false),
            },
            Some((Shape::Compound, kids)) => {
                for child in kids {
                    if child.test_in_guarded(context, stack, guard)? {
                        return Some(true);
                    }
                }
                Some(false)
            }
            Some((Shape::Intersection, kids)) => {
                for child in kids {
                    if !child.test_in_guarded(context, stack, guard)? {
                        return Some(false);
                    }
                }
                Some(true)
            }
            Some((Shape::Difference, kids)) => {
                let Some(first) = kids.first() else {
                    return Some(false);
                };
                if !first.test_in_guarded(context, stack, guard)? {
                    return Some(false);
                }
                match kids.get(1) {
                    Some(second) => Some(!second.test_in_guarded(context, stack, guard)?),
                    None => Some(true),
                }
            }
            None => {
                if (self.custom.is_none() || !self.items.is_empty())
                    && !self.items.contains(&*stack.item)
                {
                    return Some(false);
                }
                if self.rule.is_some() {
                    return Some(
                        self.compiled
                            .as_ref()
                            .is_some_and(|rule| rule.matches(stack)),
                    );
                }
                if let Some((kind, data)) = &self.custom {
                    if !guard() {
                        return None;
                    }
                    let mut accept = true;
                    native_ingredient_test(context, kind, data, stack, &mut accept);
                    if !guard() {
                        return None;
                    }
                    return Some(accept);
                }
                Some(true)
            }
        }
    }
}
