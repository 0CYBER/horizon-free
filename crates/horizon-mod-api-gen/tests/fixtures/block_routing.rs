extern crate self as horizon_world;
extern crate self as log;
extern crate self as mod_api;
#[macro_export]
macro_rules! warn {
    ($($args:tt)*) => { eprintln!($($args)*) };
}
use std::sync::atomic::{AtomicUsize, Ordering};
static CONVERSIONS: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
struct Alloc;
unsafe impl std::alloc::GlobalAlloc for Alloc {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOC: Alloc = Alloc;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockKey(u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockState(BlockKey, u16);
impl BlockState {
    pub fn block(self) -> BlockKey {
        self.0
    }
    pub fn ordinal(self) -> u16 {
        self.1
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockStateError {
    Unavailable,
    UnknownBlock,
    UnknownState,
}
#[derive(Clone, Copy)]
pub enum HookId {
    Proof,
}
pub mod __private {
    pub fn has_part_hook(_: crate::HookId) -> bool {
        true
    }
    pub fn listened(_: crate::HookId) -> bool {
        true
    }
    pub fn has_scoped_hook(_: crate::HookId) -> bool {
        true
    }
    pub fn worldgen_block_key_listens(hook: crate::HookId, key: crate::BlockKey) -> bool {
        block_key_listens(hook, key)
    }
    pub fn block_key_listens(_: crate::HookId, key: crate::BlockKey) -> bool {
        key.0 == 7
    }
}
pub mod registry {
    #[derive(Clone, Copy, Debug)]
    pub struct BlockStateId(pub u32);
    pub struct BlockRegistry(pub Vec<Result<crate::BlockState, crate::BlockStateError>>);
    impl BlockRegistry {
        pub fn len(&self) -> usize {
            self.0.len()
        }
        pub fn state(
            &self,
            id: BlockStateId,
        ) -> Option<&Result<crate::BlockState, crate::BlockStateError>> {
            self.0.get(id.0 as usize)
        }
    }
}
mod game {
    pub mod api_values {
        pub trait TryIntoApi {
            type Api;
            type Error;
            fn try_into_api(&self) -> Result<Self::Api, Self::Error>;
        }
        pub trait TryIntoEngine<C: ?Sized> {
            type Engine;
            type Error;
            fn try_into_engine(&self, c: &C) -> Result<Self::Engine, Self::Error>;
        }
        impl TryIntoApi for Result<crate::BlockState, crate::BlockStateError> {
            type Api = crate::BlockState;
            type Error = crate::BlockStateError;
            fn try_into_api(&self) -> Self {
                *self
            }
        }
        impl TryIntoApi for Vec<u32> {
            type Api = Vec<u32>;
            type Error = ();
            fn try_into_api(&self) -> Result<Self::Api, ()> {
                crate::CONVERSIONS.fetch_add(1, crate::Ordering::Relaxed);
                Ok(self.clone())
            }
        }
        impl<C: ?Sized> TryIntoEngine<C> for Vec<u32> {
            type Engine = Vec<u32>;
            type Error = ();
            fn try_into_engine(&self, _: &C) -> Result<Self::Engine, ()> {
                Ok(self.clone())
            }
        }
        #[path = "INDEX_PATH"]
        pub mod block_state;
        pub fn bind(registry: &crate::registry::BlockRegistry) {
            block_state::bind(registry).unwrap();
        }
    }
}
struct Game;
impl Game {
    fn fire(
        &mut self,
        state: registry::BlockStateId,
        secondary: registry::BlockStateId,
        writes: &mut Vec<u32>,
    ) {
        FIRE_BODY
    }
}
fn callback(_: BlockState, _: BlockState, writes: &mut Vec<u32>) {
    CALLS.fetch_add(1, Ordering::Relaxed);
    writes.push(99);
}
fn main() {
    use game::api_values::block_state;
    let a = BlockState(BlockKey(7), 0);
    let b = BlockState(BlockKey(8), 0);
    for values in [vec![Ok(a), Ok(b)], vec![Ok(b), Ok(a)]] {
        game::api_values::bind(&registry::BlockRegistry(values.clone()));
        let b_id = registry::BlockStateId(values.iter().position(|v| *v == Ok(b)).unwrap() as u32);
        let mut writes = vec![42];
        let allocations = ALLOCATIONS.load(Ordering::Relaxed);
        let now = std::time::Instant::now();
        for _ in 0..1000 {
            Game.fire(std::hint::black_box(b_id), b_id, &mut writes);
        }
        let elapsed = now.elapsed();
        assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), allocations);
        assert_eq!(CONVERSIONS.load(Ordering::Relaxed), 0);
        assert_eq!(CALLS.load(Ordering::Relaxed), 0);
        assert_eq!(writes, vec![42]);
        println!(
            "1000 unrelated block events: {} ns, 0 conversions, 0 callbacks, 0 allocations, 0 writes",
            elapsed.as_nanos()
        );
        let a_id = registry::BlockStateId(values.iter().position(|v| *v == Ok(a)).unwrap() as u32);
        Game.fire(a_id, a_id, &mut writes);
        assert_eq!(writes, vec![42, 99]);
        CONVERSIONS.store(0, Ordering::Relaxed);
        CALLS.store(0, Ordering::Relaxed);
    }
}
