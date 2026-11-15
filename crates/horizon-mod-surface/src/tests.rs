use super::*;

fn function(id: &str, refs: &[&str]) -> Function {
    Function::new(
        id,
        "",
        FunctionContract {
            signature: format!("fn {}()", leaf(id)),
            impl_generics: String::new(),
            wasm: true,
            refs: refs.iter().map(|r| r.to_string()).collect(),
        },
    )
}

fn field(id: &str, ty: &str, refs: &[&str]) -> Field {
    Field::new(
        id,
        "",
        FieldContract {
            ty: ty.into(),
            refs: refs.iter().map(|r| r.to_string()).collect(),
        },
    )
}

fn record_type(id: &str, fields: Vec<Field>) -> Type {
    Type::new(
        id,
        "",
        None,
        TypeContract {
            kind: TypeKind::Struct,
            generics: String::new(),
            wire: false,
            wire_form: None,
            constructible: false,
            non_exhaustive: false,
            target: None,
            members: vec![],
            refs: vec![],
        },
        fields,
        vec![],
    )
}

fn sample() -> Surface {
    Surface::new(
        vec![],
        vec![],
        vec![],
        vec![
            function("fn:world::ground", &["type:Block"]),
            function("fn:spawn", &[]),
        ],
        vec![],
        vec![
            record_type(
                "type:Block",
                vec![field("type:Block.state", "State", &["type:State"])],
            ),
            record_type("type:State", vec![]),
            record_type("type:Unrelated", vec![]),
        ],
        vec![],
        vec![Capability::new(
            "capability:horizon:fluids",
            "",
            CapabilityContract {
                kind: CapabilityKind::Block,
                constant: Some("block::FLUIDS".into()),
                handler: None,
                context: None,
                refs: vec![],
            },
        )],
    )
}

#[test]
fn the_document_round_trips_and_its_hash_covers_every_entry() {
    let surface = sample();
    let text = surface.to_json();
    assert!(text.ends_with("}\n"));
    assert_eq!(Surface::read(text.as_bytes()).unwrap(), surface);
    let mut entries: Vec<_> = surface.entries().map(|(id, _)| id).collect();
    entries.sort_unstable();
    assert_eq!(
        entries,
        [
            "capability:horizon:fluids",
            "fn:spawn",
            "fn:world::ground",
            "type:Block",
            "type:Block.state",
            "type:State",
            "type:Unrelated",
        ]
    );
    let mut changed = surface.clone();
    changed.types[0].fields[0] = field("type:Block.state", "u8", &[]);
    assert_ne!(changed.document_hash(), surface.document_hash());
}

#[test]
fn the_closure_follows_references_through_members() {
    let surface = sample();
    let closure = surface.closure(["fn:world::ground", "fn:not_there"]);
    assert_eq!(
        closure.into_iter().collect::<Vec<_>>(),
        [
            "fn:not_there",
            "fn:world::ground",
            "type:Block",
            "type:State"
        ]
    );
}

#[test]
fn leaves_are_what_a_mod_writes() {
    for (id, leaf_name) in [
        ("hook:player_logged_in", "player_logged_in"),
        ("trait:host::items::ItemsHost", "ItemsHost"),
        ("method:Hooks::tick", "tick"),
        ("handle:entity::EntityRef::position", "position"),
        ("fn:world::ground", "ground"),
        ("macro:horizon_mod", "horizon_mod"),
        ("type:entity::EntityRef", "EntityRef"),
        ("type:BlockClick.hand", "hand"),
        ("type:Phase.Load", "Load"),
        ("part:block_entity", "block_entity"),
        ("capability:horizon:fluids", "horizon:fluids"),
    ] {
        assert_eq!(leaf(id), leaf_name, "{id}");
    }
    let surface = sample();
    let identifiers: Vec<_> = surface.identifiers().collect();
    assert!(identifiers.contains(&("FLUIDS", "capability:horizon:fluids")));
    assert!(identifiers.contains(&("state", "type:Block.state")));
}

#[test]
fn the_reader_refuses_with_a_typed_error() {
    let text = sample().to_json();
    let older = text.replacen("\"schema\": 2", "\"schema\": 1", 1);
    assert_eq!(Surface::read(older.as_bytes()), Err(ReadError::Schema(1)));
    assert_eq!(
        ReadError::Schema(1).to_string(),
        "surface schema 1, this tool reads 2"
    );
    assert!(matches!(
        Surface::read(&text.as_bytes()[..text.len() / 2]),
        Err(ReadError::Json(_))
    ));
    assert!(matches!(
        Surface::read(b"\x00\xffnot json"),
        Err(ReadError::Json(_))
    ));
    assert!(matches!(Surface::read(b"{}"), Err(ReadError::Json(_))));
    let wrong_type = text.replacen("\"wasm\": true", "\"wasm\": \"yes\"", 1);
    assert!(matches!(
        Surface::read(wrong_type.as_bytes()),
        Err(ReadError::Json(_))
    ));
    let doubled = text.replacen("\"fn:spawn\"", "\"fn:world::ground\"", 1);
    assert_eq!(
        Surface::read(doubled.as_bytes()),
        Err(ReadError::DuplicateId("fn:world::ground".into()))
    );
}

#[test]
fn unknown_fields_are_ignored() {
    let text = sample().to_json();
    let extended = text
        .replacen("{", "{\n  \"future\": {\"x\": [1, 2]},", 1)
        .replacen(
            "\"signature\": \"fn spawn()\"",
            "\"signature\": \"fn spawn()\", \"later\": 3",
            1,
        );
    assert_ne!(extended, text);
    assert_eq!(Surface::read(extended.as_bytes()).unwrap(), sample());
}

#[test]
fn the_contract_hash_leaves_out_doc_and_info_fields() {
    let contract = FunctionContract {
        signature: "fn f(x: u8)".into(),
        impl_generics: String::new(),
        wasm: false,
        refs: vec![],
    };
    let documented = Function::new("fn:f", "Does f.", contract.clone());
    let bare = Function::new("fn:g", "", contract);
    assert_eq!(documented.hash, bare.hash);
    // Independent of the code: SHA-256 of the compact JSON, first 16 bytes.
    use sha2::{Digest, Sha256};
    let expected: String = Sha256::digest(br#"{"signature":"fn f(x: u8)","wasm":false}"#)[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(bare.hash, expected);
}

#[test]
fn deeply_nested_json_neither_overflows_nor_passes_as_records() {
    let deep = |field: &str| {
        let mut text = format!("{{\"schema\": {SCHEMA}, \"hash\": \"\", \"{field}\": ");
        text.push_str(&"[".repeat(100_000));
        text.push_str(&"]".repeat(100_000));
        text.push('}');
        text
    };
    // In a record list: refused, without exhausting the stack.
    assert!(matches!(
        Surface::read(deep("hooks").as_bytes()),
        Err(ReadError::Json(_))
    ));
    // In a field this reader does not know: skipped.
    assert!(Surface::read(deep("future").as_bytes()).is_ok());
}

fn api_trait(id: &str, role: Role, bounds: &[&str], methods: &[&str]) -> Trait {
    Trait::new(
        id,
        "",
        TraitContract {
            role,
            side: Side::Both,
            generics: String::new(),
            bounds: bounds.iter().map(|b| leaf(b).to_owned()).collect(),
            required: vec![],
            refs: bounds.iter().map(|b| b.to_string()).collect(),
        },
        methods
            .iter()
            .map(|name| {
                Method::new(
                    format!("method:{}::{name}", leaf(id)),
                    "",
                    MethodContract {
                        signature: format!("fn {name}(&self)"),
                        required: false,
                        wasm: true,
                        refs: vec![id.to_owned()],
                    },
                )
            })
            .collect(),
        vec![],
    )
}

fn traits(list: Vec<Trait>) -> Surface {
    Surface::new(vec![], list, vec![], vec![], vec![], vec![], vec![], vec![])
}

/// The ids of `before` that `after` lacks or hashes differently.
fn differences(before: &Surface, after: &Surface) -> Vec<String> {
    let after: BTreeMap<&str, &str> = after.entries().collect();
    let mut out: Vec<String> = before
        .entries()
        .filter(|(id, hash)| after.get(id) != Some(hash))
        .map(|(id, _)| id.to_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn a_called_trait_gaining_a_supertrait_changes_no_existing_hash() {
    let before = traits(vec![
        api_trait("trait:Host", Role::Call, &["trait:PlayersHost"], &[]),
        api_trait("trait:PlayersHost", Role::Call, &[], &["player"]),
    ]);
    let after = traits(vec![
        api_trait(
            "trait:Host",
            Role::Call,
            &["trait:PlayersHost", "trait:FluidsHost"],
            &[],
        ),
        api_trait("trait:PlayersHost", Role::Call, &[], &["player"]),
        api_trait("trait:FluidsHost", Role::Call, &[], &["fluid"]),
    ]);
    assert_eq!(differences(&before, &after), Vec::<String>::new());
}

#[test]
fn a_called_trait_losing_a_supertrait_shows_only_what_went() {
    let before = traits(vec![
        api_trait(
            "trait:Host",
            Role::Call,
            &["trait:PlayersHost", "trait:FluidsHost"],
            &[],
        ),
        api_trait("trait:PlayersHost", Role::Call, &[], &["player"]),
        api_trait("trait:FluidsHost", Role::Call, &[], &["fluid"]),
    ]);
    let after = traits(vec![
        api_trait("trait:Host", Role::Call, &["trait:PlayersHost"], &[]),
        api_trait("trait:PlayersHost", Role::Call, &[], &["player"]),
    ]);
    assert_eq!(
        differences(&before, &after),
        ["method:FluidsHost::fluid", "trait:FluidsHost"]
    );
    // The closure does not go from a called trait into its supertraits; a method still reaches
    // its own trait.
    let closure: Vec<String> = before
        .closure(["trait:Host", "method:PlayersHost::player"])
        .into_iter()
        .collect();
    assert_eq!(
        closure,
        [
            "method:PlayersHost::player",
            "trait:Host",
            "trait:PlayersHost"
        ]
    );
}

#[test]
fn an_implemented_trait_gaining_a_supertrait_changes_its_hash_and_leads_to_it() {
    let before = traits(vec![
        api_trait("trait:BlockEntityPart", Role::Implement, &[], &[]),
        api_trait("trait:BlockPart", Role::Implement, &[], &[]),
    ]);
    let after = traits(vec![
        api_trait(
            "trait:BlockEntityPart",
            Role::Implement,
            &["trait:BlockPart"],
            &[],
        ),
        api_trait("trait:BlockPart", Role::Implement, &[], &[]),
    ]);
    assert_eq!(differences(&before, &after), ["trait:BlockEntityPart"]);
    let closure: Vec<String> = after
        .closure(["trait:BlockEntityPart"])
        .into_iter()
        .collect();
    assert_eq!(closure, ["trait:BlockEntityPart", "trait:BlockPart"]);
}

fn hook_with(version: u64, arguments: &[(&str, &str)]) -> Hook {
    Hook::new(
        "hook:joined",
        "",
        HookEngine::default(),
        Counterparts::default(),
        HookContract {
            version,
            side: Side::Server,
            contract: HookKind::Listen,
            scope: None,
            cancellable: None,
            context: None,
            host: None,
            owner: None,
            answered_by_mod: false,
            arguments: arguments
                .iter()
                .map(|(name, ty)| Argument {
                    name: name.to_string(),
                    ty: ty.to_string(),
                })
                .collect(),
            output: None,
            refs: vec!["type:PlayerRef".into()],
        },
    )
}

fn hooks(list: Vec<Hook>) -> Surface {
    Surface::new(list, vec![], vec![], vec![], vec![], vec![], vec![], vec![])
}

#[test]
fn a_hooks_arguments_are_members_and_its_version_is_outside_its_hash() {
    let before = hooks(vec![hook_with(1, &[("player", "PlayerRef")])]);
    let ids: Vec<&str> = before.entries().map(|(id, _)| id).collect();
    assert_eq!(ids, ["hook:joined", "hook:joined.player"]);
    assert_eq!(leaf("hook:joined.player"), "player");
    // The member references the hook and the types its own type names.
    assert_eq!(
        before
            .closure(["hook:joined.player"])
            .into_iter()
            .collect::<Vec<_>>(),
        ["hook:joined", "hook:joined.player", "type:PlayerRef"]
    );
    // A new argument and a new version change no existing hash.
    let added = hooks(vec![hook_with(
        2,
        &[("player", "PlayerRef"), ("name", "&'a str")],
    )]);
    assert_eq!(differences(&before, &added), Vec::<String>::new());
    // A retyped argument changes only that argument.
    let retyped = hooks(vec![hook_with(1, &[("player", "u128")])]);
    assert_eq!(differences(&before, &retyped), ["hook:joined.player"]);
}

#[test]
fn a_hooks_own_hash_covers_what_every_listener_relies_on() {
    let base = hook_with(1, &[]);
    let mut cancellable = base.contract.clone();
    cancellable.cancellable = Some(true);
    let mut side = base.contract.clone();
    side.side = Side::Both;
    let mut output = base.contract.clone();
    output.output = Some(Argument {
        name: "answer".into(),
        ty: "u8".into(),
    });
    for changed in [cancellable, side, output] {
        let other = Hook::new(
            "hook:joined",
            "",
            HookEngine::default(),
            Counterparts::default(),
            changed,
        );
        assert_ne!(other.hash, base.hash);
    }
}

fn plain_struct(fields: &[(&str, &str)]) -> Type {
    Type::new(
        "type:Settings",
        "",
        None,
        TypeContract {
            kind: TypeKind::Struct,
            generics: String::new(),
            wire: false,
            wire_form: None,
            constructible: true,
            non_exhaustive: false,
            target: None,
            members: vec![],
            refs: vec![],
        },
        fields
            .iter()
            .map(|(name, ty)| field(&format!("type:Settings.{name}"), ty, &["type:Settings"]))
            .collect(),
        vec![],
    )
}

#[test]
fn a_structs_field_set_is_its_fields_hash_not_its_own_hash() {
    let before = plain_struct(&[("height", "i32")]);
    let after = plain_struct(&[("height", "i32"), ("sky", "bool")]);
    assert_eq!(before.hash, after.hash);
    assert_ne!(before.fields_hash, after.fields_hash);
    // Independent of the code: SHA-256 of `name: type` lines, first 16 bytes.
    use sha2::{Digest, Sha256};
    let expected: String = Sha256::digest(b"height: i32\n")[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(before.fields_hash.as_deref(), Some(expected.as_str()));
    // A literal names fields in any order: moving one changes nothing.
    let moved = plain_struct(&[("sky", "bool"), ("height", "i32")]);
    assert_eq!(moved.fields_hash, after.fields_hash);
    // Its trait impls are a set beside the hash.
    let mut with_impl = plain_struct(&[("height", "i32")]);
    with_impl.impls = vec!["Hash".into()];
    assert_eq!(
        Surface::read(types_doc(with_impl.clone()).to_json().as_bytes())
            .unwrap()
            .types[0]
            .impls,
        ["Hash"]
    );
    assert_eq!(with_impl.hash, before.hash);
}

fn types_doc(item: Type) -> Surface {
    Surface::new(
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![item],
        vec![],
        vec![],
    )
}

#[test]
fn a_trait_whose_role_is_not_known_hashes_like_one_mods_implement() {
    let with = |role: Role, required: &[&str]| {
        Trait::new(
            "trait:ChunkGenerator",
            "",
            TraitContract {
                role,
                side: Side::Both,
                generics: String::new(),
                bounds: vec![],
                required: required
                    .iter()
                    .map(|name| Required {
                        name: name.to_string(),
                        signature: format!("fn {name}(&self) -> i32"),
                    })
                    .collect(),
                refs: vec![],
            },
            vec![],
            vec![],
        )
        .hash
    };
    assert_ne!(
        with(Role::None, &["fill"]),
        with(Role::None, &["fill", "height"])
    );
    // A bound on it counts too.
    let bounded = api_trait("trait:ChunkGenerator", Role::None, &["trait:Debug"], &[]);
    let plain = api_trait("trait:ChunkGenerator", Role::None, &[], &[]);
    assert_ne!(bounded.hash, plain.hash);
}

#[test]
fn how_a_type_crosses_and_whether_a_struct_can_be_built_are_outside_its_hash() {
    let base = plain_struct(&[("height", "i32")]);
    let with = |edit: fn(&mut TypeContract)| {
        let mut contract = base.contract.clone();
        edit(&mut contract);
        Type::new(
            "type:Settings",
            "",
            None,
            contract,
            base.fields.clone(),
            vec![],
        )
        .hash
    };
    assert_eq!(with(|c| c.wire = true), base.hash);
    assert_eq!(with(|c| c.wire_form = Some("record".into())), base.hash);
    assert_eq!(with(|c| c.constructible = false), base.hash);
    assert_eq!(with(|c| c.non_exhaustive = true), base.hash);
    // An enum's `non_exhaustive` is in its hash: it decides whether a `match` needs a wildcard.
    let enum_with = |non_exhaustive: bool| {
        let mut contract = base.contract.clone();
        contract.kind = TypeKind::Enum;
        contract.non_exhaustive = non_exhaustive;
        Type::new("type:Settings", "", None, contract, vec![], vec![]).hash
    };
    assert_ne!(enum_with(true), enum_with(false));
}

#[test]
fn an_argument_references_only_the_items_its_own_type_names_by_path() {
    let mut contract = hook_with(1, &[]).contract;
    contract.arguments = vec![
        Argument {
            name: "left".into(),
            ty: "a::Item".into(),
        },
        Argument {
            name: "right".into(),
            ty: "&'a mut Vec<b::Item>".into(),
        },
    ];
    contract.refs = vec!["type:a::Item".into(), "type:b::Item".into()];
    let hook = Hook::new(
        "hook:joined",
        "",
        HookEngine::default(),
        Counterparts::default(),
        contract,
    );
    let refs: Vec<&[String]> = hook
        .fields
        .iter()
        .map(|f| f.contract.refs.as_slice())
        .collect();
    assert_eq!(
        refs,
        [
            &["hook:joined".to_owned(), "type:a::Item".to_owned()][..],
            &["hook:joined".to_owned(), "type:b::Item".to_owned()][..],
        ]
    );
}
