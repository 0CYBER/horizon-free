use super::*;

#[test]
fn contended_nested_worker_refuses_without_stopping_either_group() {
    crate::test_stack::on_guest_stack(|| {
        let import = crate::boundary::import_id("StructureRandom", "next_long").unwrap();
        let body = format!(
            r#"
            local.get 0 i32.eqz
            if (result i64)
                i32.const {import} i32.const 0 i32.const 0 i32.const 2048 i32.const 8 call $host
                drop i64.const 8796093022216
            else
                i64.const 2199023255556
            end"#
        );
        let target = module(&body);
        let source = module(&body);
        let (holding, held) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let target_thread = target.clone();
        let holder = std::thread::spawn(move || {
            crate::test_stack::on_guest_stack(move || {
                let mut random = Random(|| {
                    // The real host import has installed target's loan and still owns
                    // its group mutex before source can attempt its nested Worker.
                    holding.send(()).unwrap();
                    // Disconnect also releases the holder if the coordinator unwinds.
                    let _ = released.recv();
                    7
                });
                let mut frame = Frame::default();
                frame.set_structure_random_mut(&mut random);
                // Native synchronization is not guest work: use the existing long
                // load budget rather than the ordinary 100 ms guest deadline.
                target_thread.call(CallClass::Load, frame, 0, None, |_| {}, |r| r.take::<i64>())
            })
        });
        held.recv().unwrap();

        let (entering, entered) = std::sync::mpsc::channel();
        let (answer, answered) = std::sync::mpsc::channel();
        let target_thread = target.clone();
        let source_thread = source.clone();
        let contender = std::thread::spawn(move || {
            crate::test_stack::on_guest_stack(move || {
                let mut random = Random(|| {
                    // Source's actual import now owns a different group's loan.
                    // Worker dispatch must refuse rather than retain it while waiting
                    // for target's lent store (the prerequisite for a lock cycle).
                    entering.send(()).unwrap();
                    let result = target_thread.try_call(
                        CallMode::Ordinary(CallClass::Worker),
                        Frame::default(),
                        1,
                        None,
                        |w| w.put(&42u32),
                        |r| r.take::<u32>(),
                    );
                    answer.send(matches!(result, Err(Busy::Refused))).unwrap();
                    7
                });
                let mut frame = Frame::default();
                frame.set_structure_random_mut(&mut random);
                source_thread.call(CallClass::Load, frame, 0, None, |_| {}, |r| r.take::<i64>())
            })
        });
        entered.recv().unwrap();
        let refused = answered.recv_timeout(Duration::from_secs(5));

        // Always release and join before asserting. With the old Worker exemption,
        // the bounded receive expires, then releasing target lets the nested call
        // finish; neither version can leave a permanently blocked thread behind.
        let _ = release.send(());
        let holder_result = holder.join();
        let contender_result = contender.join();
        assert_eq!(refused, Ok(true));
        assert_eq!(holder_result.unwrap(), Some(7));
        assert_eq!(contender_result.unwrap(), Some(7));
        assert!(!target.is_stopped());
        assert!(!source.is_stopped());

        // The same Worker entry remains supported once the enclosing loans end.
        assert_eq!(
            target.call(
                CallClass::Worker,
                Frame::default(),
                1,
                None,
                |w| w.put(&42u32),
                |r| r.take::<u32>(),
            ),
            Some(42)
        );
    })
}
