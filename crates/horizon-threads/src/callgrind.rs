//! Callgrind client requests for the hot-path benchmark (docs/hot-path-trends.md): the benchmark
//! marks its measured regions with them, so a run under `valgrind --tool=callgrind
//! --instr-atstart=no` counts the instructions of each region and nothing of its setup.
//!
//! A client request is a sequence of instructions that does nothing on a real CPU (four rotations
//! of `rdi` that add up to a full turn, then an exchange of a register with itself) and that
//! Valgrind recognizes. Outside Valgrind every function here is a no-op, and off x86-64 Linux,
//! where the benchmark's instruction counts are not recorded, they compile to nothing.
use std::ffi::CStr;

/// `VG_USERREQ__RUNNING_ON_VALGRIND` (valgrind.h).
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
const RUNNING_ON_VALGRIND: u64 = 0x1001;
/// `VG_USERREQ_TOOL_BASE('C', 'T')`, the first of Callgrind's requests (callgrind.h).
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
const CALLGRIND: u64 = (b'C' as u64) << 24 | (b'T' as u64) << 16;

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn request(code: u64, argument: u64) -> u64 {
    let arguments: [u64; 6] = [code, argument, 0, 0, 0, 0];
    let mut result = 0u64;
    // SAFETY: valgrind.h's `VALGRIND_DO_CLIENT_REQUEST_EXPR` for amd64. On a real CPU the
    // rotations leave `rdi` as it was and the exchange changes nothing, so only the flags are
    // written. Under Valgrind the request reads the six words `rax` points to, which outlive
    // the block, and answers in `rdx`.
    unsafe {
        std::arch::asm!(
            "rol rdi, 3",
            "rol rdi, 13",
            "rol rdi, 61",
            "rol rdi, 51",
            "xchg rbx, rbx",
            in("rax") arguments.as_ptr(),
            inout("rdx") result,
            options(nostack),
        );
    }
    result
}

/// Whether this process runs under Valgrind.
pub fn running() -> bool {
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        request(RUNNING_ON_VALGRIND, 0) != 0
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
    {
        false
    }
}

/// Start counting: `CALLGRIND_START_INSTRUMENTATION`, then `CALLGRIND_ZERO_STATS`.
pub fn start() {
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        request(CALLGRIND + 4, 0);
        request(CALLGRIND + 1, 0);
    }
}

/// Write what was counted since [`start`] as one dump whose trigger line carries `name`, then
/// stop counting: `CALLGRIND_DUMP_STATS_AT`, then `CALLGRIND_STOP_INSTRUMENTATION`.
pub fn stop(name: &CStr) {
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        request(CALLGRIND + 3, name.as_ptr() as u64);
        request(CALLGRIND + 5, 0);
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "linux")))]
    let _ = name;
}
