fn observe_commands() {
    static SEEN: std::sync::Once = std::sync::Once::new();
    SEEN.call_once(|| fnr_lower::note("HEAD Hub::set_mod_commands (horizon_client_lib)"));
}
