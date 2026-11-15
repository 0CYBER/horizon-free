use super::*;
use mod_api::{IPayloadContext, IPayloadHandler, PayloadBytes, PayloadHandleError};

/// Any context access would mean the refused callback had begun staging effects.
struct UnusedContext;
impl IPayloadContext for UnusedContext {
    fn player(&self) -> Result<mod_api::PlayerRef, mod_api::PayloadContextError> {
        panic!("busy payload accessed its player")
    }
    fn flow(&self) -> mod_api::PayloadDirection {
        panic!("busy payload accessed its direction")
    }
    fn protocol(&self) -> mod_api::PayloadProtocol {
        panic!("busy payload accessed its protocol")
    }
    fn server_host(
        &mut self,
    ) -> Result<&mut dyn mod_api::ServerHost, mod_api::PayloadContextError> {
        panic!("busy payload accessed its server")
    }
    fn client_host(
        &mut self,
    ) -> Result<&mut dyn mod_api::ClientHost, mod_api::PayloadContextError> {
        panic!("busy payload accessed its client")
    }
    fn reply(
        &mut self,
        _: &mod_api::PayloadId,
        _: PayloadBytes,
    ) -> Result<mod_api::PayloadAccepted, mod_api::PayloadSendError> {
        panic!("busy payload staged a reply")
    }
    fn disconnect(&mut self, _: mod_api::Text) {
        panic!("busy payload staged a disconnect")
    }
    fn enqueue_work(
        &mut self,
        _: Arc<dyn mod_api::PayloadWork>,
    ) -> Result<mod_api::PayloadWorkStatus, mod_api::PayloadContextError> {
        panic!("busy payload staged work")
    }
}

#[test]
fn actual_guest_contention_returns_original_payload_without_encoding_or_waiting() {
    crate::test_stack::on_guest_stack(|| {
        let random_id = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let holder_export = crate::boundary::export_id("PayloadWork", "run").unwrap();
        let mut success = Writer::new();
        success.put(&Result::<(), PayloadHandleError>::Ok(()));
        let data = success
            .bytes()
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect::<String>();
        let reply = (4096u64 << 32) | success.bytes().len() as u64;
        let bytes = guest_with_exports(
            *mod_api::wire::LAYOUT,
            &format!(
                "local.get 0 i32.const {holder_export} i32.eq if (result i64) \
                 i32.const {random_id} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host drop \
                 i64.const 8796093022216 else i64.const {reply} end"
            ),
            &format!("(data (i32.const 4096) \"{data}\")"),
        );
        let module = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "test",
                module: &bytes,
            }],
            None,
        )
        .unwrap()
        .remove(0);
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let holding = module.clone();
        let holder_thread = std::thread::Builder::new()
            .name("payload-holder".into())
            .spawn(move || {
                crate::test_stack::on_guest_stack(move || {
                    let mut random = Random(|| {
                        let current = std::thread::current();
                        let thread = current
                            .name()
                            .map(|name| format!("'{name}'"))
                            .unwrap_or_else(|| format!("{:?}", current.id()));
                        entered.send(thread).unwrap();
                        released.recv_timeout(Duration::from_secs(10)).unwrap();
                        7
                    });
                    let mut frame = Frame::default();
                    frame.set_structure_random_mut(&mut random);
                    holding.call(
                        CallClass::Load,
                        frame,
                        holder_export,
                        None,
                        |_| {},
                        |r| r.take::<i64>(),
                    )
                })
            })
            .unwrap();
        let observed_holder_thread = ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let contender_module = module.clone();
        let handler = Arc::new(WasmHandler::new(&module, Id(7)));
        let contender_handler = handler.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let contender = std::thread::spawn(move || {
            crate::test_stack::on_guest_stack(|| {
                // The acquisition check runs before input serialization, even for a real
                // guest paused in a host import with its store lent out.
                assert!(
                    contender_module
                        .downgrade()
                        .call_payload(
                            Frame::default(),
                            holder_export,
                            None,
                            |_| panic!("serialized before checking group contention"),
                            |r| r.take::<()>(),
                        )
                        .is_err()
                );
                let handler = contender_handler;
                let mut original = Vec::with_capacity(8192);
                original.extend_from_slice(&[1, 3, 5, 7]);
                let pointer = original.as_ptr() as usize;
                let capacity = original.capacity();
                let mut payload = PayloadBytes(original);
                let mut previous: Option<mod_api::PayloadBusy> = None;
                for _ in 0..2 {
                    let PayloadHandleError::Busy {
                        payload: returned,
                        holder,
                    } = handler.handle(payload, &mut UnusedContext).unwrap_err()
                    else {
                        panic!("contention became application failure");
                    };
                    assert_eq!(returned.0.as_ptr() as usize, pointer);
                    assert_eq!(returned.0.capacity(), capacity);
                    assert_eq!(returned.0, [1, 3, 5, 7]);
                    assert_eq!(&*holder.owner, "test");
                    assert_eq!(&*holder.export, "PayloadWork::run");
                    assert_eq!(&*holder.thread, observed_holder_thread);
                    let current = contender_module.running.holder.lock().unwrap();
                    assert!(
                        holder.held_millis as u128
                            <= current.as_ref().unwrap().since.elapsed().as_millis()
                    );
                    drop(current);
                    if let Some(previous) = previous {
                        assert!(Arc::ptr_eq(&previous.owner, &holder.owner));
                        assert!(Arc::ptr_eq(&previous.export, &holder.export));
                        assert!(Arc::ptr_eq(&previous.thread, &holder.thread));
                        assert!(previous.held_millis <= holder.held_millis);
                    }
                    previous = Some(holder);
                    payload = returned;
                }

                // Enter another actual guest, then cross into the occupied group from
                // its native import. This must refuse, not return a retryable Busy.
                let nested = super::module(&format!(
                    "i32.const {random_id} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host drop i64.const 8796093022216"
                ));
                let mut nested_refused = false;
                let mut random = Random(|| {
                    nested_refused = matches!(
                        handler.handle(PayloadBytes(vec![9]), &mut UnusedContext),
                        Err(PayloadHandleError::Failed(reason)) if reason.contains("held by mod test")
                    );
                    7
                });
                let mut frame = Frame::default();
                frame.set_structure_random_mut(&mut random);
                assert_eq!(
                    nested.call(CallClass::Load, frame, 0, None, |_| {}, |r| r.take::<i64>()),
                    Some(7)
                );
                assert!(nested_refused);
                send.send(payload).unwrap();
            })
        });
        // The holder is released only after the contender has answered. This is a
        // synchronization proof, not a sleep followed by an elapsed-time guess.
        let answer = receive.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        contender.join().unwrap();
        assert_eq!(holder_thread.join().unwrap(), Some(7));
        let payload = answer.expect("payload waited on the occupied guest");
        assert_eq!(handler.handle(payload, &mut UnusedContext), Ok(()));
        assert!(!module.is_stopped());
    })
}

#[test]
fn payload_trap_reports_failed_instead_of_committing_an_empty_success() {
    crate::test_stack::on_guest_stack(|| {
        let module = module("unreachable");
        let error = WasmHandler::new(&module, Id(7))
            .handle(PayloadBytes(vec![1]), &mut UnusedContext)
            .unwrap_err();
        assert!(
            matches!(error, PayloadHandleError::Failed(reason) if reason.contains("unreachable"))
        );
        assert!(module.is_stopped());
    })
}

#[test]
fn guest_returned_busy_also_preserves_the_original_host_allocation() {
    crate::test_stack::on_guest_stack(|| {
        let mut answer = Writer::new();
        answer.put(&Result::<(), PayloadHandleError>::Err(
            PayloadHandleError::Busy {
                payload: PayloadBytes(vec![99]),
                holder: mod_api::PayloadBusy {
                    owner: "guest".into(),
                    export: "handler".into(),
                    thread: "guest-thread".into(),
                    held_millis: 4,
                },
            },
        ));
        let data = answer
            .bytes()
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect::<String>();
        let reply = (4096u64 << 32) | answer.bytes().len() as u64;
        let bytes = guest_with_exports(
            *mod_api::wire::LAYOUT,
            &format!("i64.const {reply}"),
            &format!("(data (i32.const 4096) \"{data}\")"),
        );
        let module = WasmMod::instantiate(
            &engine().unwrap(),
            DEFAULT_GROUP_MEMORY_MIB,
            &[Member {
                name: "test",
                module: &bytes,
            }],
            None,
        )
        .unwrap()
        .remove(0);
        let mut original = Vec::with_capacity(1024);
        original.extend_from_slice(&[2, 4, 6]);
        let pointer = original.as_ptr();
        let capacity = original.capacity();
        let PayloadHandleError::Busy { payload, holder } = WasmHandler::new(&module, Id(7))
            .handle(PayloadBytes(original), &mut UnusedContext)
            .unwrap_err()
        else {
            panic!("guest Busy became another result");
        };
        assert_eq!(payload.0.as_ptr(), pointer);
        assert_eq!(payload.0.capacity(), capacity);
        assert_eq!(payload.0, [2, 4, 6]);
        assert_eq!(&*holder.owner, "guest");
        assert_eq!(holder.held_millis, 4);
    })
}
