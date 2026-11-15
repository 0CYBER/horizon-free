//! The mod's side of the boundary, driven the way the game drives it, with no wasm.
#![cfg(feature = "wasm-guest")]
use mod_api::wire::{HOST_LOG, INIT, Writer, guest};
use std::cell::RefCell;
use std::rc::Rc;

#[unsafe(no_mangle)]
fn __horizon_mod_hooks() -> Box<dyn mod_api::Hooks> {
    struct Empty;
    impl mod_api::Hooks for Empty {
        fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[]
        }
    }
    Box::new(Empty)
}

fn call(id: u32, input: &[u8]) -> Result<Vec<u8>, String> {
    let at = guest::input(input.len());
    // SAFETY: `input` hands out a buffer of exactly this length.
    unsafe { std::ptr::copy_nonoverlapping(input.as_ptr(), at, input.len()) };
    let (pointer, len) = guest::run(id);
    let (pointer, len) = (pointer, len);
    if len == u32::MAX {
        // SAFETY: a refusal's reason is a wire string the guest keeps until the next call.
        return Err(format!("refused at {pointer:?}"));
    }
    // SAFETY: the reply is `len` bytes the guest keeps until the next call.
    Ok(unsafe { std::slice::from_raw_parts(pointer, len as usize) }.to_vec())
}

#[test]
fn start_up_installs_the_package_list_and_an_unknown_call_is_refused() {
    let mut packages = Writer::new();
    packages.put(&Vec::<mod_api::PackageInfo>::new());
    assert_eq!(call(INIT, packages.bytes()), Ok(Vec::new()));
    assert!(
        call(INIT, &[1, 2, 3]).is_err(),
        "a short package list is refused, not read"
    );
    assert!(call(0xFFFF_0000, &[]).is_err(), "no such call");
}

#[test]
fn a_panic_in_the_games_answer_keeps_its_message_and_a_mods_panic_reaches_the_game() {
    // Any call installs the guest's panic hook for the whole process.
    assert!(call(0xFFFF_0000, &[]).is_err());
    let calls = Rc::new(RefCell::new(Vec::new()));
    let game = calls.clone();
    guest::set_transport(Some(Rc::new(move |id, _| {
        // Held across the answer, as a game holds its call context while it serves.
        let mut calls = game.borrow_mut();
        calls.push(id);
        if id != HOST_LOG {
            panic!("the game refused call {id}");
        }
        Vec::new()
    })));
    let refused = std::panic::catch_unwind(|| guest::host(7, Writer::new()))
        .err()
        .expect("the game's answer must panic");
    assert_eq!(
        refused.downcast_ref::<String>().map(String::as_str),
        Some("the game refused call 7")
    );
    assert!(std::panic::catch_unwind(|| panic!("a mod's own panic")).is_err());
    guest::set_transport(None);
    assert_eq!(*calls.borrow(), [7, HOST_LOG]);
}
