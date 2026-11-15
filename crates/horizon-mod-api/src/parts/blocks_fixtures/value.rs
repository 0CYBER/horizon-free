#[cfg(test)]
fn value(state: u32) -> BlockState {
    let name = [
        "minecraft:air",
        "test:gauge",
        "test:other",
        "test:connector",
        "test:press",
    ]
    .get(state as usize)
    .copied()
    .unwrap_or("test:gauge");
    let key = mod_vocab::__private::block_key(name).unwrap();
    BlockState::from(BlockStateRecord::new(
        key,
        if state > 4 { u16::MAX } else { 0 },
    ))
}
