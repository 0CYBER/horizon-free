//! Guest half of the typed payload proof: one bidirectional typed declaration whose server
//! handler checks forged tokens, replies through its own token, or replies and then traps.
use mod_api::*;
use std::sync::Mutex;

#[derive(Debug, PartialEq, PayloadCodec)]
pub struct Counter {
    pub counter: VarInt,
}

/// The same claim read as another value type.
#[derive(Debug, PartialEq, PayloadCodec)]
struct Other(i32);

static DOWN: Mutex<Option<ToClient<Counter>>> = Mutex::new(None);

pub fn register(registrar: &mut dyn Registrar) {
    let (_, down) = registrar
        .payload_handlers()
        .registrar("1")
        .play_bidirectional_typed(
            PayloadId("wasm_fixture:typed".into()),
            server,
            |_: Counter, _: &mut dyn IPayloadContext| Ok(()),
        );
    *DOWN.lock().unwrap() = Some(down);
}

fn reencoded<T: wire::Wire, U: wire::Wire>(value: &T, change: impl FnOnce(&mut PayloadToken)) -> U {
    let mut out = wire::Writer::new();
    wire::Wire::put(value, &mut out);
    let mut token: PayloadToken = wire::Reader::decode(&out.into_bytes()).unwrap();
    change(&mut token);
    let mut out = wire::Writer::new();
    wire::Wire::put(&token, &mut out);
    wire::Reader::decode(&out.into_bytes()).unwrap()
}

fn server(value: Counter, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let down = DOWN.lock().unwrap().clone().ok_or("no token")?;
    let id = PayloadId("wasm_fixture:typed".into());
    // A token from another slot or load, and one read as another type, send nothing.
    for change in [
        (|token: &mut PayloadToken| token.slot = u32::MAX) as fn(&mut PayloadToken),
        |token| token.load = token.load.wrapping_add(1),
    ] {
        let forged: ToClient<Counter> = reencoded(down.token(), change);
        if forged.reply(cx, &Counter { counter: VarInt(0) })
            != Err(PayloadSendError::StaleToken(id.clone()))
        {
            return Err("a forged wasm token was accepted".into());
        }
    }
    let retyped: ToClient<Other> = reencoded(down.token(), |_| {});
    if retyped.reply(cx, &Other(0)) != Err(PayloadSendError::StaleToken(id)) {
        return Err("a retyped wasm token was accepted".into());
    }
    let reply = Counter {
        counter: VarInt(value.counter.0 + 1),
    };
    down.reply(cx, &reply)
        .map_err(|error| format!("typed reply: {error:?}"))?;
    if value.counter.0 == 13 {
        panic!("typed guest handler traps after its reply");
    }
    Ok(())
}
