//! Holds a big buffer the group's allocator may ask it to give back, then asks for more than the
//! group's memory allows: `hold` keeps the buffer, `grab` asks for one as big, `grab_all` asks for
//! more than the group's memory could ever hold.
use mod_api::{Flow, Hooks, Registrar, RegistrationContext, ServerChatEvent, ServerHost, Text};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering::Relaxed};

const MIB: usize = 1 << 20;

/// The buffer this mod holds: a leaked slice, so the evictor frees it without allocating or locking.
static HELD: AtomicPtr<MaybeUninit<u8>> = AtomicPtr::new(std::ptr::null_mut());
static HELD_LEN: AtomicUsize = AtomicUsize::new(0);
/// How many times the group's allocator asked this mod to give memory back.
static EVICTED: AtomicU32 = AtomicU32::new(0);

fn evict() {
    EVICTED.fetch_add(1, Relaxed);
    let held = HELD.swap(std::ptr::null_mut(), Relaxed);
    if !held.is_null() {
        let len = HELD_LEN.load(Relaxed);
        // SAFETY: `held` and `len` are the leaked slice `hold` stored.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(held, len)) });
    }
}

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, _registrar: &mut dyn Registrar, _cx: &RegistrationContext) {
        assert!(mod_api::wire::guest::register_eviction(evict));
    }

    fn server_chat(&self, _host: &mut dyn ServerHost, event: &mut ServerChatEvent<'_>) -> Flow {
        let answer = match event.raw_text {
            "" => return Flow::Continue,
            "hold" => {
                // Exercise allocation and eviction, not a 150 MiB byte-fill loop inside a
                // time-bounded hook. The fixture never reads the held allocation.
                let held = Box::leak(Box::<[u8]>::new_uninit_slice(150 * MIB));
                HELD_LEN.store(held.len(), Relaxed);
                HELD.store(held.as_mut_ptr(), Relaxed);
                "held"
            }
            asked => {
                let mut buffer: Vec<u8> = Vec::new();
                let size = if asked == "grab" {
                    150 * MIB
                } else {
                    1024 * MIB
                };
                match buffer.try_reserve_exact(size) {
                    Ok(()) => "grabbed",
                    Err(_) => "refused",
                }
            }
        };
        *event.message =
            Text::literal(format!("hog: {answer} [evicted={}]", EVICTED.load(Relaxed)));
        Flow::Continue
    }
}
