fn doubled(value: i64) -> i64 {
    fnr_alpha::note(&format!("MODIFY_EXPRESSION_VALUE (priority 200): {value} -> {}", value * 2));
    value * 2
}
