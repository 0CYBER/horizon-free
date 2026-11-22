# Client networking

Call `host.disconnect(reason)` through `ClientHost` to leave the joined server or world at the end of the frame, outside hooks. Pass `Some(Text)` to show a disconnect reason or `None` for the normal leave flow. The disconnect screen renders the reason in the player's language, like other translatable text: a translatable component shows its translated text and a plain-text reason is unchanged. The call returns `Result<(), DisconnectError>`: `Unavailable` means no world is joined, it is closing, or a save is pending, and `Text` reports an invalid reason (a malformed or too-deep component is refused by that name and nothing changes). A pending request belongs to its current world epoch and cannot leave a later world. Server code continues to use the separate player disconnect operation.

## Typed payloads

A typed payload sends a value of your own type on a registered channel. Its body is the value's
`PayloadCodec` encoding (vanilla's network encoding), with nothing around it, so a peer reads it
as an ordinary registered payload.

Derive `PayloadCodec` for the value, register it in `Hooks::register`, and keep the token the
registrar returns:

```rust,ignore
use mod_api::*;
use std::sync::OnceLock;

#[derive(PayloadCodec)]
struct Counter {
    counter: VarInt,
}

static REPLY: OnceLock<ToClient<Counter>> = OnceLock::new();

fn register(r: &mut dyn Registrar) {
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1");
    // The server answers each counter with the next one.
    view.play_to_server_typed::<Counter>(PayloadId("example:count".into()), |value, cx| {
        let counter = value.counter.0.checked_add(1).ok_or("counter is too large")?;
        let next = Counter { counter: VarInt(counter) };
        REPLY.get().unwrap().reply(cx, &next).map(drop).map_err(|e| format!("{e:?}"))
    });
    let reply = view.play_to_client_typed::<Counter>(
        PayloadId("example:count_reply".into()),
        |value, _| Ok(log::info!("server counted {}", value.counter.0)),
    );
    let _ = REPLY.set(reply);
}
```

- **Registration.** `play_to_server_typed`, `play_to_client_typed` and `play_bidirectional_typed`,
  and their `configuration_*` and `common_*` forms, sit beside the raw methods on the same
  registrar and take its version, `optional()` and `executes_on(...)`. The handler receives the
  decoded value and the connection's `IPayloadContext`. A typed id has one declaration: the same
  id in another protocol refuses load.
- **Tokens.** Registration returns a `ToServer<T>` (client to server) or `ToClient<T>` (server to
  client), both for a bidirectional one. Keep it and send with it; it has a Wire form, so a wasm
  mod keeps it the same way. A token grants nothing on its own: each send checks it.
- **Sending and replying.** `ToServer::send(host, &value)` sends from the client
  (`ClientHost::send_payload_to_server`), `ToClient::send(host, player, &value)` from the server
  (`PlayerRef::send_payload`), and `reply(cx, &value)` on either answers on the connection a
  handler received on. They return the raw sends' results: `Queued` means accepted locally.
  Sending the wrong direction does not compile.
- **From a shared host.** Screen hooks are given `&dyn ClientHost`, which cannot call the
  `&mut` `send_payload_to_server`. `ClientHost::queue_payload_to_server(&self, id, bytes)`
  validates the bytes now and stages them in order; the game publishes them on a later client
  tick through the same root commit. Staged inside a transaction, it goes with that
  transaction: if the handler fails and rolls back, nothing is sent. `Queued` means staged, not delivered. A typed token's
  `ToServer::send` still needs `&mut dyn ClientHost`, so stage the raw bytes from a screen hook
  only for a declaration whose encoding you control.
- **Schema negotiation.** The derive gives the value type a schema fingerprint (field types in
  order, variant positions; not names), and it is part of the negotiated version. A peer whose
  type has another shape is refused at join when the payload is required; when both sides
  declared it `optional()`, it is absent and a send returns `UnsupportedPeer`.
- **Named errors.** `PayloadSendError::StaleToken` for a token the registrar did not hand out
  as it claims (forged, from another load, read as another type, or its mod off);
  `EncodeFailed` for a value the peer could not decode (a bound or the allocation budget),
  before anything is queued; `WrongProtocol` for a send in a phase the id is not declared in. A
  received body that is not one value of the type closes the session, naming the mod, and the
  handler does not run. A handler that panics is disabled: its transaction rolls back, nothing it
  staged is sent, the session closes, and it is not called again.

## Raw play packets

On the server, `host.player_send_packet(player, ClientboundPlayPacket::new(id, body))` sends one clientbound play packet to one player as it is: `body` is the vanilla packet body, without the packet id, length prefix or compression framing. The body must decode whole as the named packet, or the call returns `Result<(), SendPacketError>` with `InvalidBody` and nothing is sent. `UnknownPacket` names a packet that is not clientbound play, `PayloadTooLarge` a custom payload past vanilla's limit, and `NotConnected` a stale, offline or fake player and any call from a client. Nothing on the server changes; a custom payload on a `horizon:` channel is refused.
