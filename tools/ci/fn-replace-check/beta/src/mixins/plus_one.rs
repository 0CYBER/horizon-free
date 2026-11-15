fn plus_one(value: i64) -> i64 {
    fnr_beta::note(&format!("MODIFY_EXPRESSION_VALUE (priority 100): {value} -> {}", value + 1));
    value + 1
}
