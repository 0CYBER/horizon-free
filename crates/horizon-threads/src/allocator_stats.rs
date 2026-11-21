//! The heap the program has in use in mimalloc, from its statistics.

/// The number after `"key":` on a line of mimalloc's JSON.
fn field(line: &str, key: &str) -> Option<usize> {
    let from = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    let digits: String = line[from..]
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Bytes of the pages mimalloc holds now: `page_bins` counts the pages of each size class (pages,
/// not blocks, maintained whatever statistics the build keeps) and `page_size` is what a page of
/// that class costs. The huge class prints no `page_size`: its blocks have no size class of their
/// own, and the statistics say only how many huge pages there are, not how big, so they are left
/// out.
fn in_use(json: &str) -> Option<usize> {
    let bins = json.split("\"page_bins\"").nth(1)?;
    let bins = bins.split(']').next()?;
    let mut total = 0;
    for line in bins.lines().filter(|l| l.contains("\"block_size\"")) {
        let page_size = field(line, "page_size")?;
        if page_size == 0 {
            continue;
        }
        total += field(line, "current")? * page_size;
    }
    Some(total)
}

fn stats_json() -> Option<String> {
    // SAFETY: a null buffer asks mimalloc for one of its own, freed below with its `mi_free`.
    let raw = unsafe { libmimalloc_sys::mi_stats_get_json(0, std::ptr::null_mut()) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: mimalloc returns a NUL-terminated string; `.cast()` bridges `cty::c_char` (`u8` on Windows ARM64) to `std::ffi::c_char`.
    let text = unsafe { std::ffi::CStr::from_ptr(raw.cast()) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `raw` came from `mi_stats_get_json` with no buffer given.
    unsafe { libmimalloc_sys::mi_free(raw.cast()) };
    Some(text)
}

/// The bytes of the pages holding allocations now: the heap in use, counting each page whole.
/// A huge block (one bigger than the largest size class) is not counted: this build's statistics do
/// not say how big one is, so the live figure leaves them out and `overhead_mb` takes them in.
pub fn live_bytes() -> Option<usize> {
    in_use(&stats_json()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_a_page_once_however_many_blocks_it_holds() {
        let json = r#"{
  "malloc_huge": { "total": 9, "peak": 9, "current": 1000 },
  "malloc_bins": [
    { "total": 9, "peak": 9, "current": 3, "block_size": 112, "page_size": 65536 },
    { "total": 1, "peak": 1, "current": 2, "block_size": 524296, "page_size": 0 }
  ],
  "page_bins": [
    { "total": 5, "peak": 5, "current": 1, "block_size": 112, "page_size": 65536 },
    { "total": 4, "peak": 4, "current": 2, "block_size": 16384, "page_size": 524288 },
    { "total": 3, "peak": 3, "current": 3, "block_size": 524296, "page_size": 0 }
  ]
}"#;
        // Three blocks live in the 112-byte bin's one page; the three huge pages have no page size.
        assert_eq!(in_use(json), Some(65536 + 2 * 524288));
    }

    #[test]
    fn a_freed_large_allocation_returns_to_its_baseline() {
        // The first call makes the buffer the statistics are printed into, and its pages count
        // from the next call on: read once so the baseline holds them.
        live_bytes().unwrap();
        let before = live_bytes().unwrap();
        // SAFETY: allocates and frees one block with mimalloc.
        let block = unsafe { libmimalloc_sys::mi_malloc(64 << 20) };
        let during = live_bytes().unwrap();
        let (commit, _) = crate::allocator_commit();
        // SAFETY: `block` came from `mi_malloc` above.
        unsafe { libmimalloc_sys::mi_free(block) };
        let after = live_bytes().unwrap();
        // A huge block has no page size to count, and leaving it out is what keeps the figure
        // under `commit_mb`; a cumulative counter would have kept the 64 MiB in `after`. The
        // statistics buffer itself may take or release a page between the reads, so allow a
        // few pages of slack: the 64 MiB block would be far above it.
        let slack = 4 << 20;
        assert!(during <= commit, "{during} over {commit} committed");
        assert!(
            after <= before + slack,
            "{before} then {during} then {after}"
        );
    }
}
