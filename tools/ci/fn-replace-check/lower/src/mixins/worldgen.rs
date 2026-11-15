fn worldgen() {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD shared_data_with (horizon_client_lib)"));
}
