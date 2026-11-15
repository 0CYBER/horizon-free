// Compiled against the actual generated crossing by the generator's focused test.
extern crate self as log;
#[macro_export]
macro_rules! warn {
    ($($args:tt)*) => {{
        crate::REFUSALS.fetch_add(1, crate::Ordering::Relaxed);
        eprintln!($($args)*);
    }};
}
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
static REFUSALS: AtomicUsize = AtomicUsize::new(0);
static SUBSCRIBED: AtomicBool = AtomicBool::new(false);
static CONVERSIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
struct Alloc;
unsafe impl std::alloc::GlobalAlloc for Alloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOC: Alloc = Alloc;
#[derive(Clone, Copy)]
pub struct EngineState(u32);
#[derive(Clone, Copy)]
pub struct ApiState(u32);
pub struct Game;
mod game {
    pub mod api_values {
        pub trait TryIntoApi {
            fn try_into_api(&self) -> Result<crate::ApiState, ()>;
        }
        pub trait TryIntoEngine<C: ?Sized> {
            fn try_into_engine(&self, context: &C) -> Result<crate::EngineState, ()>;
        }
        impl TryIntoApi for crate::EngineState {
            fn try_into_api(&self) -> Result<crate::ApiState, ()> {
                crate::CONVERSIONS.fetch_add(1, crate::Ordering::Relaxed);
                Ok(crate::ApiState(self.0))
            }
        }
        impl<C: ?Sized> TryIntoEngine<C> for crate::ApiState {
            fn try_into_engine(&self, _: &C) -> Result<crate::EngineState, ()> {
                if self.0 > 2 {
                    Err(())
                } else {
                    Ok(crate::EngineState(self.0))
                }
            }
        }
    }
}
mod mod_api {
    pub enum HookId {
        BlockProof,
    }
    pub mod __private {
        pub fn has_part_hook(_: super::HookId) -> bool {
            crate::SUBSCRIBED.load(crate::Ordering::Relaxed)
        }
    }
}
fn callback(state: &mut ApiState, sibling: &mut u32) {
    state.0 = *sibling;
    *sibling = 77;
}
impl Game {
    fn fire(&mut self, state: &mut EngineState, sibling: &mut u32) {
        FIRE_BODY
    }
    #[inline(never)]
    fn baseline(&mut self, state: &mut EngineState, sibling: &mut u32) {
        if !SUBSCRIBED.load(Ordering::Relaxed) {
            return;
        }
        use game::api_values::{TryIntoApi, TryIntoEngine};
        let mut api = state.try_into_api().unwrap();
        let mut output = *sibling;
        callback(&mut api, &mut output);
        let Ok(engine) = api.try_into_engine(&*self) else {
            return;
        };
        *state = engine;
        *sibling = output;
    }
}
fn main() {
    let mut game = Game;
    let mut state = EngineState(1);
    let mut sibling = 2;
    for _ in 0..1_000 {
        game.fire(&mut state, &mut sibling);
    }
    assert_eq!(CONVERSIONS.load(Ordering::Relaxed), 0);
    assert_eq!((state.0, sibling), (1, 2));
    SUBSCRIBED.store(true, Ordering::Relaxed);
    game.fire(&mut state, &mut sibling);
    assert_eq!((state.0, sibling), (2, 77));
    for invalid in [u32::MAX, 3] {
        sibling = invalid;
        game.fire(&mut state, &mut sibling);
        assert_eq!((state.0, sibling), (2, invalid));
    }
    assert_eq!(
        REFUSALS.load(Ordering::Relaxed),
        2,
        "invalid states must report a refusal"
    );
    let mut results = [(0u128, 0usize); 2];
    for (i, generated) in [false, true].into_iter().enumerate() {
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        let start = std::time::Instant::now();
        for _ in 0..1_000_000 {
            let mut state = std::hint::black_box(EngineState(1));
            let mut sibling = std::hint::black_box(2);
            if generated {
                game.fire(&mut state, &mut sibling);
            } else {
                game.baseline(&mut state, &mut sibling);
            }
            std::hint::black_box((state, sibling));
        }
        results[i] = (
            start.elapsed().as_nanos(),
            ALLOCATIONS.load(Ordering::Relaxed) - allocations,
        );
    }
    assert_eq!(results[1].1, 0);
    println!(
        "baseline=handwritten_checked_scalar iterations=1000000 ns={} allocations={}",
        results[0].0, results[0].1
    );
    println!(
        "generated iterations=1000000 ns={} allocations={}",
        results[1].0, results[1].1
    );
}
