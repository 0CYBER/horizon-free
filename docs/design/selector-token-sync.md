# Selector-token synchronization

Horizon sends `horizon:selector_tokens` immediately before each play command tree to a
Horizon peer. Vanilla command-tree bytes are unchanged. The channel is reserved for the
engine; a mod cannot declare it as a raw payload. The snapshot contains parsing and
completion metadata, never the server's callable selector implementation.

## Revision 1 layout

All integer and string encodings below are the protocol's standard encodings. Fixed-width
integers are big-endian. A string is a VarInt UTF-8 byte length followed by those bytes.

| Field | Encoding |
| --- | --- |
| Revision | One unsigned byte, exactly `1` |
| Token count | Nonnegative VarInt |
| Name, for each token | String, lowercase ASCII letters, digits and underscore |
| Tooltip translation key | String |
| Players only | Boolean byte |
| Has limit | Boolean byte |
| Limit, if present | Unsigned 32-bit integer, nonzero |
| Sort | One byte: `0` arbitrary, `1` nearest, `2` furthest, `3` random |

Names are unique and cannot replace vanilla's `p`, `a`, `r`, `s`, `e` or `n`. An absent
limit means unbounded. Names fit 32767 ASCII bytes; tooltip keys fit 32767 UTF-16 units,
using the protocol string limit. No trailing bytes are accepted.

## Bounds and publication

The **256 KiB total encoded metadata limit is Horizon's bound**, not a vanilla selector
limit. It includes the revision, count prefix, every string prefix and byte, flags, limits
and sort bytes. Registration accounts for the exact revision 1 encoding before insertion.
The token that would exceed the total is refused with its name and the limit; earlier
records and their budget remain unchanged. This bound is below the 1,048,576-byte
clientbound custom-payload fallback cap. The receiver also refuses an oversized revision 1
snapshot before allocating token records.

The receiver validates a complete snapshot before replacing its command registry metadata.
Starting another connection clears the previous server's tokens. A client registry rebuild
keeps the current server's token snapshot rather than substituting locally installed mods.

## Revision handling

The server emits revision 1 only. There is no unversioned format, older-format fallback or
alias channel. An unknown revision disables selector metadata for that connection, clears
any prior token snapshot and emits one log line naming the unsupported revision. Further
selector metadata for that connection is ignored, including a later revision 1 snapshot.
A new connection starts with revision support enabled again. Unknown revisions do not
panic or disconnect the client; malformed revision 1 data is a protocol error.
