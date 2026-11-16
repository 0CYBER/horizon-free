use super::*;
use crate::{
    IPayloadContext, PayloadAccepted, PayloadBytes, PayloadCodec, PayloadContextError,
    PayloadHandleError, PayloadSendError, PayloadToken, PayloadWork, PayloadWorkStatus, PlayerRef,
    Text, VarInt, payload_schema, typed_payload_version,
};
use std::sync::Mutex;

fn frozen(registrar: &PartRegistrar) -> mod_vocab::__private::game::Payloads {
    registrar
        .parts
        .payloads
        .as_ref()
        .unwrap()
        .as_ref()
        .clone()
        .into()
}

#[derive(Debug, PartialEq, PayloadCodec)]
struct Counter {
    counter: VarInt,
}

#[derive(Debug, PartialEq, PayloadCodec)]
struct Other {
    counter: i32,
}

fn ignore<T>(_: T, _: &mut dyn IPayloadContext) -> Result<(), String> {
    Ok(())
}

fn with_token(token: &PayloadToken, change: impl FnOnce(&mut PayloadToken)) -> PayloadToken {
    let mut token = token.clone();
    change(&mut token);
    token
}

#[test]
fn a_token_is_accepted_only_as_the_registrar_handed_it_out() {
    let mut registrar = PartRegistrar::new("test");
    let (up, (both_up, both_down)) = {
        let r = &mut registrar as &mut dyn Registrar;
        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let up = view.play_to_server_typed(PayloadId("test:up".into()), ignore::<Counter>);
        let both = view.common_bidirectional_typed(
            PayloadId("test:both".into()),
            ignore::<Counter>,
            ignore::<Counter>,
        );
        (up, both)
    };
    assert!(registrar.refused.is_empty(), "{:?}", registrar.refused);
    let table = frozen(&registrar);
    let up_declaration = table
        .declarations()
        .find(|declaration| declaration.id.0 == "test:up")
        .unwrap();
    assert_eq!(
        up_declaration.version,
        typed_payload_version("1", payload_schema::<Counter>())
    );
    for token in [up.token(), both_up.token(), both_down.token()] {
        assert_eq!(table.check_token(token), Ok(()));
    }
    let stale = |token: PayloadToken| {
        assert_eq!(
            table.check_token(&token),
            Err(PayloadSendError::StaleToken(token.id.clone())),
            "{token:?}"
        );
    };
    // Forged: another declaration's slot, a slot nobody has, another load.
    stale(with_token(up.token(), |t| t.slot = both_up.token().slot));
    stale(with_token(up.token(), |t| t.slot = 7));
    stale(with_token(up.token(), |t| t.load += 1));
    // Another value type, another direction, another protocol.
    stale(with_token(up.token(), |t| {
        t.schema = payload_schema::<Other>()
    }));
    stale(with_token(up.token(), |t| {
        t.direction = PayloadDirection::ToClient
    }));
    stale(with_token(up.token(), |t| {
        t.direction = PayloadDirection::Bidirectional
    }));
    stale(with_token(both_up.token(), |t| {
        t.protocol = PayloadProtocol::Play
    }));

    // The same registration in a later load hands out a token of its own.
    let mut later = PartRegistrar::new("test");
    let again = (&mut later as &mut dyn Registrar)
        .payload_handlers()
        .registrar("1")
        .play_to_server_typed(PayloadId("test:up".into()), ignore::<Counter>);
    assert_eq!(frozen(&later).check_token(again.token()), Ok(()));
    stale(again.token().clone());
}

#[test]
fn a_second_owner_of_an_id_is_refused_naming_the_first() {
    let mut registrar = PartRegistrar::new("test");
    let r = &mut registrar as &mut dyn Registrar;
    r.payload_handlers()
        .registrar("1")
        .play_to_server_typed(PayloadId("test:up".into()), ignore::<Counter>);
    let refused = r
        .payload_handlers()
        .registrar("2")
        .play_to_server_typed(PayloadId("test:up".into()), ignore::<Other>);
    assert_eq!(registrar.refused.len(), 1);
    let refusal = &registrar.refused[0];
    for name in ["test:up", "already registered by test"] {
        assert!(refusal.contains(name), "{refusal}");
    }
    // The refused registration's token is for no load, so it never sends.
    assert_eq!(refused.token().load, 0);

    // A raw version cannot pose as a typed one.
    (&mut registrar as &mut dyn Registrar)
        .payload_handlers()
        .registrar("1+payload-schema.0000000000000001")
        .play_to_server(PayloadId("test:raw".into()), Arc::new(Ignore));
    assert_eq!(registrar.refused.len(), 2);
    assert!(registrar.refused[1].contains("typed schema"));
}

struct Ignore;
impl IPayloadHandler for Ignore {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        Ok(())
    }
}

/// Decoding runs before any context capability: a stand-in whose capabilities are never reached.
struct Unreached;
impl IPayloadContext for Unreached {
    fn player(&self) -> Result<PlayerRef, PayloadContextError> {
        unreachable!()
    }
    fn flow(&self) -> PayloadDirection {
        unreachable!()
    }
    fn protocol(&self) -> PayloadProtocol {
        unreachable!()
    }
    fn server_host(&mut self) -> Result<&mut dyn crate::ServerHost, PayloadContextError> {
        unreachable!()
    }
    fn client_host(&mut self) -> Result<&mut dyn crate::ClientHost, PayloadContextError> {
        unreachable!()
    }
    fn reply(
        &mut self,
        _: &PayloadId,
        _: PayloadBytes,
    ) -> Result<PayloadAccepted, PayloadSendError> {
        unreachable!()
    }
    fn check_payload_token(&self, _: &PayloadToken) -> Result<(), PayloadSendError> {
        unreachable!()
    }
    fn disconnect(&mut self, _: Text) {
        unreachable!()
    }
    fn enqueue_work(
        &mut self,
        _: Arc<dyn PayloadWork>,
    ) -> Result<PayloadWorkStatus, PayloadContextError> {
        unreachable!()
    }
}

#[derive(Debug, PartialEq, PayloadCodec)]
struct Labelled {
    counter: VarInt,
    labels: Vec<String>,
    chain: Chain,
}

#[derive(Debug, PartialEq, PayloadCodec)]
struct Chain(Option<Box<Chain>>);

#[test]
fn a_malformed_body_is_refused_by_name_before_the_handler_runs() {
    static SEEN: Mutex<Vec<i32>> = Mutex::new(Vec::new());
    let mut registrar = PartRegistrar::new("test");
    (&mut registrar as &mut dyn Registrar)
        .payload_handlers()
        .registrar("1")
        .play_to_server_typed(
            PayloadId("test:labelled".into()),
            |value: Labelled, _: &mut dyn IPayloadContext| {
                SEEN.lock().unwrap().push(value.counter.0);
                Ok(())
            },
        );
    let receiver = frozen(&registrar)
        .handler(
            "test:labelled",
            PayloadProtocol::Play,
            PayloadDirection::ToServer,
        )
        .unwrap();
    let nested = |levels: usize| {
        let mut body = vec![7, 0];
        body.extend(std::iter::repeat_n(1, levels));
        body.push(0);
        body
    };
    for (body, error) in [
        // A label count of -1.
        (vec![7, 0xff, 0xff, 0xff, 0xff, 0x0f], "NegativeLength"),
        // 4097 labels, one past the list bound, before any is reserved.
        (vec![7, 0x81, 0x20], "LengthOverLimit"),
        // 33 boxed options: nesting level 65.
        (nested(33), "TooDeep"),
        (vec![7, 0, 0, 0xaa], "Trailing"),
    ] {
        let refused = receiver
            .handle(PayloadBytes(body), &mut Unreached)
            .unwrap_err();
        let PayloadHandleError::Failed(refused) = refused else {
            panic!("expected a failed decode, got {refused:?}");
        };
        assert!(refused.contains("test:labelled"), "{refused}");
        assert!(refused.contains(error), "{refused}");
    }
    assert!(SEEN.lock().unwrap().is_empty());
    // Level 64 is the deepest a value goes.
    receiver
        .handle(PayloadBytes(nested(32)), &mut Unreached)
        .unwrap();
    receiver
        .handle(PayloadBytes(vec![7, 0, 0]), &mut Unreached)
        .unwrap();
    assert_eq!(*SEEN.lock().unwrap(), [7, 7]);
}

#[test]
fn a_table_made_by_a_raw_channel_is_still_a_load_of_its_own() {
    let register = || {
        let mut registrar = PartRegistrar::new("test");
        let r = &mut registrar as &mut dyn Registrar;
        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        view.play_raw(PayloadId("test:raw".into()), None);
        let token = view.play_to_server_typed::<Counter>(PayloadId("test:up".into()), ignore);
        (registrar, token)
    };
    let (first, first_token) = register();
    let (second, second_token) = register();
    assert_ne!(first_token.token().load, 0);
    assert_eq!(frozen(&first).check_token(first_token.token()), Ok(()));
    assert_eq!(frozen(&second).check_token(second_token.token()), Ok(()));
    for (table, token) in [(&first, &second_token), (&second, &first_token)] {
        assert_eq!(
            frozen(table).check_token(token.token()),
            Err(PayloadSendError::StaleToken(token.token().id.clone()))
        );
    }
}

#[test]
fn a_typed_id_has_one_declaration_in_one_protocol() {
    let mut registrar = PartRegistrar::new("test");
    let r = &mut registrar as &mut dyn Registrar;
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1");
    view.configuration_to_server_typed::<Other>(PayloadId("test:phased".into()), ignore);
    let play = view.play_to_server_typed::<Counter>(PayloadId("test:phased".into()), ignore);
    // A raw declaration in another protocol of a typed id is refused too.
    view.play_to_server(PayloadId("test:phased".into()), Arc::new(Ignore));
    assert_eq!(registrar.refused.len(), 2, "{:?}", registrar.refused);
    for refusal in &registrar.refused {
        assert!(refusal.contains("one declaration"), "{refusal}");
    }
    assert_eq!(play.token().load, 0);
}

#[test]
fn a_callback_that_panicked_is_never_invoked_again() {
    static CALLS: Mutex<u32> = Mutex::new(0);
    let mut registrar = PartRegistrar::new("test");
    (&mut registrar as &mut dyn Registrar)
        .payload_handlers()
        .registrar("1")
        .play_to_server_typed::<Counter>(PayloadId("test:panics".into()), |_, _| {
            *CALLS.lock().unwrap() += 1;
            panic!("the handler panics");
        });
    let table = frozen(&registrar);
    let receiver = || {
        table
            .handler(
                "test:panics",
                PayloadProtocol::Play,
                PayloadDirection::ToServer,
            )
            .unwrap()
    };
    let first = receiver();
    assert!(first.enabled());
    let error = first
        .handle(PayloadBytes(vec![7]), &mut Unreached)
        .unwrap_err();
    let PayloadHandleError::Failed(error) = error else {
        panic!("expected a failed callback, got {error:?}");
    };
    assert!(error.contains("panicked"), "{error}");
    // Disabled for this connection and for one that resolves the callback later.
    for receiver in [first, receiver()] {
        assert!(!receiver.enabled());
        assert_eq!(
            receiver.handle(PayloadBytes(vec![7]), &mut Unreached),
            Err(PayloadHandleError::Failed(
                "payload owner is disabled".into()
            ))
        );
    }
    assert_eq!(*CALLS.lock().unwrap(), 1);
}
