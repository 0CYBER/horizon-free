/// How many falling ticks the sand part heard on this thread since the last call.
pub fn falling_ticks_heard() -> u32 {
    FALLING_TICKS.with(|heard| heard.replace(0))
}
