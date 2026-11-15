# Custom payload discard limits

Part of the raw payload transport work tracked by arismoko/horizon#2882. This change is only the
existing codec boundary, not a new registration or negotiation mechanism.

## Vanilla contract

In Minecraft 26.3, `ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` is 32,767 and
`ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` is 1,048,576. Both configuration
and play pass these values to `DiscardedPayload.codec` for an **unknown id**.
That codec checks the remaining body length before consuming it. The identifier
is not included in the limit. A body exactly at the limit passes; one byte over
fails the session. These are not caps on registered payloads.

The constants were checked against the pinned vanilla classfiles with `javap
-constants -private`, the decompiled packet classes, and REA inspection of the
packet classes and `DiscardedPayload.codec`. No third-party implementation code
is included here.

A registered payload instead has its own codec constraints. On network
connections it must also fit the frame: `frame::MAX_FRAME` is 2,097,151 bytes;
with compression, `frame::MAX_UNCOMPRESSED` is 8,388,608 bytes and the compressed
frame must still fit `MAX_FRAME`. Vanilla in-process connections have no such
frame bound. Packet encoding alone does not know whether a connection is local
or whether it uses compression. This codec change therefore adds no framing
cap to registered payloads. The subsequent connection-aware transport must
return a named send error before writing when a registered mod packet cannot
fit; that deliberate difference from vanilla's encode-failure disconnect is
not implemented by this change. Packet splitting is out of scope.

## Boundary

`packets::custom_payload::uses_registered_payload_codec` is the single seam for
the subsequent transport's frozen registered-codec lookup. For now it recognizes
only existing engine receiving codecs, keyed by phase and direction. No
arbitrary namespace is exempted. Wrong-phase and wrong-direction ids use the
discard fallback. A new mod id is unknown until that transport implements
registration. The configuration clientbound quiet-command and mod-message
advertisements have an empty-body codec; their play bodies are different.

Existing codec constraints remain with their owners:

| Channel | Existing owner and constraints |
| --- | --- |
| `minecraft:brand` | UTF string codec, 32,767 UTF-16 code units, not 32,767 bytes |
| `horizon:quiet_commands` | `quiet`, 32,767 body bytes, 256 lines with 256 characters each |
| `horizon:recipe_book_settings` (configuration) | exact private capability version byte `[1]` |
| `horizon:mod_list` | `mod_list`, 128 entries, 64-character id and version fields on decode |
| `horizon:attachment_sync` | `attachment_sync`, 1,048,576 body bytes |
| `horizon:menu_extra` | `menu_extra`, 32,768 extra-data bytes plus window/position/framing |
| `horizon:anvil_cost` | `anvil_cost`, one VarInt window and one fixed i32 cost, no trailing bytes |
| `horizon:lod` | `lod_net`, fixed offer/request fields; replies at most 524,288 bytes |
| `horizon:block_registry` | `content_identity`, an empty digest query or exactly 32 digest bytes |
| `horizon:recipe_content` | `lan::recipes`, content parts at most 1,047,552 bytes |
| `horizon:recipe_transfer` | `lan::recipes`, request has a VarInt window, a 512-character recipe id and at most one flag byte; reply is empty or a VarInt window and one result byte; no trailing bytes |
| `horizon:recipe_book_settings` | `recipe_book::settings_codec`, 524,288 body bytes |
| `horizon:levels` | `viewed_levels`, configuration both ways: exactly a 2-byte query and a 3-byte answer for a served revision (1 to 3), while another revision's bytes after its revision are ignored; play clientbound only: one level's packets, each of a level-scoped kind (from revision 3 also `Sound` and `SoundEntity`, a protocol error below it) and read exactly by its play codec; a serverbound play payload is refused |

This finite engine-only classification is replaced, not extended, by mod payload
registration. It does not authorize sending to an unnegotiated peer. The current
phase/direction matrix follows the engine's receiving handlers: mod lists are
received only in serverbound configuration, identity digests in configuration
both ways, LOD offers in clientbound configuration and LOD traffic in play,
recipe-book settings and recipe transfers in play both ways, and attachment,
menu, anvil and recipe-content bodies only in clientbound play. Quiet commands
are serverbound play; mod messages are bidirectional play. The brand codec is
common to both phases and directions.

Unknown bodies are bounded before their owned `Vec` is created. Protocol errors
retain structured `PayloadTooLarge` details (id, size and limit), so the public
raw packet send service can preserve its existing named error without copying
or reparsing a rejected body. The old blanket checks after decoding in the LAN
connection were removed: they copied first and incorrectly bounded known ids.
The registered-engine decoder regression table exercises every listed channel
with a hostile oversized body. Fixed-shape decoders reject trailing bytes;
variable-size decoders bound their counts, strings or total body before
allocating decoded fields. This change also adds the missing recipe-content
receive bound matching its encoder, validates identity digest shape, rejects
trailing bytes after LOD offers, and validates the brand string before making
an owned custom body. The network frame remains the outer bound on the packet
buffer preceding these channel-specific decoders.

## Checked encoding

`packet_enum` can select a directional codec without changing shared packet
value constructors. `try_encode` validates before allocating the encoded body.
Trusted `encode` remains infallible and never truncates data. Changing every
configuration/play encoder to return `Result` would force an unrelated API-wide
migration of gameplay packet producers and send methods.

Mod-controlled client sends go through `net::Session::send`, and server sends
through `lan::guest::Guest::send`; both use `try_encode`. Configuration reply and
server configuration send closures also use it, as does the direct LOD sender.
A future mod-controlled producer calling trusted `encode` directly would bypass
the local checked-send contract; the transport's explicit send service must
continue to own that boundary. Receivers always enforce the unknown-id limit,
regardless of which encoder the sender used.
