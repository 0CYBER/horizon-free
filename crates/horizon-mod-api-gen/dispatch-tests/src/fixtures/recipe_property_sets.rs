mod crate_root {
    // Dispatch fixtures isolate ordering; API tests cover property validation.
    mod item {
        pub(crate) mod crafting {
            pub(crate) fn validate_recipe_properties<T>(_: &str, _: &[T], _: &mut Vec<T>) {}
        }
    }
}
