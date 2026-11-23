//! Shapeless item-list or concrete-stack assignment.
use super::*;
/// A perfect matching of the ingredients to distinct stacks (`StackedContents.tryPick`).
pub(super) fn matches_all(
    context: &LoopContext,
    ingredients: &[Ingredient],
    stacks: &[&ItemStack],
) -> bool {
    fn augment(
        context: &LoopContext,
        i: usize,
        ingredients: &[Ingredient],
        stacks: &[&ItemStack],
        owner: &mut [Option<usize>],
        seen: &mut [bool],
        simple: bool,
    ) -> bool {
        for s in 0..stacks.len() {
            if seen[s]
                || !(if simple {
                    ingredients[i].items.contains(&*stacks[s].item)
                } else {
                    ingredients[i].test_in(context, stacks[s])
                })
            {
                continue;
            }
            seen[s] = true;
            if owner[s]
                .is_none_or(|o| augment(context, o, ingredients, stacks, owner, seen, simple))
            {
                owner[s] = Some(i);
                return true;
            }
        }
        false
    }
    if ingredients.len() != stacks.len() {
        return false;
    }
    let simple = ingredients.iter().all(Ingredient::is_simple);
    let n = stacks.len();
    let mut inline_owner = [None; 9];
    let mut inline_seen = [false; 9];
    let mut heap_owner;
    let mut heap_seen;
    let (owner, seen): (&mut [Option<usize>], &mut [bool]) = if n <= 9 {
        (&mut inline_owner[..n], &mut inline_seen[..n])
    } else {
        heap_owner = vec![None; n];
        heap_seen = vec![false; n];
        (&mut heap_owner, &mut heap_seen)
    };
    (0..ingredients.len()).all(|i| {
        seen.fill(false);
        augment(context, i, ingredients, stacks, owner, seen, simple)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shapeless_components_tests_each_stack_and_simple_uses_lists() {
        let load = crate::test_resources::server_loop_context();
        let items = Items::empty();
        let ingredient = Ingredient::parse(&serde_json::json!({"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}}), &items).unwrap();
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        assert!(!matches_all(
            load,
            std::slice::from_ref(&ingredient),
            &[&sword]
        ));
        sword.damage = 5;
        assert!(matches_all(load, &[ingredient], &[&sword]));
        let mut simple = Ingredient::of(vec!["minecraft:diamond_sword".into()]);
        simple.children = Some((
            Shape::Compound,
            vec![Ingredient::of(vec!["minecraft:apple".into()])],
        ));
        assert!(simple.is_simple());
        assert!(
            !simple.test_in(load, &sword),
            "shaped recipes follow the concrete predicate"
        );
        assert!(
            matches_all(load, &[simple], &[&sword]),
            "all-simple shapeless matching uses the flat item list"
        );
    }
}
