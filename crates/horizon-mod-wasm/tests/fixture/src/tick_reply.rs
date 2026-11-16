use mod_api::*;

pub struct Ticker;
#[mod_api::horizon_part]
impl BlockPart for Ticker {}
#[mod_api::horizon_part]
impl BlockEntityPart for Ticker {
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        let mode = event.entity.data[0];
        assert_eq!(*event.reply, BlockEntityTickReply::Changed);
        if mode == 1 {
            event.entity.data[1] += 1;
            return;
        }
        if mode == 5 {
            event.entity.data[1] = 7;
            event.entity.nbt.insert("first".into(), Tag::Int(7));
            return;
        }
        if mode == 10 {
            let entity = host.entity(event.entity.data[3] as u32).unwrap();
            event.entity.items[0] = Some(host.item_entity(entity).unwrap().stack(host).unwrap());
            return;
        }
        *event.reply = BlockEntityTickReply::Unchanged;
        if mode == 8 {
            // A genuinely untouched Unchanged listener must need no checkpoint copy.
            return;
        }
        // Edits are deliberately discarded, even when a later Part accepts its own edits.
        event.entity.data[1] = 999;
        event.entity.nbt.insert("discarded".into(), Tag::Int(999));
        if mode == 2 {
            let level = host.level("minecraft:overworld").unwrap();
            let replacement = host.block_state_id("minecraft:chest").unwrap();
            assert!(
                level
                    .set_block(
                        host,
                        event.pos,
                        replacement,
                        BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                    )
                    .unwrap()
            );
        } else if mode == 3 {
            let level = host.level("minecraft:overworld").unwrap();
            let sink = host.block_entity(level, [2, 64, 1]).unwrap();
            let value = sink.data_slots(host).unwrap()[1];
            assert!(sink.set_data_slot(host, 1, value + 1).unwrap());
        } else if mode == 6 || mode == 7 {
            let entity = host.entity(event.entity.data[3] as u32).unwrap();
            let item = host.item_entity(entity).unwrap();
            let mut stack = item.stack(host).unwrap();
            assert_eq!(stack.count(), 5);
            stack.set_count(2).unwrap();
            if mode == 6 {
                let aborted: Result<(), ()> = host.transaction(|host, _| {
                    assert_eq!(item.set_stack(host, stack), Ok(true));
                    assert_eq!(item.stack(host).unwrap().count(), 2);
                    Err(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(item.stack(host).unwrap().count(), 5);
            } else {
                assert_eq!(item.set_stack(host, stack), Ok(true));
                assert_eq!(item.stack(host).unwrap().count(), 2);
            }
        }
    }
}

pub struct Later;
#[mod_api::horizon_part]
impl BlockPart for Later {}
#[mod_api::horizon_part]
impl BlockEntityPart for Later {
    fn block_entity_tick(&self, _: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        // Every Part starts with Changed, independent of the preceding Part's reply.
        assert_eq!(*event.reply, BlockEntityTickReply::Changed);
        match event.entity.data[0] {
            4 | 9 => {
                assert_eq!(event.entity.data[1], 0);
                assert!(event.entity.nbt.is_empty());
                event.entity.data[2] = 42;
                if event.entity.data[0] == 4 {
                    *event.reply = BlockEntityTickReply::Changed;
                }
            }
            5 => {
                assert_eq!(event.entity.data[1], 7);
                assert_eq!(event.entity.nbt.get(&"first".into()), Some(&Tag::Int(7)));
                event.entity.data[2] = 999;
                event.entity.nbt.clear();
                *event.reply = BlockEntityTickReply::Unchanged;
            }
            _ => *event.reply = BlockEntityTickReply::Unchanged,
        }
    }
}

pub fn register(registrar: &mut dyn Registrar) {
    let kind = registrar
        .block_entity_type("tick_reply")
        .blocks(&[])
        .build();
    registrar.block_entity_part(&kind, Ticker).unwrap();
    registrar.block_entity_part(&kind, Later).unwrap();
}
