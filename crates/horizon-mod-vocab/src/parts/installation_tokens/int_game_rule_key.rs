//! Opaque registration tokens; callers validate membership in their owning request table.
/// Cold authoring access to the opaque token representation.
pub fn int_game_rule_key(slot: u32) -> crate::IntGameRuleKey {
    crate::IntGameRuleKey(slot)
}
/// Cold authoring access to the opaque token representation.
pub fn int_game_rule_key_slot(key: crate::IntGameRuleKey) -> u32 {
    key.0
}

/// Validate a token using the current registration or installed lookup epoch.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub fn validate_int_game_rule_key(slot: u32) -> Result<crate::IntGameRuleKey, &'static str> {
    crate::IntGameRuleKey::from_slot(slot)
}
