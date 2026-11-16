fn trait_block_state() {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| println!("fnr: trait <GameLevel as LevelAccess>::block_state"));
}
