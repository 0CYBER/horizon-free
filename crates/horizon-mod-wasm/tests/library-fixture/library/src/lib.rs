use mod_api::Hooks;

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {}

/// Exported across the wasm group.
#[inline(never)]
pub fn shared_value() -> i32 {
    37
}

pub static mut SHARED: i32 = 0;
pub static mut DROPS: i32 = 0;
pub fn generic<T: Into<i64>>(value: T) -> i64 {
    value.into() + 1
}
pub struct Owned {
    pub text: String,
    pub values: Vec<i32>,
}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            DROPS += 1;
        }
    }
}
#[inline(never)]
pub fn owned() -> Owned {
    Owned {
        text: "shared".into(),
        values: vec![7, 8],
    }
}
pub trait Answer {
    fn answer(&self) -> i32;
}
struct Concrete;
impl Answer for Concrete {
    fn answer(&self) -> i32 {
        19
    }
}
#[inline(never)]
pub fn object() -> Box<dyn Answer> {
    Box::new(Concrete)
}
