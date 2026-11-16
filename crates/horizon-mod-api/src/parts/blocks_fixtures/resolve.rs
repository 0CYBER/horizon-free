#[cfg(test)]
/// Resolve this thread's parts with tag `test:gauged` holding `members`.
pub(crate) fn resolve(members: &[&str]) -> Result<(), super::ResolvePartsError> {
    mod_vocab::__private::resolve_block_keys(
        [
            "minecraft:air",
            "minecraft:granite",
            "test:gauge",
            "test:other",
            "test:connector",
            "test:press",
            "test:void_bin",
            "test:ordered",
        ],
        |_| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&Default::default()], 0)
                .map_err(|e| format!("{e:?}"))
        },
    )
    .unwrap();
    let tags = HashMap::from([(
        "test:gauged".to_owned(),
        members.iter().map(|m| m.to_string()).collect(),
    )]);
    resolve_parts(
        &tags,
        &HashMap::new(),
        KnownIds {
            block: &|_| true,
            item: &|_| true,
        },
    )
}
