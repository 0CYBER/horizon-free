#!/usr/bin/env python3
"""Audit the std B-tree and fresh hash table contracts used by retained.rs; run after a toolchain update.

This deliberately fails on changed node layout/invariants rather than pretending to
measure opaque std allocations. Install rust-src for the compiler being audited.
"""
import pathlib
import re
import subprocess

sysroot = pathlib.Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
root = sysroot / "lib/rustlib/src/rust/library/alloc/src/collections/btree"
source = (root / "node.rs").read_text()
for contract in (
    "const B: usize = 6;",
    "const CAPACITY: usize = 2 * B - 1;",
    "const MIN_LEN_AFTER_SPLIT: usize = B - 1;",
    "parent: Option<NonNull<InternalNode<K, V>>>",
    "parent_idx: MaybeUninit<u16>",
    "len: u16",
    "keys: [MaybeUninit<K>; CAPACITY]",
    "vals: [MaybeUninit<V>; CAPACITY]",
    "data: LeafNode<K, V>",
    "edges: [MaybeUninit<BoxedNode<K, V>>; 2 * B]",
    "type BoxedNode<K, V> = NonNull<LeafNode<K, V>>;",
):
    assert contract in source, f"B-tree accounting must be re-audited: {contract}"
for name, expected_fields in (("LeafNode", 5), ("InternalNode", 2)):
    body = re.search(rf"struct {name}<K, V> \{{(.*?)\n\}}", source, re.S).group(1)
    fields = re.findall(r"^\s+[a-z_]+:", body, re.M)
    assert len(fields) == expected_fields, f"{name} layout changed: {fields}"
print(subprocess.check_output(["rustc", "--version"], text=True).strip())
print("B-tree capacity, node fields and minimum occupancy contract match retained.rs")

assert "const MIN_LEN: usize = node::MIN_LEN_AFTER_SPLIT;" in (root / "map.rs").read_text()
assert "Every non-root node contains at least MIN_LEN elements." in (root / "map.rs").read_text()
assert "if len < MIN_LEN" in (root / "remove.rs").read_text()

# Rust 1.99.1 vendors this exact hashbrown version for std's HashMap.
hashbrown = root.parents[3] / "vendor/hashbrown-0.17.1/src"
raw = (hashbrown / "raw.rs").read_text()
for contract in (
    "self.table.items + self.table.growth_left",
    "if bucket_mask < 8",
    "((bucket_mask + 1) / 8) * 7",
    "size.checked_mul(buckets)?.checked_add(ctrl_align - 1)? & !(ctrl_align - 1)",
    "ctrl_offset.checked_add(buckets + Group::WIDTH)?",
    "layout.align() > Group::WIDTH",
):
    assert contract in raw, f"fresh hash table accounting must be re-audited: {contract}"
groups = hashbrown / "control/group"
expected = {"sse2.rs", "neon.rs", "lsx.rs", "generic.rs", "mod.rs"}
assert {p.name for p in groups.glob("*.rs")} == expected, "re-audit new hash control group target"
for name, representation in (("sse2", "x86::__m128i"), ("lsx", "m128i"), ("neon", "neon::uint8x8_t"), ("generic", "GroupWord")):
    group = (groups / f"{name}.rs").read_text()
    assert f"struct Group({representation})" in group
    assert "const WIDTH: usize = mem::size_of::<Self>();" in group
assert "type GroupWord = u64;" in (groups / "generic.rs").read_text()
assert "type GroupWord = u32;" in (groups / "generic.rs").read_text()
print("Fresh hash table capacity/layout and <=16-byte control group contracts match retained.rs")
