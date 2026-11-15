//! Interaction fixture protocol, shared with the native gameplay listener.
//! Control data: [use_block, use_item, before, block, after, right_click]. Switches are
//! Default/True/False = 0/1/2; answers are Continue/Pass/Fail/Success/
//! SuccessServer/Consume/TryWithEmptyHand = 0/1/2/3/4/5/6.
use mod_api::{Flow, Hand, HostSide, InteractionResult, LogicalSide, TriState, UsePhase};
use std::sync::Mutex;

static STATE: Mutex<([u8; 6], Vec<u8>)> = Mutex::new(([0; 6], Vec::new()));

fn answer(value: u8) -> Option<InteractionResult> {
    Some(match value {
        0 => return None,
        1 => InteractionResult::Pass,
        2 => InteractionResult::Fail,
        3 => InteractionResult::Success,
        4 => InteractionResult::SuccessServer,
        5 => InteractionResult::Consume,
        6 => InteractionResult::TryWithEmptyHand,
        _ => panic!("invalid fixture answer"),
    })
}

fn switch(value: u8) -> TriState {
    match value {
        0 => TriState::Default,
        1 => TriState::True,
        2 => TriState::False,
        _ => panic!("invalid fixture switch"),
    }
}

fn record(trace: &mut Vec<u8>, phase: u8, hand: Hand, side: LogicalSide, empty: bool) {
    trace.extend([
        phase,
        u8::from(hand == Hand::Off),
        u8::from(side == LogicalSide::Client),
        u8::from(empty),
    ]);
}

pub fn right_click_block(_: HostSide<'_>, event: &mut mod_api::RightClickBlockEvent<'_>) -> Flow {
    let mut state = STATE.lock().unwrap();
    *event.use_block = switch(state.0[0]);
    *event.use_item = switch(state.0[1]);
    record(
        &mut state.1,
        0,
        event.click.hand,
        event.click.side,
        event.click.stack.is_none(),
    );
    if let Some(answer) = answer(state.0[5]) {
        *event.cancellation_result = answer;
        Flow::Cancel
    } else {
        Flow::Continue
    }
}

pub fn use_item_on_block(_: HostSide<'_>, event: &mut mod_api::UseItemOnBlockEvent<'_>) -> Flow {
    let phase = match event.phase {
        UsePhase::ItemBeforeBlock => 1,
        UsePhase::Block => 2,
        UsePhase::ItemAfterBlock => 3,
        _ => unreachable!("unknown fixture phase"),
    };
    let mut state = STATE.lock().unwrap();
    record(
        &mut state.1,
        phase,
        event.click.hand,
        event.click.side,
        event.click.stack.is_none(),
    );
    let Some(answer) = answer(state.0[phase as usize + 1]) else {
        return Flow::Continue;
    };
    *event.cancellation_result = answer;
    Flow::Cancel
}

pub struct Control;
#[mod_api::horizon_part]
impl mod_api::BlockPart for Control {}
#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Control {
    fn block_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        let mut state = STATE.lock().unwrap();
        if event.entity.data.len() == 6 {
            // A malformed table is left in place, so the test fails on its
            // own assertion with the case named, not with a guest trap.
            let Some(controls) = controls(&event.entity.data) else {
                return;
            };
            state.0 = controls;
            state.1.clear();
            event.entity.data.clear();
        } else {
            event.entity.data = state.1.iter().map(|&byte| i32::from(byte)).collect();
        }
    }
}

/// Two switches (0..=2), then four answers (0..=6); anything else is refused.
fn controls(data: &[i32]) -> Option<[u8; 6]> {
    let mut controls = [0; 6];
    for (index, (slot, value)) in controls.iter_mut().zip(data).enumerate() {
        let limit = if index < 2 { 2 } else { 6 };
        *slot = u8::try_from(*value).ok().filter(|v| *v <= limit)?;
    }
    Some(controls)
}

pub fn register(registrar: &mut dyn mod_api::Registrar) {
    let kind = registrar
        .block_entity_type("interaction_control")
        .blocks(&[])
        .build();
    registrar.block_entity_part(&kind, Control).unwrap();
}
