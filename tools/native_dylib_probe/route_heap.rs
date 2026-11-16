//! Windows only: which heap each allocator of the shipped prebuilt route draws from.
//! Built exactly like the launcher's executable (prefer-dynamic, the SDK's game dylib). Pointers are classified by
//! asking the process heap (std's System allocator) whether it owns them; nothing is freed across allocators.
//! Arguments: game DLL, std DLL, the allocator's exported alloc and dealloc names.
// Links the game DLL like the shipped executable does; without it rustc never loads it, and `mod_api` (metadata
// only in the SDK) would have to be linked statically.
extern crate horizon_client;

use std::ffi::{CString, c_void};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut c_void;
    fn HeapValidate(heap: *mut c_void, flags: u32, block: *const c_void) -> i32;
    fn GetModuleHandleA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

type Alloc = unsafe fn(usize, usize) -> *mut u8;
type Dealloc = unsafe fn(*mut u8, usize, usize);

fn process_heap(pointer: *const u8) -> bool {
    // SAFETY: HeapValidate only reads the heap's own bookkeeping; a foreign pointer yields FALSE.
    unsafe { HeapValidate(GetProcessHeap(), 0, pointer.cast()) != 0 }
}

fn export(module: &str, name: &str) -> Option<*mut c_void> {
    let module = CString::new(module).unwrap();
    let name = CString::new(name).unwrap();
    // SAFETY: both modules are already loaded by this executable's imports.
    let address = unsafe {
        GetProcAddress(
            GetModuleHandleA(module.as_ptr().cast()),
            name.as_ptr().cast(),
        )
    };
    (!address.is_null()).then_some(address)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, std_dll, alloc, dealloc] = <[String; 4]>::try_from(args).unwrap();
    let mut rows = Vec::new();
    for module in [&game, &std_dll] {
        // A game without its own allocator exports none: only the images that do are classified.
        let (Some(allocate), Some(free)) = (export(module, &alloc), export(module, &dealloc))
        else {
            continue;
        };
        // SAFETY: the Rust allocator ABI of this exact compiler: (size, align) and (pointer, size, align).
        unsafe {
            let allocate: Alloc = std::mem::transmute(allocate);
            let free: Dealloc = std::mem::transmute(free);
            let pointer = allocate(64, 8);
            rows.push((format!("{module} allocator"), process_heap(pointer)));
            free(pointer, 64, 8);
        }
    }
    let executable = Box::new([0u8; 64]);
    rows.push(("executable Box".into(), process_heap(executable.as_ptr())));
    mod_api::content::crash::set_detail("allocator probe", "game");
    let game_copy = mod_api::content::crash::details();
    rows.push((
        "mod_api::content::crash::details() Vec".into(),
        process_heap(game_copy.as_ptr().cast()),
    ));
    let json: Vec<_> = rows
        .iter()
        .map(|(name, heap)| format!(r#"{{"allocation":"{name}","process_heap":{heap}}}"#))
        .collect();
    println!("[{}]", json.join(","));
    let split = rows.iter().any(|row| row.1 != rows[0].1);
    println!(
        "{}",
        if split {
            "TWO ALLOCATORS: the route's allocations come from different heaps"
        } else {
            "ONE ALLOCATOR: every allocator in the route draws from one heap"
        }
    );
    // Freeing the game's copy here would cross allocators when they differ.
    std::mem::forget(game_copy);
}
