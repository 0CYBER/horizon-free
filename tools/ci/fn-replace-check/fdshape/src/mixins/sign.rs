fn sign(kind: &str, wood: &str, original: impl FnOnce(&str, &str) -> String) -> String {
    original(kind, wood)
}
