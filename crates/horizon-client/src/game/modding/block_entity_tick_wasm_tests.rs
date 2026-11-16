use mod_api::{
    BlockEntityPart, BlockEntityTickEvent, EntityRef, ItemEntityRef, ServerHost,
    wire::{Id, Writer},
};

pub(super) fn tick(
    host: &mut dyn ServerHost,
    event: &mut BlockEntityTickEvent<'_>,
    first: EntityRef,
    second: EntityRef,
    stack: mod_api::ItemStack,
    mode: &str,
) {
    let mut set = Writer::new();
    set.put(&ItemEntityRef::new(first));
    set.put(&stack);
    let mut remove = Writer::new();
    remove.put(&second);
    let escape = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect::<String>()
    };
    let setter =
        horizon_mod_wasm::fixture::import_id("BackendMut", "item_entity_set_stack").unwrap();
    let discard = horizon_mod_wasm::fixture::import_id("BackendMut", "entity_discard").unwrap();
    let mut fake = mod_api::ModBlockEntity::new("wrong:identity".into(), Default::default());
    fake.data.push(6);
    let mut six = stack.clone();
    six.set_count(6).unwrap();
    fake.items = vec![Some(six)];
    let mut wrong_revision = Writer::new();
    wrong_revision.put(&mod_api::BlockEntityTickReply::Changed);
    wrong_revision.put(&event.pos);
    wrong_revision.put(&mod_api::item::containers::ModBlockEntitySnapshot::from(
        &fake,
    ));
    let output = match mode {
        "wasm_malformed" => "i64.const 0".to_owned(),
        "wasm_identity_readonly" => format!("i64.const 32768 i64.const 32 i64.shl i64.const {} i64.or", wrong_revision.bytes().len()),
        _ => "i64.const 520 i64.const 32 i64.shl global.get $length i32.const 8 i32.sub i64.extend_i32_u i64.or".to_owned(),
    };
    let bytes = wat::parse_str(format!(r#"(module
      (@custom "dylink.0" "\01\04\00\00\00\00")
      (import "env" "memory" (memory 1))
      (import "env" "__indirect_function_table" (table 1 funcref))
      (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
      (import "horizon" "host_reply" (func (param i32)))
      (func (export "horizon_layout") (result i64) i64.const {layout})
      (global $length (mut i32) (i32.const 0))
      (data (i32.const 4096) "{set}")
      (data (i32.const 8192) "{remove}")
      (data (i32.const 32768) "{revision}")
      (func (export "horizon_in") (param i32) (result i32) local.get 0 global.set $length i32.const 512)
      (func (export "horizon_hook") (param i32) (result i64)
        i32.const {setter} i32.const 4096 i32.const {set_len} i32.const 16384 i32.const 4096 call $host drop
        i32.const {discard} i32.const 8192 i32.const {remove_len} i32.const 16384 i32.const 4096 call $host drop
        {output})
    )"#, layout=*mod_api::wire::LAYOUT, revision=escape(wrong_revision.bytes()), set=escape(set.bytes()), remove=escape(remove.bytes()), set_len=set.bytes().len(), remove_len=remove.bytes().len())).unwrap();
    let module = horizon_mod_wasm::fixture::module(&bytes).unwrap();
    let handler = horizon_mod_wasm::WasmHandler::new(&module, Id(1));
    handler.block_entity_tick(host, event);
}
