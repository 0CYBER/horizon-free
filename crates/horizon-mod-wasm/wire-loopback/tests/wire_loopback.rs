//! Exercise the generated proxies and service against the real guest dispatcher, without wasm.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#![allow(dead_code, unused_variables, unused_mut, unsafe_op_in_unsafe_fn)]
use mod_api::wire::{Id, Reader, WireError, Writer, guest};
use mod_api::{Hooks, LoopContext, StructurePiece};
use std::{rc::Rc, sync::Arc};
// One module per file of tests/wire_loopback/, written by build.rs: a new test goes in a file there.
include!(concat!(env!("OUT_DIR"), "/wire_loopback_modules.rs"));
// The guest's one Hooks object, assembled by build.rs from the inherent `impl <Feature>Hooks` of those files.
include!(concat!(env!("OUT_DIR"), "/wire_loopback_hooks.rs"));
mod boundary {
    include!(concat!(env!("OUT_DIR"), "/wire_host_generated.rs"));
}
/// The generated proxies' refusal reason; the loopback refuses only on its own encode errors.
fn wasm_call_refusal() -> String {
    "loopback refused".into()
}
thread_local! {
    static EXPECTED_LOOT_HAND: std::cell::RefCell<Option<mod_api::ItemStack>> = const { std::cell::RefCell::new(None) };
}
#[derive(Clone)]
struct WasmMod {
    group: u8,
    subscribed: &'static [Option<mod_api::HookId>],
    listeners: &'static [mod_api::Listener],
    corrupt: bool,
}
impl std::fmt::Debug for WasmMod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Loopback")
    }
}
#[derive(Debug)]
struct WasmHandler {
    index: Id,
    module: WasmMod,
}
#[derive(Clone, Copy)]
enum CallClass {
    Load,
    Hook,
    Worker,
    Callback,
}
trait NoAnswer {
    fn no_answer() -> Self;
}
impl<T: Default> NoAnswer for T {
    fn no_answer() -> Self {
        Self::default()
    }
}
fn call_refused<E: From<(mod_api::Identifier, String)>>() -> E {
    E::from((
        mod_api::Identifier::parse("test:refused").unwrap(),
        "refused".into(),
    ))
}
impl WasmHandler {
    fn new(module: &WasmMod, index: Id) -> Self {
        Self {
            module: module.clone(),
            index,
        }
    }
    fn belongs_to(&self, module: &WasmMod) -> bool {
        self.module.same_group(module)
    }
    fn subscriptions(&self, owner: &'static str, id: u32) -> &'static [Option<mod_api::HookId>] {
        let names = self
            .module
            .call(
                CallClass::Load,
                boundary::Frame::default(),
                id,
                Some(self.index),
                |_| {},
                |r| r.take(),
            )
            .unwrap();
        boundary::subscriptions_of(owner, names)
    }
}
thread_local! {
    static LOOPBACK_TRANSPORT: std::cell::RefCell<Option<guest::Transport>> = const { std::cell::RefCell::new(None) };
}
struct TransportScope(Option<guest::Transport>);
impl Drop for TransportScope {
    fn drop(&mut self) {
        guest::set_transport(self.0.clone());
        LOOPBACK_TRANSPORT.with(|transport| *transport.borrow_mut() = self.0.take());
    }
}
impl WasmMod {
    fn same_group(&self, other: &Self) -> bool {
        self.group == other.group
    }
    fn mod_id(&self) -> &str {
        if self.group == 0 { "loopback" } else { "other" }
    }
    fn call<R>(
        &self,
        _: CallClass,
        frame: boundary::Frame,
        id: u32,
        handler: Option<Id>,
        encode: impl FnOnce(&mut Writer),
        decode: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>,
    ) -> Option<R> {
        // Each mock Cx models a fresh store; real stores own their guest string table.
        guest::reset_static_strings();
        let mut cx = Box::new(boundary::Cx::default());
        cx.frames.push(frame);
        cx.module = self as *const Self;
        let pointer = &mut *cx as *mut boundary::Cx;
        let _resources = unsafe { boundary::resource_scope(pointer, true) };
        // The pointer remains live until this synchronous call finishes; guest callbacks never retain it.
        // SAFETY: the transport is restored before this synchronous call drops cx.
        let transport = unsafe { host_dispatch::transport(pointer) };
        let previous =
            LOOPBACK_TRANSPORT.with(|active| active.borrow_mut().replace(transport.clone()));
        let _transport = TransportScope(previous);
        guest::set_transport(Some(transport));
        let mut input = Writer::new();
        if let Some(handler) = handler {
            input.put(&handler);
        }
        encode(&mut input);
        if input.error().is_some() {
            return None;
        }
        prepared_equip::record(id, input.bytes());
        let at = guest::input(input.bytes().len());
        unsafe {
            std::ptr::copy_nonoverlapping(input.bytes().as_ptr(), at, input.bytes().len());
        }
        let (at, len) = guest::run(id);
        if len == u32::MAX {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(at, len as usize) };
        let mut forged;
        let bytes = if matches!(APPEND_ATTACK.get(), 1 | 2) {
            forged = bytes.to_vec();
            let at = forged
                .windows(4)
                .position(|b| b == 0x7fc01234u32.to_le_bytes())
                .unwrap();
            if APPEND_ATTACK.get() == 1 {
                forged[at..at + 4].copy_from_slice(&1.0f32.to_le_bytes());
            } else {
                let count = u32::from_le_bytes(forged[at - 8..at - 4].try_into().unwrap());
                forged[at - 8..at - 4].copy_from_slice(&(count - 1).to_le_bytes());
                forged.drain(at - 4..at + 4);
            }
            forged.as_slice()
        } else {
            bytes
        };
        let bytes = if self.corrupt || APPEND_ATTACK.get() == 3 {
            &bytes[..bytes.len().saturating_sub(1)]
        } else {
            bytes
        };
        let invalid;
        let bytes = if piston_push_reaction::UNKNOWN_REACTION.get() {
            // A replacement response is Option<PushReaction>: Some followed by its enum tag.
            let mut out = Writer::new();
            out.put(&true);
            out.put(&u32::MAX);
            invalid = out.into_bytes();
            invalid.as_slice()
        } else {
            bytes
        };
        decode(&mut Reader::new(bytes)).ok()
    }
}
struct Context;
impl LoopContext for Context {
    fn side(&self) -> mod_api::LoopSide {
        mod_api::LoopSide::Server
    }
    fn registry(&self, _: &str) -> Option<Arc<[String]>> {
        None
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
    fn mods(&self) -> mod_api::LoadedMods {
        unreachable!("not used by this fixture")
    }
    fn mod_recipe_property_contains(&self, _: &str, _: mod_api::ItemKey) -> bool {
        false
    }
}
#[unsafe(no_mangle)]
fn __horizon_mod_hooks() -> Box<dyn Hooks> {
    Box::new(Mod)
}
#[derive(Debug)]
struct Piece;
impl StructurePiece for Piece {
    fn bounding_box(&self) -> mod_api::BoundingBox {
        mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6])
    }
    fn beardifier(&self) -> Option<mod_api::PieceBeardifier> {
        Some(mod_api::PieceBeardifier::new(
            mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6]),
            mod_api::TerrainAdjustment::BeardThin,
            2,
        ))
    }
    fn post_process(
        &self,
        _: &mut dyn mod_api::PieceContext,
    ) -> Result<(), mod_api::StructureError> {
        Ok(())
    }
    fn add_additional_save_data(&self, _: &mut mod_api::Compound) {}
}
struct MachineBlock;
impl mod_api::BlockPart for MachineBlock {
    fn block_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[]
    }
}
impl mod_api::BlockEntityPart for MachineBlock {
    fn block_entity_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[Some(mod_api::HookId::BlockEntityTick)]
    }
    fn block_entity(
        &self,
        level: mod_api::LevelSide<'_>,
        event: &mut mod_api::BlockEntityEvent<'_>,
    ) -> Option<mod_api::Compound> {
        assert_eq!(event.block, "test:machine");
        matches!(level, mod_api::LevelSide::Server(_)).then(mod_api::Compound::new)
    }
    fn block_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        if event.entity.data[0] == -1 {
            *event.reply = mod_api::BlockEntityTickReply::Unchanged;
            event.entity.data[0] = 999;
            return;
        }
        event.entity.data[0] += 3;
    }
}
#[test]
fn wire_loopback_machine_tick_uses_checked_snapshot_writeback() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::BlockEntityPart;
    let id = guest::add_handler(vec![guest::view::<dyn mod_api::BlockEntityPart>(Arc::new(
        MachineBlock,
    ))]);
    let mut entity = mod_api::ModBlockEntity::default();
    entity.data = vec![5];
    let identity = entity.id().to_string();
    let born = entity.revision().identity();
    let proxy = WasmHandler {
        index: id,
        module: WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: false,
        },
    };
    proxy.block_entity_tick(
        &mut guest::Guest,
        &mut mod_api::BlockEntityTickEvent {
            pos: [1, 2, 3],
            reply: &mut mod_api::BlockEntityTickReply::Changed,
            entity: mod_api::ModBlockEntityView::new(&mut entity),
        },
    );
    assert_eq!(entity.data, vec![8]);
    assert_eq!(entity.id(), identity);
    assert_eq!(entity.revision().identity(), born);
    let before = entity.revision().changes();
    let bad = WasmHandler {
        index: id,
        module: WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: true,
        },
    };
    bad.block_entity_tick(
        &mut guest::Guest,
        &mut mod_api::BlockEntityTickEvent {
            pos: [1, 2, 3],
            reply: &mut mod_api::BlockEntityTickReply::Changed,
            entity: mod_api::ModBlockEntityView::new(&mut entity),
        },
    );
    assert_eq!(entity.data, vec![8]);
    assert_eq!(entity.revision().changes(), before);
    entity.data[0] = -1;
    let before = entity.revision();
    let mut reply = mod_api::BlockEntityTickReply::Changed;
    proxy.block_entity_tick(
        &mut guest::Guest,
        &mut mod_api::BlockEntityTickEvent {
            pos: [1, 2, 3],
            reply: &mut reply,
            entity: mod_api::ModBlockEntityView::new(&mut entity),
        },
    );
    assert_eq!(reply, mod_api::BlockEntityTickReply::Unchanged);
    assert_eq!(entity.data, [-1]);
    assert_eq!(entity.revision(), before);
}
#[test]
fn borrowed_view_reply_is_byte_identical_to_owned_snapshot() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use horizon_core::nbt::Tag;
    install_counter();
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 17).unwrap();
    let mut entity = mod_api::ModBlockEntity::default();
    entity.items = vec![Some(stack), None];
    entity.data = vec![i32::MIN, 7, i32::MAX];
    entity.nbt.insert(
        "nested".into(),
        Tag::compound([
            ("float", Tag::Float(0x7fc01234)),
            ("double", Tag::Double(0x7ff8000000001234)),
            ("negative_zero", Tag::Float((-0.0f32).to_bits())),
            (
                "list",
                Tag::List(vec![
                    Tag::Double((-0.0f64).to_bits()),
                    Tag::String("payload".into()),
                ]),
            ),
        ]),
    );
    let mut update = mod_api::Compound::new();
    update.insert("display".into(), Tag::Int(42));
    mod_api::ModBlockEntityView::new(&mut entity).set_client_update_fields(update);
    let mut owned = Writer::new();
    owned.put(&mod_api::item::containers::ModBlockEntitySnapshot::from(
        &entity,
    ));
    let mut borrowed = Writer::new();
    borrowed.put(&mod_api::ModBlockEntityView::new(&mut entity));
    assert!(owned.error().is_none());
    assert!(borrowed.error().is_none());
    assert_eq!(borrowed.bytes(), owned.bytes());
    let mut reply = Writer::new();
    mod_api::ModBlockEntityView::new(&mut entity).__wire_put_snapshot(&mut reply);
    assert!(reply.error().is_none());
    assert_eq!(reply.bytes(), owned.bytes());
}
fn static_reply(
    tag: u8,
    id: u32,
    text: Option<&str>,
    trailing: bool,
) -> Result<&'static str, WireError> {
    let mut out = Writer::new();
    out.put(&tag);
    out.put(&id);
    if let Some(text) = text {
        out.put(text);
    }
    if trailing {
        out.put(&7u8);
    }
    let bytes = out.into_bytes();
    guest::set_transport(Some(Rc::new(move |_, _| bytes.clone())));
    let result = guest::host(0, guest::begin())
        .take_static::<()>()
        .map(|(text, ())| text);
    guest::set_transport(None);
    result
}
#[test]
fn static_reply_definitions_are_checked_before_acknowledgement() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    guest::reset_static_strings();
    assert_eq!(
        static_reply(0, 0, None, false),
        Err(WireError::Invalid("unknown static string id"))
    );
    assert_eq!(static_reply(1, 1, Some("second"), false).unwrap(), "second");
    assert_eq!(
        guest::static_ack(),
        1,
        "acknowledge only the exact decoded id, independently of holes"
    );
    let first = static_reply(1, 0, Some("first"), false).unwrap();
    assert_eq!(guest::static_ack(), 0);
    assert!(std::ptr::eq(
        first,
        static_reply(0, 0, None, false).unwrap()
    ));
    assert!(std::ptr::eq(
        first,
        static_reply(1, 0, Some("first"), false).unwrap()
    ));
    assert!(static_reply(1, 0, Some("changed"), false).is_err());
    assert_eq!(
        static_reply(1, 2, Some("not admitted"), true),
        Err(WireError::Trailing)
    );
    assert_eq!(guest::static_ack(), 0);
    assert!(static_reply(0, 2, None, false).is_err());
    assert_eq!(static_reply(9, 0, None, false), Err(WireError::Tag(9)));
    assert!(static_reply(1, u32::MAX, Some("bad id"), false).is_err());
    assert!(static_reply(1, 2, Some(&"x".repeat(257)), false).is_err());
    for tail in [None, Some(2u8)] {
        let mut out = Writer::new();
        out.put(&1u8);
        out.put(&2u32);
        out.put("unadmitted scalar tail");
        if let Some(tail) = tail {
            out.put(&tail);
        }
        let bytes = out.into_bytes();
        guest::set_transport(Some(Rc::new(move |_, _| bytes.clone())));
        assert!(
            guest::host(0, guest::begin())
                .take_static::<bool>()
                .is_err()
        );
        guest::set_transport(None);
        assert!(static_reply(0, 2, None, false).is_err());
    }
    assert_eq!(
        static_reply(2, u32::MAX, Some("uncached"), false).unwrap(),
        "uncached"
    );
    assert_eq!(
        guest::static_ack(),
        0,
        "uncached text does not claim a table id"
    );
    assert!(static_reply(2, 0, Some("bad marker"), false).is_err());
    guest::reset_static_strings();
    assert_eq!(guest::static_ack(), u32::MAX);
    assert!(
        static_reply(0, 0, None, false).is_err(),
        "a fresh instance cannot resolve an old id"
    );
}
#[test]
fn static_reply_table_is_bounded() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    guest::reset_static_strings();
    for id in 0..4096 {
        assert_eq!(static_reply(1, id, Some("value"), false).unwrap(), "value");
    }
    assert_eq!(guest::static_ack(), 4095);
    assert!(static_reply(1, 4096, Some("overflow"), false).is_err());
    assert_eq!(guest::static_ack(), 4095);
}
#[test]
fn native_static_reply_resends_until_guest_acknowledges() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use std::cell::Cell;
    struct World(Cell<usize>);
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World, except(config));
    impl mod_api::prelude::ConfigHost for World {
        fn dimension(&self) -> &'static str {
            self.0.set(self.0.get() + 1);
            "minecraft:overworld"
        }
    }
    let world = World(Cell::new(0));
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let pointer = &mut cx as *mut boundary::Cx;
    let _resources = unsafe { boundary::resource_scope(pointer, true) };
    let id = boundary::import_id("Host", "dimension").unwrap();
    let mut call = |ack: u32| {
        let mut args = Writer::new();
        args.put(&ack);
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, args.bytes(), &mut out) }?;
        Ok::<_, boundary::ServeError>(out.into_bytes())
    };
    assert!(call(1).is_err());
    assert_eq!(
        world.0.get(),
        0,
        "invalid acknowledgement must refuse before native invocation"
    );
    for ack in [u32::MAX, u32::MAX, 0, u32::MAX] {
        let bytes = call(ack).unwrap();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.take::<u8>().unwrap(), u8::from(ack == u32::MAX));
        assert_eq!(reader.take::<u32>().unwrap(), 0);
        if ack == u32::MAX {
            assert_eq!(reader.take::<String>().unwrap(), "minecraft:overworld");
        }
        assert!(reader.rest().is_empty());
    }
    assert_eq!(world.0.get(), 4);
    assert!(call(2).is_err());
    assert_eq!(world.0.get(), 4);
}
#[test]
fn native_static_reply_table_refuses_overflow_without_losing_existing_ids() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use std::cell::Cell;
    struct World(Cell<&'static str>);
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World, except(config));
    impl mod_api::prelude::ConfigHost for World {
        fn dimension(&self) -> &'static str {
            self.0.get()
        }
    }
    let world = World(Cell::new(""));
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let pointer = &mut cx as *mut boundary::Cx;
    let _resources = unsafe { boundary::resource_scope(pointer, true) };
    let id = boundary::import_id("Host", "dimension").unwrap();
    let call = |ack: u32| {
        let mut args = Writer::new();
        args.put(&ack);
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, args.bytes(), &mut out) }?;
        Ok::<_, boundary::ServeError>(out.into_bytes())
    };
    for i in 0..4096 {
        world
            .0
            .set(Box::leak(format!("test:dimension_{i}").into_boxed_str()));
        assert!(call(u32::MAX).is_ok());
    }
    world.0.set("test:overflow");
    let uncached = call(u32::MAX).unwrap();
    let mut reader = Reader::new(&uncached);
    assert_eq!(reader.take::<u8>().unwrap(), 2);
    assert_eq!(reader.take::<u32>().unwrap(), u32::MAX);
    assert_eq!(reader.take::<String>().unwrap(), "test:overflow");
    assert!(reader.rest().is_empty());
    world.0.set("test:dimension_0");
    let bytes = call(0).unwrap();
    assert_eq!(bytes, [0, 0, 0, 0, 0]);
    assert!(
        call(4096).is_err(),
        "overflow must not insert a new table entry"
    );
}
#[test]
fn wire_loopback_readonly_private_projection() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use horizon_core::nbt::Tag;
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap();
    let hidden = Tag::compound([(
        "hidden_components",
        Tag::List(vec![Tag::String("test:hidden".into())]),
    )]);
    let suppressed = Tag::compound([("hide_tooltip", Tag::Byte(1))]);
    for (component, ominous_banner, expected) in [
        (None, false, 15),
        (None, true, 11),
        (Some(&hidden), true, 7),
        (Some(&suppressed), false, 2),
    ] {
        let display = mod_api::TooltipDisplay::__wire_from_fields(component, ominous_banner);
        let mut lines = vec![(mod_api::Text::literal("prefix"), 99)];
        module.item_tooltip(
            &Client,
            &mut mod_api::ItemTooltipEvent {
                player: 1,
                player_entity_id: 2,
                stack: &stack,
                display: &display,
                advanced: false,
                creative: false,
                lines: &mut lines,
            },
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].1, 99);
        assert_eq!(lines[1].1, expected);
    }
}
/// An answered collect crosses the generated wire proxies as the guest's own plain batch, run by the
/// real guest dispatcher (natively; the wasm-mods workflow runs a wasm guest in `real_member`): the
/// guest starts empty although a native mod pushed before it, and the host names the guest mod on
/// each entry it pushed, after the native one's.
#[test]
fn wire_loopback_answered_collect_names_the_guest_mod() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::__private::game::RegistrationContextNew;
    let id = |id: &str| mod_api::Identifier::parse(id).unwrap();
    struct First;
    #[mod_api::__subscriptions]
    impl Hooks for First {
        fn register_json_animation_types(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut mod_api::RegisterJsonAnimationTypesEvent<'_>,
        ) {
            event.types.push(mod_api::JsonAnimationType::Target(
                mod_api::Identifier::parse("first:glow").unwrap(),
            ));
        }
    }
    let mods = [
        mod_api::ModEntry::new("first", Box::new(First)),
        mod_api::ModEntry::new(
            "wire_test",
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::RegisterJsonAnimationTypes)],
                listeners: &[],
                corrupt: false,
            }),
        ),
    ];
    mod_api::testing::with_mods(Arc::from(mods), || {
        let mut types = Vec::new();
        mod_api::dispatch_register_json_animation_types(
            &mod_api::RegistrationContext::new(),
            &mut types,
        );
        let named: Vec<_> = types
            .iter()
            .map(|entry| (entry.answer.clone(), entry.mod_id))
            .collect();
        assert_eq!(
            named,
            [
                (
                    mod_api::JsonAnimationType::Target(id("first:glow")),
                    "first"
                ),
                (
                    mod_api::JsonAnimationType::Target(id("wire_test:pulse")),
                    "wire_test"
                ),
                (
                    mod_api::JsonAnimationType::Interpolation(id("wire_test:step")),
                    "wire_test"
                ),
            ]
        );
    });
}

#[test]
fn wire_loopback_biome_region_declaration_reaches_the_host_bit_exact() {
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
    let declared = mod_api::__private::biome_region_declarations();
    assert_eq!(declared.len(), 1);
    assert_eq!(declared[0].mod_id, "wire_test");
    assert_eq!(declared[0].id, "wire_test:ember_marsh");
    assert_eq!(declared[0].region, biome_region_registration::region());
}
#[test]
fn wire_loopback_checked_append_is_atomic_and_bit_exact() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::EffectTickChange;
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            group: 0,
            subscribed: &[Some(mod_api::HookId::ApplyEffectTick)],
            listeners: &[],
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
    let holder = mod_api::LivingRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7));
    mod_api::testing::with_mods(Arc::from(mods), || {
        for attack in [0, 1, 2, 3] {
            APPEND_ATTACK.set(attack);
            let mut changes = vec![EffectTickChange::Heal(f32::from_bits(0x7fc01234))];
            let mut keep = true;
            mod_api::dispatch_apply_effect_tick(
                &World,
                mod_api::__private::ApplyEffectTickCall {
                    effect: REGISTERED_EFFECT.get().unwrap(),
                    dimension: "test:world",
                    entity: Some(7),
                    amplifier: 0,
                    holder,
                    health: 10.,
                    max_health: 20.,
                    food: None,
                    changes: &mut changes,
                    keep: &mut keep,
                },
            );
            assert!(matches!(changes[0], EffectTickChange::Heal(v) if v.to_bits() == 0x7fc01234));
            if attack == 0 {
                assert!(!keep);
                assert_eq!(
                    &changes[1..],
                    &[
                        EffectTickChange::Heal(2.5),
                        EffectTickChange::Hurt("minecraft:wither".into(), 3.),
                        EffectTickChange::Eat(2, 0.25),
                        EffectTickChange::AddExhaustion(0.5)
                    ]
                );
            } else {
                assert!(
                    keep,
                    "refused prefix must also refuse the public keep writeback"
                );
                assert_eq!(changes.len(), 1);
            }
        }
        APPEND_ATTACK.set(0);
    });
}
#[test]
fn wire_loopback_optional_borrowed_render_record() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    struct Draw(u32);
    impl mod_api::BlockEntityDrawContext for Draw {
        fn set_light(&mut self, value: u32) {
            self.0 = value;
        }
    }
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mob = mod_api::MobRender {
        id: 1,
        kind: "test:mob",
        custom_name: Some("custom"),
        baby: false,
        partial: 0.5,
        age: 1.,
        body_yaw: 0.,
        head_yaw: 0.,
        pitch: 0.,
        walk_position: 0.,
        walk_speed: 0.,
        hurt_time: 0,
        death_time: 0,
        on_ground: true,
    };
    let player = mod_api::PlayerRender {
        name: "test:player",
        local: true,
    };
    let still = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    let parts = mod_api::PosedParts::new(|_| Some((still, true))).unwrap();
    for (parts, light) in [(None, 7), (Some(&parts), 9)] {
        let mut draw = Draw(0);
        module.render_living_post(
            &Client,
            &mut mod_api::RenderLivingPostEvent {
                entity: &mob,
                parts,
                draw: &mut draw,
            },
        );
        assert_eq!(draw.0, light);
    }
    let mut draw = Draw(0);
    module.render_player_post(
        &Client,
        &mut mod_api::RenderPlayerPostEvent {
            player: &mob,
            who: &player,
            parts: None,
            draw: &mut draw,
        },
    );
    assert_eq!(draw.0, 42);
}
#[test]
fn wire_loopback_optional_borrowed_record() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let actor: mod_api::GameRuleActor = mod_api::GameRuleActor {
        uuid: 123,
        entity_id: 45,
        name: "test:actor",
    };
    for actor in [None, Some(&actor)] {
        let mut event = mod_api::GameRuleChangedEvent {
            rule: "test:rule",
            value: mod_api::GameRuleValue::Bool(!actor.is_some()),
            actor,
        };
        module.game_rule_changed(&mut World, &mut event);
        assert_eq!(event.value, mod_api::GameRuleValue::Bool(actor.is_some()));
    }
}
#[test]
fn wire_loopback_item_picked_up_post_carries_the_original_and_writes_back_the_remainder() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let original = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 5).unwrap();
    let mut remaining = None;
    let item = mod_api::ItemEntityRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 9));
    let mut event = mod_api::ItemPickedUpPostEvent {
        player: 0x1234_5678_9abc_def0_1234_5678_9abc_def0,
        item,
        original: &original,
        remaining: &mut remaining,
    };
    module.item_picked_up_post(&mut World, &mut event);
    assert_eq!(
        PICKED_UP_POST.get(),
        Some((0x1234_5678_9abc_def0_1234_5678_9abc_def0, 9, 5, None))
    );
    assert_eq!(
        remaining.map(|rest| rest.count()),
        Some(3),
        "the remainder a guest sets comes back"
    );
}
#[test]
fn wire_loopback_item_toss_carries_the_item_entity_and_its_cancel() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let item =
        mod_api::ItemEntityRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 11));
    let mut event = mod_api::ItemTossEvent {
        player: 0x42,
        item,
        cancelled: false,
    };
    let flow = module.item_toss(&mut World, &mut event);
    assert_eq!(TOSSED.get(), Some((0x42, 11)));
    assert!(
        matches!(flow, mod_api::Flow::Cancel),
        "the guest's cancel comes back"
    );
}
#[test]
fn wire_loopback_item_picked_up_partial_carries_the_original_and_writes_back_the_remainder() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let original = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 8).unwrap();
    let mut remaining = Some(mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 3).unwrap());
    let mut event = mod_api::ItemPickedUpPartialEvent {
        player: 0x1234_5678_9abc_def0_1234_5678_9abc_def0,
        original: &original,
        remaining: &mut remaining,
    };
    module.item_picked_up_partial(&mut World, &mut event);
    assert_eq!(
        PICKED_UP_PARTIAL.get(),
        Some((0x1234_5678_9abc_def0_1234_5678_9abc_def0, 8, Some(3)))
    );
    assert!(
        remaining.is_none(),
        "the remainder a guest empties comes back"
    );
}
#[test]
fn wire_loopback_array_of_optional_borrowed_records() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::BlockEntityDrawContext;
    struct Draw(bool);
    impl BlockEntityDrawContext for Draw {
        fn cube(
            &mut self,
            from: [f32; 3],
            to: [f32; 3],
            faces: [Option<mod_api::CubeFace<'_>>; 6],
            tint: u32,
        ) -> bool {
            assert_eq!((from, to, tint), ([1.; 3], [2.; 3], 0x123456));
            assert!(faces[0].is_none());
            let face = faces[1].unwrap();
            assert_eq!(face.sprite, "test:face");
            assert_eq!(face.uv, Some([1., 2., 3., 4.]));
            assert!(faces[2..].iter().all(Option::is_none));
            self.0 = true;
            true
        }
    }
    let mut draw = Draw(false);
    let mut cx = Box::new(boundary::Cx::default());
    let mut frame = boundary::Frame::default();
    frame.set_block_entity_draw_context_mut(&mut draw);
    cx.frames.push(frame);
    let pointer = &mut *cx as *mut boundary::Cx;
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let sprite = String::from("test:face");
    let faces = [
        None,
        Some(mod_api::CubeFace::new(&sprite).with_uv([1., 2., 3., 4.])),
        None,
        None,
        None,
        None,
    ];
    assert!(guest::Guest.cube([1.; 3], [2.; 3], faces, 0x123456));
    assert!(draw.0);
    guest::set_transport(None);
}
#[test]
fn wire_loopback_borrowed_tuple_inside_optional_context() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::__private::game::BackendRefOf;
    struct World;
    impl mod_api::Host for World {
        fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
            Some(mod_api::BackendRef::of(self))
        }
    }
    mod_api::impl_host_areas!(World, except(entities, living));
    impl mod_api::prelude::EntitiesHost for World {
        fn entity(&self, id: u32) -> Option<mod_api::EntityRef> {
            Some(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), id))
        }
    }
    impl mod_api::prelude::LivingHost for World {
        fn living(&self, entity: mod_api::EntityRef) -> Option<mod_api::LivingRef> {
            Some(mod_api::LivingRef::new(entity))
        }
    }
    impl mod_api::Backend for World {}
    mod_api::impl_backend_areas!(World, except(living));
    impl mod_api::prelude::LivingClientBackend for World {}
    impl mod_api::prelude::LivingBackend for World {
        fn living_hand_item(
            &self,
            _: mod_api::LivingRef,
            _: mod_api::Hand,
        ) -> Option<mod_api::ItemStack> {
            EXPECTED_LOOT_HAND.with(|expected| expected.borrow().clone())
        }
    }
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);

    install_counter();
    let table = "t".repeat(300);
    let mut hand = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 7).unwrap();
    hand.set_damage(19);
    hand.set_components(
        [(
            "minecraft:custom_data".into(),
            mod_api::Tag::Compound([("marker".into(), mod_api::Tag::Int(73))].into()),
        )]
        .into(),
    )
    .unwrap();
    EXPECTED_LOOT_HAND.with(|expected| *expected.borrow_mut() = Some(hand.clone()));
    let context: mod_api::LootDrops = mod_api::__private::built::LootDrops {
        weapon: Some(&hand),
        damage_source: Some(mod_api::HurtSource {
            damage_type: "minecraft:arrow",
            causing: 41,
            direct: 73,
            position: Some([1.0, 2.0, 3.0]),
        }),
        table: &table,
        dimension: "minecraft:overworld",
        origin: [0.; 3],
        block: None,
        entity: Some((12, "test:mob")),
        player: None,
        tool: None,
        explosion: None,
        used: false,
    }
    .into();
    let mut drops = vec![mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap()];
    WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    }
    .loot_drops(
        &mut World,
        &mut mod_api::LootDropsEvent {
            context: &context,
            drops: &mut drops,
        },
    );
    assert!(drops.is_empty());
}
#[test]
fn wire_loopback_item_fished_edits_catch() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let key = REGISTERED_ITEM.get().unwrap();
    let mut drops = vec![mod_api::ItemStack::new(key, 1).unwrap()];
    let fished = mod_api::ItemFished::from(mod_api::__private::built::ItemFished {
        hook: 19,
        position: [1.0, 64.0, 2.0],
    });
    let mut event = mod_api::ItemFishedEvent {
        player: 7,
        player_entity_id: 3,
        fished: &fished,
        drops: &mut drops,
        rod_damage: 1,
        cancelled: false,
    };
    let flow = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    }
    .item_fished(&mut World, &mut event);
    assert!(matches!(flow, mod_api::Flow::Continue));
    assert_eq!(event.rod_damage, 5);
    assert!(!event.cancelled);
    assert_eq!(drops, vec![mod_api::ItemStack::new(key, 2).unwrap()]);
}
#[test]
fn wire_loopback_nested_tuple_reference_writes_back() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    install_counter();
    let mut carried = Some(mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap());
    let mut clicked = guest::Guest;
    let mut stack = mod_api::StackedOnOther {
        player: None,
        player_entity_id: None,
        slot: 0,
        container: 0,
        container_slot: 0,
        primary: true,
        carried: mod_api::item::menu::CarriedAccess(&mut carried),
        clicked: &mut clicked,
        cancellation_result: false,
    };
    module.item_stacked_on_other(
        &Context,
        &mut mod_api::ItemStackedOnOtherEvent {
            event: &mut stack,
            cancelled: false,
        },
    );
    assert!(stack.carried.0.is_none());
    assert!(stack.cancellation_result);
}
thread_local! {
    static REGISTERED_ITEM:std::cell::Cell<Option<mod_api::ItemKey>> = const {std::cell::Cell::new(None)};
    static REGISTERED_BLOCK:std::cell::Cell<Option<mod_api::BlockKey>> = const {std::cell::Cell::new(None)};
    static REGISTERED_EFFECT:std::cell::Cell<Option<mod_api::EffectKey>> = const {std::cell::Cell::new(None)};
    static APPEND_ATTACK:std::cell::Cell<u8> = const {std::cell::Cell::new(0)};
    static REGISTERED_KIND:std::cell::Cell<Option<mod_api::BlockEntityTypeKey>> = const {std::cell::Cell::new(None)};
    static PICKED_UP_POST: std::cell::Cell<Option<(u128, u32, u32, Option<u32>)>> = const { std::cell::Cell::new(None) };
    static TOSSED: std::cell::Cell<Option<(u128, u32)>> = const { std::cell::Cell::new(None) };
    static PICKED_UP_PARTIAL: std::cell::Cell<Option<(u128, u32, Option<u32>)>> = const { std::cell::Cell::new(None) };
}
fn install_counter() {
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
}
#[test]
fn wire_loopback_registration_attaches_subscribed_part() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    assert!(
        mod_api::__private::block_entity_types()
            .iter()
            .any(|decl| decl.id == "wire_test:counter")
    );
    assert!(mod_api::__private::has_part_hook(
        mod_api::HookId::BlockEntityTick
    ));
}
#[test]
fn wire_loopback_tagged_frame_enum_preserves_side() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::BlockEntityPart;
    install_counter();
    let index = guest::add_handler(vec![guest::view::<dyn mod_api::BlockEntityPart>(Arc::new(
        MachineBlock,
    ))]);
    let proxy = WasmHandler {
        index,
        module: WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: false,
        },
    };
    let mut event = mod_api::BlockEntityEvent {
        block: "test:machine",
        state: mod_api::BlockStateRecord::new(REGISTERED_BLOCK.get().unwrap(), 0).into(),
    };
    assert!(
        proxy
            .block_entity(mod_api::LevelSide::Server(&guest::Guest), &mut event)
            .is_some()
    );
    assert!(
        proxy
            .block_entity(mod_api::LevelSide::Client(&guest::Guest), &mut event)
            .is_none()
    );
}
#[test]
fn wire_loopback_owned_box_handlers_inside_result_and_vec() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::ReloadApply;
    let trace = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut out = Writer::new();
    {
        let _resources = guest::resource_scope();
        let trace = trace.clone();
        let value: Result<Box<dyn ReloadApply>, u32> = Ok(<dyn ReloadApply>::new(move || {
            trace.lock().unwrap().push(7)
        }));
        out.put_owned(value);
    }
    assert_eq!(out.error(), None);
    let mut cx = boundary::Cx::default();
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    cx.module = &module;
    {
        let _resources = unsafe { boundary::resource_scope(&mut cx, true) };
        let value = Reader::decode::<Result<Box<dyn ReloadApply>, u32>>(out.bytes())
            .unwrap()
            .unwrap();
        value.apply();
    }
    assert_eq!(*trace.lock().unwrap(), vec![7]);
    out.clear();
    {
        let _resources = guest::resource_scope();
        let values = (0..3)
            .map(|n| {
                let trace = trace.clone();
                <dyn ReloadApply>::new(move || trace.lock().unwrap().push(n))
            })
            .collect::<Vec<_>>();
        out.put_owned(values);
    }
    assert_eq!(out.error(), None);
    {
        let _resources = unsafe { boundary::resource_scope(&mut cx, true) };
        for value in Reader::decode::<Vec<Box<dyn ReloadApply>>>(out.bytes()).unwrap() {
            value.apply();
        }
    }
    assert_eq!(*trace.lock().unwrap(), vec![7, 0, 1, 2]);
}
#[test]
fn wire_loopback_returned_frame_resource_uses_source_permissions() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct Start {
        draws: i64,
    }
    impl mod_api::StructureRandom for Start {
        fn next_long(&mut self) -> i64 {
            self.draws += 1;
            41 + self.draws
        }
        fn next_int_bound(&mut self, _: i32) -> i32 {
            unreachable!()
        }
        fn next_float(&mut self) -> f32 {
            unreachable!()
        }
        fn next_double(&mut self) -> f64 {
            unreachable!()
        }
        fn next_boolean(&mut self) -> bool {
            unreachable!()
        }
    }
    impl mod_api::GenerationContext for Start {
        fn metadata(&self) -> &dyn mod_api::BlockMetadata {
            unreachable!()
        }
        fn random(&mut self) -> &mut dyn mod_api::StructureRandom {
            self
        }
        fn chunk(&self) -> mod_api::ChunkPos {
            unreachable!()
        }
        fn seed(&self) -> i64 {
            unreachable!()
        }
        fn sea_level(&self) -> i32 {
            unreachable!()
        }
        fn min_y(&self) -> i32 {
            unreachable!()
        }
        fn max_y(&self) -> i32 {
            unreachable!()
        }
        fn base_height(&mut self, _: i32, _: i32, _: mod_api::HeightmapType) -> i32 {
            unreachable!()
        }
        fn base_columns(&mut self, _: i32, _: i32, _: &[mod_api::HeightmapType]) -> Vec<Vec<bool>> {
            unreachable!()
        }
        fn noise_column(&mut self, _: i32, _: i32) -> mod_api::NoiseColumn {
            unreachable!()
        }
        fn noise_biome(&mut self, _: [i32; 3]) -> mod_api::BiomeId {
            unreachable!()
        }
        fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
            unreachable!()
        }
        fn config(&self) -> Arc<mod_api::Tag> {
            unreachable!()
        }
    }
    let mut start = Start { draws: 0 };
    let mut cx = boundary::Cx::default();
    let mut frame = boundary::Frame::default();
    frame.set_generation_context_mut(&mut start);
    cx.frames.push(frame);
    let mut out = Writer::new();
    let id = boundary::import_id("StructureRandom", "next_long").unwrap();
    unsafe { boundary::serve(id, &mut cx, &[], &mut out) }.unwrap();
    assert_eq!(Reader::decode::<i64>(out.bytes()).unwrap(), 42);
    assert_eq!(start.draws, 1);
    cx.frames.clear();
    let mut frame = boundary::Frame::default();
    frame.set_generation_context(&start);
    cx.frames.push(frame);
    out.clear();
    assert!(matches!(
        unsafe { boundary::serve(id, &mut cx, &[], &mut out) },
        Err(boundary::ServeError::Missing(_))
    ));
    assert_eq!(start.draws, 1);
    assert!(out.bytes().is_empty());
}
#[test]
fn wire_loopback_provider_queries_return_owned_options_and_atomic_cursor_writeback() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::*;
    struct Table;
    impl CapabilityProviders for Table {}
    impl BlocksCapabilityProviders for Table {
        fn block_provider(
            &self,
            _: &str,
            kind: Option<BlockEntityTypeKey>,
            block: Option<BlockKey>,
            index: &mut u32,
        ) -> Option<(BlockProviderOf, &'static str, bool)> {
            // Which of the position's two keys crossed picks the answer.
            match (kind, block) {
                (Some(_), None) => {
                    *index += 1;
                    Some((
                        BlockProviderOf::EnergyHandler(Arc::new(
                            |_: &mut dyn ServerHost, _: BlockSide| None::<Box<dyn EnergyHandler>>,
                        )),
                        "test:block",
                        true,
                    ))
                }
                (None, Some(_)) => {
                    *index += 4;
                    Some((
                        BlockProviderOf::ResourceHandlerItemResource(Arc::new(
                            |_: &mut dyn ServerHost, _: BlockSide| {
                                None::<Box<dyn ResourceHandler<ItemResource>>>
                            },
                        )),
                        "test:plain_block",
                        false,
                    ))
                }
                _ => None,
            }
        }
    }
    impl EntitiesCapabilityProviders for Table {
        fn entity_provider(
            &self,
            _: &str,
            _: &str,
            index: &mut u32,
        ) -> Option<(EntityProviderOf, &'static str, bool)> {
            *index += 2;
            Some((
                EntityProviderOf::EnergyHandlerOptionDirection(Arc::new(
                    |_: &mut dyn ServerHost, _: EntityRef, _: Option<Direction>| {
                        None::<Box<dyn EnergyHandler>>
                    },
                )),
                "test:entity",
                false,
            ))
        }
    }
    impl ItemsCapabilityProviders for Table {
        fn item_provider(
            &self,
            _: &str,
            _: ItemKey,
            index: &mut u32,
        ) -> Option<(ItemProviderOf, &'static str, bool)> {
            *index += 3;
            Some((
                ItemProviderOf::EnergyHandler(Arc::new(|_: &mut dyn ServerHost, _: &ItemPlace| {
                    None::<Box<dyn EnergyHandler>>
                })),
                "test:item",
                true,
            ))
        }
        fn place_provider(&self, _: &str) -> Option<(Arc<dyn PlaceProvider>, &'static str)> {
            None
        }
    }
    impl CapabilitiesCapabilityProviders for Table {
        fn provided(&self, _: ProvidedBy, _: &str) -> bool {
            true
        }
        fn block_capability_names(&self) -> Vec<String> {
            Vec::new()
        }
        fn block_capability_proxyable(&self, _: &str) -> bool {
            false
        }
        fn block_capability_proxyable_names(&self) -> Vec<String> {
            Vec::new()
        }
        fn mods_switched(&self) -> u64 {
            7
        }
    }
    impl DefaultCapabilitiesCapabilityProviders for Table {
        fn block_container(&self, _: &str) -> Option<ModContainer> {
            None
        }
        fn block_fluids(&self, _: &str) -> Option<ModFluidTanks> {
            None
        }
        fn block_energy(&self, _: &str) -> Option<ModEnergyBuffer> {
            None
        }
        fn item_container_provider(
            &self,
            _: &str,
            _: &mut u32,
        ) -> Option<Arc<dyn ItemContainerProvider>> {
            None
        }
    }
    struct World;
    impl Host for World {
        fn capability_providers(&self) -> &'static dyn CapabilityProviders {
            &Table
        }
    }
    mod_api::impl_host_areas!(World);
    install_counter();
    let world = World;
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mut cx = Box::new(boundary::Cx::default());
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    cx.frames.push(frame);
    cx.module = &module;
    let pointer = &mut *cx as *mut boundary::Cx;
    let _root = unsafe { boundary::resource_scope(pointer, true) };
    let _guest = guest::resource_scope();
    let corrupt = Rc::new(std::cell::Cell::new(false));
    let flag = corrupt.clone();
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        if id == mod_api::wire::HOST_LOG {
            return Vec::new();
        }
        if id == mod_api::wire::HOST_RELEASE {
            let id = Reader::decode(bytes).unwrap();
            let released = unsafe { (&mut *pointer).held.remove(id) }.unwrap();
            drop(released);
            return Vec::new();
        }
        let _scope = unsafe { boundary::resource_scope(pointer, false) };
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
        if flag.get() && id != mod_api::wire::HOST_RELEASE {
            out.put(&0xffu8)
        }
        out.into_bytes()
    })));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            guest::set_transport(None);
        }
    }
    let _reset = Reset;
    let mut index = 5;
    let (block, owner, before) = guest::Guest
        .block_provider(
            "test:energy",
            Some(REGISTERED_KIND.get().unwrap()),
            None,
            &mut index,
        )
        .unwrap();
    assert!(matches!(block, BlockProviderOf::EnergyHandler(_)));
    assert_eq!((index, owner, before), (6, "test:block", true));
    let (entity, owner, before) = guest::Guest
        .entity_provider("test:energy", "test:mob", &mut index)
        .unwrap();
    assert!(matches!(
        entity,
        EntityProviderOf::EnergyHandlerOptionDirection(_)
    ));
    assert_eq!((index, owner, before), (8, "test:entity", false));
    let (item, owner, before) = guest::Guest
        .item_provider("test:energy", REGISTERED_ITEM.get().unwrap(), &mut index)
        .unwrap();
    assert!(matches!(item, ItemProviderOf::EnergyHandler(_)));
    assert_eq!((index, owner, before), (11, "test:item", true));
    let (block, owner, before) = guest::Guest
        .block_provider(
            "test:items",
            None,
            Some(REGISTERED_BLOCK.get().unwrap()),
            &mut index,
        )
        .unwrap();
    assert!(matches!(
        block,
        BlockProviderOf::ResourceHandlerItemResource(_)
    ));
    assert_eq!((index, owner, before), (15, "test:plain_block", false));
    assert_eq!(
        guest::Guest.mods_switched(),
        7,
        "a guest reads the game's count of mod switches, not its own"
    );
    corrupt.set(true);
    index = 42;
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        guest::Guest.item_provider("test:energy", REGISTERED_ITEM.get().unwrap(), &mut index)
    }));
    assert!(refused.is_err());
    assert_eq!(
        index, 42,
        "trailing bytes applied the cursor before validating the whole reply"
    );
}

#[test]
fn wire_loopback_panic_hook_without_transport_does_not_panic_again() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    guest::set_transport(None);
    let panic = std::panic::catch_unwind(|| panic!("native panic after the guest call ended"));
    assert!(panic.is_err());
}

#[test]
fn wire_loopback_erased_resource_restores_identity_and_refuses_other_groups() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct Capture(std::sync::Mutex<Vec<String>>);
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, r: &log::Record<'_>) {
            self.0.lock().unwrap().push(r.args().to_string());
        }
        fn flush(&self) {}
    }
    static LOG: Capture = Capture(std::sync::Mutex::new(Vec::new()));
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    let original: Arc<dyn std::any::Any + Send + Sync> = Arc::new(97u32);
    let handler: Arc<str> = format!("test:long-handler-{}", "a".repeat(300)).into();
    let mut outbound = Writer::new();
    {
        let _scope = guest::resource_scope();
        outbound.put(&handler);
        outbound.put(&original);
    }
    let mut cx = boundary::Cx::default();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    cx.module = &module;
    let native: mod_api::OwnProvider = {
        let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
        Reader::decode(outbound.bytes()).unwrap()
    };
    let mut back = Writer::new();
    {
        let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
        back.put(&native);
    }
    assert_eq!(back.bytes(), outbound.bytes());
    {
        let _scope = guest::resource_scope();
        let mut input = Reader::new(back.bytes());
        assert_eq!(input.take::<Arc<str>>().unwrap(), handler);
        let restored: Arc<dyn std::any::Any + Send + Sync> = input.take().unwrap();
        assert!(input.rest().is_empty());
        assert!(Arc::ptr_eq(&original, &restored));
        assert_eq!(restored.downcast_ref::<u32>(), Some(&97));
    }
    let other_module = WasmMod {
        group: 1,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    cx.module = &other_module;
    for _ in 0..2 {
        back.clear();
        {
            let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
            back.put(&native);
        }
        let _scope = guest::resource_scope();
        let mut input = Reader::new(back.bytes());
        assert_eq!(input.take::<Arc<str>>().unwrap(), handler);
        let refused: Arc<dyn std::any::Any + Send + Sync> = input.take().unwrap();
        assert!(input.rest().is_empty());
        assert_eq!(
            refused
                .downcast_ref::<mod_api::wire::Unsupported>()
                .unwrap()
                .reason,
            "cross-group capability"
        );
        assert!(refused.downcast_ref::<u32>().is_none());
    }
    assert_eq!(
        LOG.0
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.contains(&*handler) && s.contains("cross-group capability"))
            .count(),
        1
    );
}

#[test]
fn wire_loopback_builtin_provider_invokes_nested_box_handler() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use mod_api::{EnergyHandler, ItemCapabilityProvider, ItemProviderOf, ServerHost, Transaction};
    struct Energy(u32);
    impl EnergyHandler for Energy {
        fn amount(&self, _: &mut dyn ServerHost) -> u32 {
            self.0
        }
        fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
            self.0
        }
        fn insert(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
            0
        }
        fn extract(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
            0
        }
    }
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    let place = mod_api::ItemPlace::PlayerSlot {
        player: 19,
        slot: 3,
    };
    let provider: Arc<dyn ItemCapabilityProvider<dyn EnergyHandler>> = Arc::new(
        |_: &mut dyn ServerHost, place: &mod_api::ItemPlace| match place {
            mod_api::ItemPlace::PlayerSlot { player: 19, slot } => {
                Some(Box::new(Energy(*slot as u32)) as Box<dyn EnergyHandler>)
            }
            _ => None,
        },
    );
    let mut out = Writer::new();
    {
        let _scope = guest::resource_scope();
        out.put(&ItemProviderOf::EnergyHandler(provider));
    }
    let mut cx = boundary::Cx::default();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    cx.module = &module;
    let decoded: ItemProviderOf = {
        let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
        Reader::decode(out.bytes()).unwrap()
    };
    let ItemProviderOf::EnergyHandler(provider) = decoded else {
        panic!("wrong typed provider")
    };
    let energy = provider.provide(&mut World, &place).unwrap();
    assert_eq!(energy.amount(&mut World), 3);
    assert_eq!(energy.capacity(&mut World), 3);
}

#[test]
fn wire_loopback_bad_import_bytes_are_typed_errors() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut cx = boundary::Cx::default();
    let mut out = Writer::new();
    let id = boundary::import_id("StructureRandom", "next_int_bound").unwrap();
    assert!(matches!(
        unsafe { boundary::serve(id, &mut cx, &[0xff], &mut out) },
        Err(boundary::ServeError::Decode(_))
    ));
    assert!(out.bytes().is_empty());
}
#[test]
fn wire_loopback_owner_thread_guards_access_and_drop() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Local {
        _local: Rc<()>,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Local {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let local = boundary::OwnerThread::new(Local {
        _local: Rc::new(()),
        drops: drops.clone(),
    });
    assert!(local.get().is_ok());
    std::thread::spawn(move || {
        assert!(matches!(
            local.get(),
            Err(boundary::ServeError::WrongThread)
        ));
        drop(local);
    })
    .join()
    .unwrap();
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "foreign-thread destruction must leak the thread-affine value"
    );
    drop(boundary::OwnerThread::new(Local {
        _local: Rc::new(()),
        drops: drops.clone(),
    }));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "creator-thread destruction must release the value"
    );
}
#[test]
fn wire_loopback_roundtrip_and_atomic_writeback() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    let mut event = mod_api::LivingJumpPowerEvent {
        entity: 42,
        kind: "original",
        power: 1.5,
    };
    module.living_jump_power(&Context, &mut event);
    assert_eq!(event.power, 7.25);
    assert_eq!(event.kind, "original");
    let bad = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: true,
    };
    event.entity = 42;
    event.power = 2.0;
    bad.living_jump_power(&Context, &mut event);
    assert_eq!(
        event.power, 2.0,
        "a truncated last field must leave every host field untouched"
    );
    assert_eq!(event.entity, 42);
    let id = guest::add_handler(vec![guest::view::<dyn StructurePiece>(Arc::new(Piece))]);
    let proxy = WasmHandler { index: id, module };
    assert_eq!(
        proxy.bounding_box(),
        mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6])
    );
    let terrain = proxy
        .beardifier()
        .expect("piece terrain crosses the worker boundary");
    assert_eq!(
        terrain.bounds,
        mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6])
    );
    assert_eq!(terrain.adjustment, mod_api::TerrainAdjustment::BeardThin);
    assert_eq!(terrain.ground_level_delta, 2);
    let mut release = Writer::new();
    release.put(&vec![id]);
    assert_eq!(
        proxy.module.call(
            CallClass::Load,
            boundary::Frame::default(),
            mod_api::wire::RELEASE,
            None,
            |w| w.put_bytes(release.bytes()),
            |r| r.take::<()>()
        ),
        Some(())
    );
    assert!(
        proxy
            .module
            .call::<mod_api::BoundingBox>(
                CallClass::Worker,
                boundary::Frame::default(),
                boundary::export_id("StructurePiece", "bounding_box").unwrap(),
                Some(id),
                |_| {},
                |r| r.take()
            )
            .is_none(),
        "released handler must be refused"
    );
}

#[test]
fn wire_loopback_resource_codec_restores_context_between_groups() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    fn encode() -> Id {
        let apply: Arc<dyn mod_api::ReloadApply> = <dyn mod_api::ReloadApply>::new(|| {}).into();
        let mut writer = Writer::new();
        writer.put_owned(apply);
        assert_eq!(writer.error(), None);
        Reader::decode(writer.bytes()).unwrap()
    }
    let mut first = Box::new(boundary::Cx::default());
    let mut second = Box::new(boundary::Cx::default());
    // Both contexts stay boxed and live through their private lexical guards.
    let outer = unsafe { boundary::resource_scope(&mut *first, true) };
    let first_id = encode();
    {
        let _inner = unsafe { boundary::resource_scope(&mut *second, true) };
        let second_id = encode();
        assert_eq!(
            second_id, first_id,
            "independent tables start with the same slot"
        );
        assert!(second.held.get(second_id).is_ok());
        assert!(mod_api::wire::borrow_token().unwrap().check().is_ok());
    }
    let next_first_id = encode();
    assert_ne!(next_first_id, first_id);
    assert!(first.held.get(first_id).is_ok());
    assert!(first.held.get(next_first_id).is_ok());
    assert!(second.held.get(next_first_id).is_err());
    drop(outer);
    let apply: Arc<dyn mod_api::ReloadApply> = <dyn mod_api::ReloadApply>::new(|| {}).into();
    let mut writer = Writer::new();
    writer.put_owned(apply);
    assert!(
        writer.error().is_some(),
        "no context survives the outer guard"
    );
}

#[test]
fn wire_loopback_resource_load_restores_module_after_cross_group_unwind() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    fn load(id: Id) -> Arc<dyn StructurePiece> {
        let mut bytes = Writer::new();
        bytes.put(&id);
        Reader::decode(bytes.bytes()).unwrap()
    }
    let id = guest::add_handler(vec![guest::view::<dyn StructurePiece>(Arc::new(Piece))]);
    let mut first = Box::new(boundary::Cx::default());
    let first_module = WasmMod {
        group: 1,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    first.module = &first_module;
    let mut second = Box::new(boundary::Cx::default());
    let second_module = WasmMod {
        group: 2,
        subscribed: &[],
        listeners: &[],
        corrupt: true,
    };
    second.module = &second_module;
    let _outer = unsafe { boundary::resource_scope(&mut *first, true) };
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _inner = unsafe { boundary::resource_scope(&mut *second, true) };
        assert_eq!(load(id).bounding_box(), mod_api::BoundingBox::default());
        // Resume directly to avoid depending on the process-wide guest panic logger.
        std::panic::resume_unwind(Box::new(()));
    }));
    assert!(unwind.unwrap_err().is::<()>());
    assert_eq!(
        load(id).bounding_box(),
        mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6])
    );
}

#[test]
fn listener_metadata_and_cancellation_cross_the_real_guest() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let metadata: Vec<(String, i32, bool)> = module
        .call(
            CallClass::Load,
            boundary::Frame::default(),
            boundary::SUBSCRIBED,
            None,
            |_| {},
            |r| r.take(),
        )
        .unwrap();
    assert_eq!(metadata, vec![("render_gui_pre".into(), -37, true)]);
    let event: mod_api::RenderGuiPreEvent = Reader::new(&[1]).take().unwrap();
    let id = boundary::export_id("Hooks", "render_gui_pre").unwrap();
    let (flow, event) = module
        .call(
            CallClass::Hook,
            boundary::Frame::default(),
            id,
            None,
            |w| w.put(&event),
            |r| {
                Ok((
                    r.take::<mod_api::Flow>()?,
                    r.take::<mod_api::RenderGuiPreEvent>()?,
                ))
            },
        )
        .unwrap();
    assert_eq!(flow, mod_api::Flow::Continue);
    assert!(!event.cancelled);
}

#[test]
fn wire_loopback_render_living_pre_hidden_parts_write_back() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    struct Draw(u32);
    impl mod_api::BlockEntityDrawContext for Draw {
        fn set_light(&mut self, value: u32) {
            self.0 = value;
        }
    }
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mob = mod_api::MobRender {
        id: 1,
        kind: "test:mob",
        custom_name: Some("custom"),
        baby: false,
        partial: 0.5,
        age: 1.,
        body_yaw: 0.,
        head_yaw: 0.,
        pitch: 0.,
        walk_position: 0.,
        walk_speed: 0.,
        hurt_time: 0,
        death_time: 0,
        on_ground: true,
    };
    let mut hidden = mod_api::HiddenParts::default();
    hidden.hide_named("nose").unwrap();
    let mut draw = Draw(0);
    let mut event = mod_api::RenderLivingPreEvent {
        entity: &mob,
        draw: &mut draw,
        hidden: &mut hidden,
        cancelled: false,
    };
    module.render_living_pre(&Client, &mut event);
    assert!(!event.cancelled);
    assert!(hidden.contains_named("nose"));
    assert!(hidden.contains_named("hat_rim"));
    assert!(hidden.contains(mod_api::HumanoidPart::Head));
    // The player pre hook cancels this window's own player and lets another be drawn.
    let player = mod_api::MobRender {
        kind: "minecraft:player",
        ..mob.clone()
    };
    for (who, cancelled) in [("own", true), ("other", false)] {
        let who = mod_api::PlayerRender {
            name: who,
            local: cancelled,
        };
        let mut hidden = mod_api::HiddenParts::default();
        let mut draw = Draw(0);
        let mut event = mod_api::RenderPlayerPreEvent {
            player: &player,
            who: &who,
            draw: &mut draw,
            hidden: &mut hidden,
            cancelled: false,
        };
        module.render_player_pre(&Client, &mut event);
        assert_eq!(event.cancelled, cancelled);
        assert_eq!(draw.0, who.name.len() as u32);
    }
}

#[test]
fn wrong_phase_service_call_is_a_named_refusal_not_a_panic() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Packages);
    let id = boundary::import_id("Messages", "text_allowed").unwrap();
    let mut cx = boundary::Cx::default();
    let mut input = Writer::new();
    input.put("hello");
    let mut out = Writer::new();
    assert!(matches!(
        unsafe { boundary::serve(id, &mut cx, input.bytes(), &mut out) },
        Err(boundary::ServeError::Phase {
            operation: "messages",
            phase: mod_api::__private::ModPhase::Packages
        })
    ));
    assert!(out.bytes().is_empty());
}

#[test]
fn wrong_phase_fallible_call_returns_unavailable_to_guest() {
    mod_api::__private::initialize().unwrap();
    use mod_api::prelude::*;
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Packages);
    let cx = std::cell::RefCell::new(boundary::Cx::default());
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let level = mod_api::LevelRef::new(
        mod_api::WorldEpoch::new(1),
        mod_api::DimensionKey::OVERWORLD,
    );
    let result = guest::Guest.level_game_rules(level);
    guest::set_transport(None);
    assert_eq!(result, Err(mod_api::LevelReadError::Unavailable));
}

#[test]
fn wire_loopback_entity_by_uuid_returns_a_handle_or_absence() {
    use mod_api::{EntityRef, EntityUuid, ServerHost, WorldEpoch};

    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);

    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl ServerHost for World {}
    mod_api::impl_server_host_areas!(World, except(entities));
    impl mod_api::prelude::EntitiesServerHost for World {
        fn entity_by_uuid(&self, uuid: EntityUuid) -> Option<EntityRef> {
            (uuid.0 == 0xfedc_ba98_7654_3210_0123_4567_89ab_cdef)
                .then(|| EntityRef::new(WorldEpoch::new(4), 19))
        }
    }

    let mut frame = boundary::Frame::default();
    frame.set_server_host(&World);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));

    let host: &dyn ServerHost = &guest::Guest;
    let found = host.entity_by_uuid(EntityUuid(0xfedc_ba98_7654_3210_0123_4567_89ab_cdef));
    let missing = host.entity_by_uuid(EntityUuid(u128::MAX));
    guest::set_transport(None);
    assert_eq!(
        found.map(|entity| (entity.epoch().get(), entity.id())),
        Some((4, 19))
    );
    assert_eq!(missing, None);
}

#[test]
fn world_phase_messages_round_trip_returns_filter_decision() {
    mod_api::__private::initialize().unwrap();
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World, except(services));
    impl mod_api::prelude::ServicesServerHost for World {
        fn messages(&self) -> &dyn mod_api::Messages {
            self
        }
    }
    impl mod_api::Messages for World {
        fn text_allowed(&self, text: &str) -> Option<bool> {
            assert_eq!(text, "blocked by server filter");
            Some(false)
        }
    }
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let world = World;
    let mut frame = boundary::Frame::default();
    frame.set_server_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let id = boundary::import_id("Messages", "text_allowed").unwrap();
    let mut input = Writer::new();
    input.put("blocked by server filter");
    let mut out = Writer::new();
    unsafe { boundary::serve(id, &mut cx, input.bytes(), &mut out) }.unwrap();
    assert_eq!(
        Reader::decode::<Option<bool>>(out.bytes()).unwrap(),
        Some(false)
    );
}

#[test]
fn wire_loopback_block_entity_data_received_preserves_tag_and_handle() {
    thread_local! { static CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    use horizon_core::nbt::Tag;
    use mod_api::BlockEntityPart;
    struct Notice;
    impl mod_api::BlockPart for Notice {
        fn block_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[]
        }
    }
    impl BlockEntityPart for Notice {
        fn block_entity_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[Some(mod_api::HookId::BlockEntityDataReceived)]
        }
        fn block_entity_data_received(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut mod_api::BlockEntityDataReceivedEvent<'_>,
        ) {
            assert_eq!(event.tag.get(&"count".into()), Some(&Tag::Int(3)));
            assert_eq!(event.entity, notice_handle());
            assert_eq!(event.source, mod_api::BlockEntityDataSource::Chunk);
            CALLS.set(CALLS.get() + 1);
        }
    }
    fn notice_handle() -> mod_api::BlockEntityRef {
        mod_api::__private::game::block_entity_ref(
            mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(1),
                mod_api::DimensionKey::OVERWORLD,
            ),
            [1, 64, 1],
            1,
        )
    }
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    // The platform phase this scope sets is process-wide state another test may not have installed yet.
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let index = guest::add_handler(vec![guest::view::<dyn BlockEntityPart>(Arc::new(Notice))]);
    let proxy = WasmHandler {
        index,
        module: WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    let tag = [("count".into(), Tag::Int(3))].into();
    proxy.block_entity_data_received(
        &Client,
        &mut mod_api::BlockEntityDataReceivedEvent {
            entity: notice_handle(),
            source: mod_api::BlockEntityDataSource::Chunk,
            tag: &tag,
        },
    );
    assert_eq!(CALLS.get(), 1);
}

#[test]
fn datapack_sync_recipient_and_answer_cross_the_wire() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    for uuid in [12, 13] {
        let player = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), uuid as u32),
            uuid,
        );
        let mut types = Vec::new();
        module.datapack_sync(
            &mut World,
            &mut mod_api::DatapackSyncEvent {
                player,
                recipe_types: &mut types,
            },
        );
        assert_eq!(
            types,
            if uuid == 12 {
                vec!["testmod:grinding".to_owned()]
            } else {
                vec![]
            }
        );
    }
}
