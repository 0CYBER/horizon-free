# Retained packet allocation traversal

Run from the repository root:

```sh
CARGO_TARGET_DIR="$PWD/target" cargo run --manifest-path crates/horizon-protocol/tools/retained-gen/Cargo.toml
rustfmt --edition 2024 $(git diff --name-only -- '*.rs')
python3 crates/horizon-protocol/tools/audit_retained_std.py
```

`packet!` and `packet_enum!` generate traversal from their existing field and
variant declarations. This tool handles manually declared protocol values with
`syn`; it emits exhaustive struct destructuring and enum matches. Adding a field
or variant without regenerating fails compilation. New field representations
must implement `retained::OwnedHeap`; there is no default-zero escape hatch.
Generic `TrimHolder` and the `move_entity!` macro have explicit traversal beside
their declarations. Test code and custom-payload codec registries are excluded.

## Charge policy

`Clientbound::owned_heap_bytes()` and `Serverbound::owned_heap_bytes()` report a
safe retained-allocation **charge**, not encoded length or an allocator census.
They exclude inline packet/enum storage, allocator metadata and rounding beyond
requested allocation layouts. Callers must charge their own queue/receipt nodes.

Vectors and strings charge capacity, not length, and recursively visit live
values. Boxes charge the boxed inline layout plus descendants. Arc allocations
are charged in full per reference with conservative header/alignment padding,
including item wire-patch caches retained by that packet; sharing can therefore
be overcounted. No global registry or unrelated shared cache is traversed.

NBT compounds use opaque std B-trees. Their conservative node-storage bound was
audited against Rust **1.99.1**, `alloc/src/collections/btree/{map,node,remove}.rs`.
This is an explicit std-layout dependency, not an exact reading of tree capacity:

- `B = 6`, maximum 11 key/value slots per node, at least 5 live entries in each
  non-root node (including after removal rebalancing).
- At most `1 + len / 5` nodes, allowing a retained empty root even for an empty
  map. A fresh empty map is intentionally overcharged too.
- Charge every possible node as an internal node: leaf header (parent pointer,
  two `u16`s), 11 keys, 11 values and 12 child pointers. Eight maximum-alignment
  gaps conservatively cover the five leaf fields, two internal fields and final
  layout rounding regardless of Rust field reordering.
- Live key/value descendants are charged recursively in addition to node slots.

`audit_retained_std.py` checks this layout contract in the installed `rust-src`.
Toolchain upgrades must run this audit and re-examine changed invariants; this
repository's general CI follows stable rather than freezing std's private layout.
No safe public std API exposes a B-tree allocation census. The isolated
`retained_census` integration-test binary instead installs horizon-threads'
existing `CensusAllocator` and uses a unique heap tag to observe requested live
allocation-layout bytes. The test invokes the unconditional tag API directly, so
its dev dependency does not enable `alloc-census` across production consumers.
The tag is scoped only around value construction; census result allocations and
the test harness are outside it. Each case checks all tagged bytes disappear on
drop. Populated and nested NBT compounds, spare child capacities, insertion/removal
churn and a retained empty root must all fit under the charge. This validates the
conservative bound against observed requested layouts, not allocator metadata,
allocator rounding or an exact node-by-node census.
There are no HashMap/HashSet fields in the decoded packet representations; the
custom-payload codec registry uses these but is not retained by a packet and is
not part of this traversal.

## Fresh configuration snapshots and hash tables

`registry::Dynamic::fresh_owned_heap_bytes()` sizes only a frozen snapshot built
by configure's insertion/replacement-only `add` / `add_tags` boundary. Its public
fields are not policed: erasing from any outer or nested hash table invalidates
the sizing precondition. Exhaustive destructuring makes new Dynamic fields a
compile error until their accounting is considered. Builtin/SyncedNames and other
shared global registries remain excluded. A containing Arc's storage must be
charged separately by the queue owner.

`retained::fresh_hash_table_bytes<T>(capacity)` is the shared storage-only bound:
pass `(K, V)` for a map or `T` for a set. It excludes inline table storage and
owned descendants, which the snapshot traversal charges separately. It must not
be used on arbitrary erased/tombstoned tables: public capacity reports
`items + growth_left`, not the physical bucket count. With no erasures, the
initial free-space budget and all insertions preserve the usable capacity.
Rust 1.99.1's vendored hashbrown 0.17.1 reserves one slot in small tables or 1/8
of buckets in larger tables; thus physical buckets are at most twice fresh
capacity. Storage is at most `2 * capacity * (size_of::<T>() + 1)` plus 16 mirrored
control bytes and `max(align_of::<T>(), 16) - 1` bytes for alignment before the
controls. Capacity zero has no allocation at this fresh boundary. All arithmetic
saturates rather than wrapping. The std audit now checks this hashbrown layout
and target group representations too; toolchain upgrades require re-audit.

The isolated census test also covers nested fresh Dynamic maps populated through
add/add_tags, including replacement, and table capacities across small/growth
boundaries with zero-sized and 128-byte-aligned set elements. It observes layouts
independently of these formulas; no universal HashMap OwnedHeap implementation is
provided.
