//! Guest half of the Game-backed payload proof. The native test owns the receipt channel.
use mod_api::*;
use std::sync::Arc;

pub fn register(registrar: &mut dyn Registrar) {
    registrar
        .payload_handlers()
        .registrar("1")
        .executes_on(HandlerThread::Network)
        .configuration_bidirectional(
            PayloadId("wasm_fixture:configuration".into()),
            Arc::new(Configuration),
            Some(Arc::new(ConfigurationReceipt)),
        );
    registrar
        .payload_handlers()
        .registrar("1")
        .executes_on(HandlerThread::Network)
        .play_to_server(
            PayloadId("wasm_fixture:network_trap".into()),
            Arc::new(NetworkTrap),
        );
    registrar
        .payload_handlers()
        .registrar("1")
        .executes_on(HandlerThread::Network)
        .play_bidirectional(
            PayloadId("wasm_fixture:measure".into()),
            Arc::new(Echo),
            Some(Arc::new(Echo)),
        );
    registrar
        .payload_handlers()
        .registrar("1")
        .executes_on(HandlerThread::Network)
        .play_bidirectional(
            PayloadId("wasm_fixture:busy".into()),
            Arc::new(BusyEcho),
            Some(Arc::new(BusyEcho)),
        );
    registrar
        .payload_handlers()
        .registrar("1")
        .executes_on(HandlerThread::Network)
        .play_to_server(
            PayloadId("wasm_fixture:pressure".into()),
            Arc::new(Pressure),
        );
    for (channel, thread) in [
        ("wasm_fixture:game_main", HandlerThread::Main),
        ("wasm_fixture:game_network", HandlerThread::Network),
    ] {
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(thread)
            .play_to_server(PayloadId(channel.into()), Arc::new(Handler(thread)));
    }
}

static CONFIGURATION_CALLS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

struct Configuration;
impl IPayloadHandler for Configuration {
    fn handle(
        &self,
        mut bytes: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        assert_eq!(cx.protocol(), PayloadProtocol::Configuration);
        assert_eq!(cx.flow(), PayloadDirection::ToServer);
        assert!(matches!(cx.player(), Err(PayloadContextError::NoPlayer)));
        assert!(matches!(
            cx.server_host(),
            Err(PayloadContextError::WrongThread)
        ));
        assert!(matches!(
            cx.client_host(),
            Err(PayloadContextError::WrongSide)
        ));
        assert_eq!(bytes.0, [3, 5, 8, 13]);
        bytes.0.reverse();
        for byte in &mut bytes.0 {
            *byte *= 3;
        }
        let call = CONFIGURATION_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        bytes.0.insert(0, call);
        cx.reply(&PayloadId("wasm_fixture:configuration".into()), bytes)
            .map_err(|error| {
                PayloadHandleError::Failed(format!("configuration reply: {error:?}"))
            })?;
        Ok(())
    }
}

struct ConfigurationReceipt;
impl IPayloadHandler for ConfigurationReceipt {
    fn handle(
        &self,
        bytes: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        assert_eq!(cx.protocol(), PayloadProtocol::Configuration);
        assert_eq!(cx.flow(), PayloadDirection::ToClient);
        assert!(matches!(cx.player(), Err(PayloadContextError::NoPlayer)));
        assert_eq!(bytes.0.len(), 5);
        assert_eq!(&bytes.0[1..], &[39, 24, 15, 9]);
        Ok(())
    }
}
struct NetworkTrap;
impl IPayloadHandler for NetworkTrap {
    fn handle(
        &self,
        _: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        cx.enqueue_work(Arc::new(Noop)).unwrap();
        cx.reply(
            &PayloadId("raw_probe:receipt".into()),
            PayloadBytes(vec![0xde, 0xad]),
        )
        .unwrap();
        panic!("NETWORK handler trap after staged work and reply");
    }
}
struct Echo;
impl IPayloadHandler for Echo {
    fn handle(
        &self,
        bytes: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        assert_eq!(bytes.0.len(), 1 << 20);
        echo("wasm_fixture:measure", bytes, cx)
    }
}
struct BusyEcho;
impl IPayloadHandler for BusyEcho {
    fn handle(
        &self,
        bytes: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        echo("wasm_fixture:busy", bytes, cx)
    }
}
fn echo(
    channel: &str,
    bytes: PayloadBytes,
    cx: &mut dyn IPayloadContext,
) -> Result<(), PayloadHandleError> {
    cx.reply(&PayloadId(channel.into()), bytes)
        .map_err(|e| PayloadHandleError::Failed(format!("reply: {e:?}")))?;
    Ok(())
}
struct Pressure;
impl IPayloadHandler for Pressure {
    fn handle(
        &self,
        _: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        assert_eq!(
            cx.enqueue_work(Arc::new(Noop)),
            Err(PayloadContextError::Backpressure)
        );
        Ok(())
    }
}
struct Noop;
impl PayloadWork for Noop {
    fn run(&self, _: &mut dyn IPayloadContext) -> Result<(), String> {
        panic!("refused work must never run")
    }
}
struct Handler(HandlerThread);
impl IPayloadHandler for Handler {
    fn handle(
        &self,
        bytes: PayloadBytes,
        cx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        assert_eq!(cx.protocol(), PayloadProtocol::Play);
        assert_eq!(cx.flow(), PayloadDirection::ToServer);
        let player = cx.player().unwrap();
        assert_ne!(player.uuid(), 0);
        if self.0 == HandlerThread::Network {
            assert!(matches!(
                cx.server_host(),
                Err(PayloadContextError::WrongThread)
            ));
            assert!(matches!(
                cx.client_host(),
                Err(PayloadContextError::WrongSide)
            ));
            cx.enqueue_work(Arc::new(Work(std::sync::Mutex::new(Some(bytes)))))
                .map_err(|e| PayloadHandleError::Failed(format!("enqueue: {e:?}")))?;
            return Ok(());
        }
        finish(bytes, cx).map_err(PayloadHandleError::Failed)
    }
}
struct Work(std::sync::Mutex<Option<PayloadBytes>>);
impl PayloadWork for Work {
    fn run(&self, cx: &mut dyn IPayloadContext) -> Result<(), String> {
        finish(self.0.lock().unwrap().take().unwrap(), cx)
    }
}
fn finish(bytes: PayloadBytes, cx: &mut dyn IPayloadContext) -> Result<(), String> {
    let player = cx.player().unwrap();
    assert!(matches!(
        cx.client_host(),
        Err(PayloadContextError::WrongSide)
    ));
    let host = cx.server_host().unwrap();
    assert!(
        host.item_key(&Identifier::parse("minecraft:stone").unwrap())
            .is_some()
    );
    let living = LivingRef::new(player.entity());
    let before = living.health(host).unwrap();
    assert!(before > 3.0);
    assert!(living.set_health(host, before - 1.0));
    // The parent test observes this actual root-owned write, not a substitute Host.
    // A one-byte request stages a receipt and then traps. Both the receipt and
    // health write must be rolled back by the real executor/Game root.
    if bytes.0.len() == 1 {
        cx.reply(
            &PayloadId("raw_probe:receipt".into()),
            PayloadBytes(vec![0xde, 0xad]),
        )
        .map_err(|e| format!("reply: {e:?}"))?;
        panic!("Game-backed trap after health write and staged receipt");
    }
    assert_eq!(bytes.0.len(), 1 << 20);
    for (i, byte) in bytes.0.iter().enumerate() {
        assert_eq!(*byte, ((i * 31 + 7) % 251) as u8);
    }
    cx.reply(&PayloadId("raw_probe:receipt".into()), bytes)
        .map_err(|e| format!("reply: {e:?}"))?;
    Ok(())
}
