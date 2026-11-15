fn wrapped(rate: f32, operation: impl FnOnce(f32) -> i64) -> i64 {
    fnr_beta::note(&format!("WRAP_OPERATION autosave_interval({rate})"));
    operation(rate)
}
