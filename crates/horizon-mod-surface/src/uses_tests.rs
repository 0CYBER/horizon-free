use crate::uses::*;
use crate::*;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn method(id: &str, signature: &str, refs: &[&str]) -> Method {
    Method::new(
        id,
        "",
        MethodContract {
            signature: signature.into(),
            required: false,
            wasm: true,
            refs: strings(refs),
        },
    )
}

fn handle(id: &str, signature: &str, refs: &[&str]) -> Function {
    Function::new(
        id,
        "",
        FunctionContract {
            signature: signature.into(),
            impl_generics: String::new(),
            wasm: true,
            refs: strings(refs),
        },
    )
}

fn api_trait(id: &str, role: Role, bounds: &[&str], methods: Vec<Method>) -> Trait {
    Trait::new(
        id,
        "",
        TraitContract {
            role,
            side: Side::Both,
            generics: String::new(),
            bounds: bounds.iter().map(|b| leaf(b).to_owned()).collect(),
            required: vec![],
            refs: strings(bounds),
        },
        methods,
        vec![],
    )
}

fn field(id: &str, ty: &str, refs: &[&str]) -> Field {
    Field::new(
        id,
        "",
        FieldContract {
            ty: ty.into(),
            refs: strings(refs),
        },
    )
}

/// A struct mods can build when `constructible`.
pub(crate) fn api_type(id: &str, generics: &str, constructible: bool, fields: Vec<Field>) -> Type {
    Type::new(
        id,
        "",
        None,
        TypeContract {
            kind: TypeKind::Struct,
            generics: generics.into(),
            wire: false,
            wire_form: None,
            constructible,
            non_exhaustive: false,
            target: None,
            members: vec![],
            refs: vec![],
        },
        fields,
        vec![],
    )
}

/// An exhaustive enum with unit variants.
fn api_enum(id: &str, variants: &[&str]) -> Type {
    Type::new(
        id,
        "",
        None,
        TypeContract {
            kind: TypeKind::Enum,
            generics: String::new(),
            wire: false,
            wire_form: None,
            constructible: true,
            non_exhaustive: false,
            target: None,
            members: variants.iter().map(|v| v.to_string()).collect(),
            refs: vec![],
        },
        vec![],
        variants
            .iter()
            .map(|v| {
                Variant::new(
                    format!("{id}.{v}"),
                    "",
                    VariantContract {
                        fields: vec![],
                        named: false,
                        refs: vec![id.to_owned()],
                    },
                )
            })
            .collect(),
    )
}

fn hook(name: &str, argument_type: &str, refs: &[&str]) -> Hook {
    Hook::new(
        format!("hook:{name}"),
        "",
        HookEngine::default(),
        Counterparts::default(),
        HookContract {
            version: 1,
            side: Side::Server,
            contract: HookKind::Listen,
            scope: None,
            cancellable: None,
            context: None,
            host: None,
            owner: None,
            answered_by_mod: false,
            arguments: vec![Argument {
                name: "player".into(),
                ty: argument_type.into(),
            }],
            output: None,
            refs: strings(refs),
        },
    )
}

/// The pieces of a hand-written surface, changed per test before [`Pieces::build`].
#[derive(Clone)]
pub(crate) struct Pieces {
    hooks: Vec<Hook>,
    traits: Vec<Trait>,
    handles: Vec<Function>,
    pub(crate) types: Vec<Type>,
    capabilities: Vec<Capability>,
}

impl Pieces {
    pub(crate) fn build(self) -> Surface {
        Surface::new(
            self.hooks,
            self.traits,
            self.handles,
            vec![],
            // The API's own attribute macro, which a mod may put on its items.
            vec![Macro::new(
                "macro:horizon_mod",
                "",
                MacroContract {
                    kind: MacroKind::Attribute,
                    attributes: vec![],
                },
            )],
            self.types,
            vec![],
            self.capabilities,
        )
    }

    pub(crate) fn replace_type(&mut self, new: Type) {
        let slot = self.types.iter_mut().find(|t| t.id == new.id).unwrap();
        *slot = new;
    }
}

fn event_type() -> Type {
    api_type(
        "type:PlayerLoggedInEvent",
        "",
        true,
        vec![field(
            "type:PlayerLoggedInEvent.player",
            "PlayerRef",
            &["type:PlayerLoggedInEvent", "type:PlayerRef"],
        )],
    )
}

fn host_trait(extra: Vec<Method>) -> Trait {
    let mut methods = vec![method(
        "method:Host::level_name",
        "fn level_name(&self) -> String",
        &["trait:Host"],
    )];
    methods.extend(extra);
    api_trait("trait:Host", Role::Call, &[], methods)
}

pub(crate) fn base() -> Pieces {
    Pieces {
        hooks: vec![
            hook(
                "player_logged_in",
                "PlayerRef",
                &["type:PlayerLoggedInEvent"],
            ),
            hook("block_broken", "PlayerRef", &[]),
        ],
        traits: vec![
            host_trait(vec![]),
            api_trait(
                "trait:ServerHost",
                Role::Call,
                &["trait:Host"],
                vec![method(
                    "method:ServerHost::give_item",
                    "fn give_item(&mut self, _: PlayerRef, _: ItemStack) -> bool",
                    &["trait:ServerHost", "type:PlayerRef", "type:ItemStack"],
                )],
            ),
            api_trait(
                "trait:Hooks",
                Role::Implement,
                &[],
                vec![method(
                    "method:Hooks::player_logged_in",
                    "fn player_logged_in(&self, _: &mut dyn ServerHost, _: &mut PlayerLoggedInEvent)",
                    &[
                        "trait:Hooks",
                        "trait:ServerHost",
                        "type:PlayerLoggedInEvent",
                    ],
                )],
            ),
            api_trait(
                "trait:Particles",
                Role::Call,
                &[],
                vec![method(
                    "method:Particles::tell",
                    "fn tell(&self, _: u32)",
                    &["trait:Particles"],
                )],
            ),
        ],
        handles: vec![
            handle(
                "handle:PlayerRef::tell",
                "fn tell(self, _: &mut dyn ServerHost, _: Text) -> bool",
                &["type:PlayerRef", "trait:ServerHost", "type:Text"],
            ),
            handle(
                "handle:PlayerRef::inventory",
                "fn inventory(self) -> Inventory",
                &["type:PlayerRef", "type:Inventory"],
            ),
        ],
        types: vec![
            event_type(),
            api_type("type:PlayerRef", "", false, vec![]),
            api_type(
                "type:ItemStack",
                "",
                true,
                vec![field("type:ItemStack.count", "u8", &["type:ItemStack"])],
            ),
            api_type(
                "type:Text",
                "",
                false,
                vec![field("type:Text.content", "String", &["type:Text"])],
            ),
            api_type("type:Inventory", "", false, vec![]),
            api_type("type:Unrelated", "", false, vec![]),
        ],
        capabilities: vec![Capability::new(
            "capability:registries",
            "",
            CapabilityContract {
                kind: CapabilityKind::Manifest,
                constant: None,
                handler: None,
                context: None,
                refs: vec![],
            },
        )],
    }
}

/// A tuple struct mods can build.
pub(crate) fn api_tuple(id: &str, types: &[&str]) -> Type {
    Type::new(
        id,
        "",
        None,
        TypeContract {
            kind: TypeKind::TupleStruct,
            generics: String::new(),
            wire: false,
            wire_form: None,
            constructible: true,
            non_exhaustive: false,
            target: None,
            members: vec![],
            refs: vec![],
        },
        types
            .iter()
            .enumerate()
            .map(|(i, ty)| field(&format!("{id}.{i}"), ty, &[id]))
            .collect(),
        vec![],
    )
}

pub(crate) fn settings(fields: &[(&str, &str)], impls: &[&str]) -> Type {
    let mut record = api_type(
        "type:Settings",
        "",
        true,
        fields
            .iter()
            .map(|(name, ty)| field(&format!("type:Settings.{name}"), ty, &["type:Settings"]))
            .collect(),
    );
    record.impls = impls.iter().map(|i| i.to_string()).collect();
    record
}

/// The base surface with the types the building and path tests use.
pub(crate) fn wider() -> Pieces {
    let mut pieces = base();
    pieces
        .types
        .push(settings(&[("height", "i32")], &["Clone", "Default"]));
    pieces.types.push(api_tuple("type:Pos", &["i32", "i32"]));
    pieces.types.push(api_type("type:Item", "", false, vec![]));
    pieces
        .types
        .push(api_enum("type:TextHover", &["Item", "Plain"]));
    pieces
}

const MOD: &str = r#"use mod_api::{Hooks, ItemStack, PlayerLoggedInEvent, ServerHost};

struct Mod;

impl Hooks for Mod {
    // block_broken is for later; Unrelated too.
    fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent) {
        let stack = ItemStack { count: 1 };
        let _ = event.player.inventory();
        host.give_item(event.player, stack);
        let _ = "Particles and block_broken in a string";
    }
}
"#;

pub(crate) fn manifest(names: &[&str]) -> ManifestFacts {
    ManifestFacts {
        names: names.iter().map(|n| n.to_string()).collect(),
        hook_versions: Default::default(),
    }
}

fn scan_on(pieces: Pieces, source: &str) -> Uses {
    uses(
        [("src/lib.rs", source)],
        &manifest(&["registries"]),
        &pieces.build(),
    )
    .unwrap()
}

fn scan(source: &str) -> Uses {
    scan_on(base(), source)
}

fn ids(uses: &Uses) -> Vec<&str> {
    uses.entries().map(|e| e.id.as_str()).collect()
}

pub(crate) fn entry<'a>(uses: &'a Uses, id: &str) -> &'a Entry {
    uses.entries().find(|e| e.id == id).unwrap()
}

fn verdict(target: Pieces) -> Fit {
    fits(scan(MOD).entries(), &target.build())
}

fn misfit(id: &str, change: Change) -> Misfit {
    Misfit {
        id: id.into(),
        change,
    }
}

fn words(fit: Fit) -> Vec<String> {
    match fit {
        Fit::Fits => vec![],
        Fit::DoesNotFit(misfits) => misfits.iter().map(ToString::to_string).collect(),
    }
}

#[test]
fn the_list_is_what_the_source_and_manifest_name_and_what_their_signatures_reach() {
    let uses = scan(MOD);
    assert_eq!(
        ids(&uses),
        [
            "capability:registries",
            "handle:PlayerRef::inventory",
            "hook:player_logged_in",
            "hook:player_logged_in.player",
            "method:Hooks::player_logged_in",
            "method:ServerHost::give_item",
            "trait:Hooks",
            "trait:ServerHost",
            "type:Inventory",
            "type:ItemStack",
            "type:ItemStack.count",
            "type:PlayerLoggedInEvent",
            "type:PlayerLoggedInEvent.player",
            "type:PlayerRef",
        ]
    );
    let origin = |id: &str| {
        uses.items
            .iter()
            .find(|u| u.entry.id == id)
            .unwrap()
            .origin
            .clone()
    };
    let at = |line| Origin::Source {
        file: "src/lib.rs".into(),
        line,
    };
    assert_eq!(origin("trait:Hooks"), at(1));
    assert_eq!(origin("method:ServerHost::give_item"), at(10));
    assert_eq!(origin("type:PlayerLoggedInEvent.player"), at(9));
    assert_eq!(origin("hook:player_logged_in.player"), at(9));
    assert_eq!(origin("capability:registries"), Origin::Manifest);
    assert_eq!(
        origin("type:Inventory"),
        Origin::Via("handle:PlayerRef::inventory".into())
    );
    let base = base().build();
    let hashes: std::collections::BTreeMap<&str, &str> = base.entries().collect();
    for entry in uses.entries() {
        assert_eq!(entry.hash, hashes[entry.id.as_str()], "{}", entry.id);
    }
    // How each named struct is built: `ItemStack` with a full literal on line 8; the event and
    // `PlayerRef` not at all.
    let stack = entry(&uses, "type:ItemStack");
    assert_eq!(stack.built, Some(Built::Full));
    assert_eq!(stack.built_at.as_deref(), Some("src/lib.rs:8"));
    let stack_record = base
        .types
        .iter()
        .find(|t| t.id == "type:ItemStack")
        .unwrap();
    assert_eq!(stack.fields_hash, stack_record.fields_hash);
    assert_eq!(
        entry(&uses, "type:PlayerLoggedInEvent").built,
        Some(Built::None)
    );
    assert_eq!(entry(&uses, "type:PlayerLoggedInEvent").fields_hash, None);
    assert_eq!(entry(&uses, "trait:Hooks").built, None);
}

#[test]
fn the_first_place_is_the_first_file_then_line_whatever_the_order_given() {
    let sources = [
        ("src/b.rs", "\n\nfn f(_: Inventory) {}"),
        ("src/a.rs", "\nfn f(_: Inventory) {}"),
    ];
    let uses = uses(sources, &ManifestFacts::default(), &base().build()).unwrap();
    assert_eq!(
        uses.items[0].origin,
        Origin::Source {
            file: "src/a.rs".into(),
            line: 2
        }
    );
}

#[test]
fn it_fits_the_same_build() {
    assert_eq!(verdict(base()), Fit::Fits);
}

#[test]
fn it_fits_when_host_gains_a_method_a_hook_is_added_and_a_read_only_type_gains_a_field() {
    let mut target = base();
    target.traits[0] = host_trait(vec![method(
        "method:Host::time",
        "fn time(&self) -> u64",
        &["trait:Host"],
    )]);
    target
        .hooks
        .push(hook("player_left", "PlayerRef", &["type:PlayerRef"]));
    target.replace_type(api_type(
        "type:PlayerRef",
        "",
        false,
        vec![field("type:PlayerRef.uuid", "u128", &["type:PlayerRef"])],
    ));
    // The event the mod reads (and never builds) gains a field too.
    target.replace_type(api_type(
        "type:PlayerLoggedInEvent",
        "",
        true,
        vec![
            field(
                "type:PlayerLoggedInEvent.player",
                "PlayerRef",
                &["type:PlayerLoggedInEvent", "type:PlayerRef"],
            ),
            field(
                "type:PlayerLoggedInEvent.name",
                "String",
                &["type:PlayerLoggedInEvent"],
            ),
        ],
    ));
    assert_eq!(verdict(target), Fit::Fits);
}

#[test]
fn it_fits_when_an_item_it_does_not_use_changes_or_goes() {
    let mut target = base();
    target.hooks.retain(|h| h.id != "hook:block_broken");
    target.replace_type(api_type("type:Unrelated", "<T>", false, vec![]));
    assert_eq!(verdict(target), Fit::Fits);
}

#[test]
fn a_hook_fits_when_it_gains_an_argument_and_not_when_a_named_one_is_retyped() {
    let event = |extra: bool| {
        let mut list = vec![Argument {
            name: "player".into(),
            ty: "PlayerRef".into(),
        }];
        if extra {
            list.push(Argument {
                name: "name".into(),
                ty: "String".into(),
            });
        }
        list
    };
    let mut gained = base();
    gained.hooks[0].contract.arguments = event(true);
    gained.hooks[0] = Hook::new(
        "hook:player_logged_in",
        "",
        HookEngine::default(),
        Counterparts::default(),
        gained.hooks[0].contract.clone(),
    );
    assert_eq!(verdict(gained), Fit::Fits);
    let mut retyped = base();
    retyped.hooks[0] = hook(
        "player_logged_in",
        "ServerPlayerRef",
        &["type:PlayerLoggedInEvent"],
    );
    assert_eq!(
        verdict(retyped),
        Fit::DoesNotFit(vec![misfit(
            "hook:player_logged_in.player",
            Change::Changed
        )])
    );
    // A mod that subscribes but never names the argument fits the retyped hook.
    let quiet = scan("impl Hooks for M { fn player_logged_in(&self) {} }");
    let mut target = base();
    target.hooks[0] = hook(
        "player_logged_in",
        "ServerPlayerRef",
        &["type:PlayerLoggedInEvent"],
    );
    assert_eq!(fits(quiet.entries(), &target.build()), Fit::Fits);
}

#[test]
fn a_hook_version_counts_only_when_the_manifest_names_one() {
    let mut target = base();
    target.hooks[0].contract.version = 2;
    let version_two = target.build();
    let source = [("src/lib.rs", MOD)];
    let plain = uses(source, &manifest(&["registries"]), &base().build()).unwrap();
    assert_eq!(fits(plain.entries(), &version_two), Fit::Fits);
    let mut facts = manifest(&["registries"]);
    facts.hook_versions.insert("player_logged_in".into(), 1);
    let pinned = uses(source, &facts, &base().build()).unwrap();
    assert_eq!(entry(&pinned, "hook:player_logged_in").version, Some(1));
    assert_eq!(
        words(fits(pinned.entries(), &version_two)),
        ["the hook `player_logged_in` is now version 2; this mod names version 1"]
    );
}

#[test]
fn a_removed_used_method_is_named() {
    let mut target = base();
    target.traits[1].methods.clear();
    let Fit::DoesNotFit(misfits) = verdict(target) else {
        panic!("fits")
    };
    assert_eq!(
        misfits,
        [misfit("method:ServerHost::give_item", Change::Removed)]
    );
    assert_eq!(
        misfits[0].to_string(),
        "the method `ServerHost::give_item` (removed)"
    );
}

#[test]
fn a_struct_built_field_by_field_that_gains_a_field_is_named_with_the_line() {
    let mut target = base();
    target.replace_type(api_type(
        "type:ItemStack",
        "",
        true,
        vec![
            field("type:ItemStack.count", "u8", &["type:ItemStack"]),
            field("type:ItemStack.damage", "u16", &["type:ItemStack"]),
        ],
    ));
    assert_eq!(
        words(verdict(target)),
        [
            "the type `ItemStack` gained the field `damage`, and `src/lib.rs:8` builds it field by field"
        ]
    );
}

#[test]
fn a_struct_built_with_a_base_or_not_built_fits_a_new_field() {
    let source = "fn f() -> Settings { let s = Settings { height: 1, ..Default::default() }; s }";
    let uses = scan_on(wider(), source);
    let record = entry(&uses, "type:Settings");
    assert_eq!(record.built, Some(Built::Base));
    assert_eq!(record.built_at.as_deref(), Some("src/lib.rs:1"));
    assert_eq!(record.fields_hash, None);
    let mut target = wider();
    target.replace_type(settings(
        &[("height", "i32"), ("sky", "bool")],
        &["Clone", "Default"],
    ));
    assert_eq!(fits(uses.entries(), &target.build()), Fit::Fits);
    // A named field that is retyped still refuses.
    let mut retyped = wider();
    retyped.replace_type(settings(&[("height", "u32")], &["Clone", "Default"]));
    assert_eq!(
        words(fits(uses.entries(), &retyped.build())),
        ["the field `Settings.height` (changed)"]
    );
}

#[test]
fn how_a_struct_is_built_is_read_from_literals_and_patterns() {
    let built = |source: &str| entry(&scan_on(wider(), source), "type:Settings").built;
    // Literals and patterns with no `..` are full; a literal with a base is base; a pattern with
    // `..` is a read, not a build.
    assert_eq!(
        built("fn t() { let s = Settings { height: 1 }; }"),
        Some(Built::Full)
    );
    assert_eq!(
        built("fn t() { let Settings { height } = s; }"),
        Some(Built::Full)
    );
    assert_eq!(
        built("fn t() { let Settings { .. } = s; }"),
        Some(Built::None)
    );
    assert_eq!(
        built("fn t() { let s = Settings { ..d }; }"),
        Some(Built::Base)
    );
    assert_eq!(
        built("fn t() { let s = Settings { height: f(Settings { ..d }) }; }"),
        Some(Built::Full)
    );
    // A range in a field value is not a base.
    assert_eq!(
        built("fn t() { let s = Settings { height: 0..5 }; }"),
        Some(Built::Full)
    );
    // Where a type stands, a brace opens a body, not a literal.
    assert_eq!(built("fn f() -> Settings { make() }"), Some(Built::None));
    assert_eq!(
        built("impl Mine for mod_api::Settings { fn x() {} }"),
        Some(Built::None)
    );
    assert_eq!(
        built("fn t() { let s: Settings = make(); }"),
        Some(Built::None)
    );
    // One full literal anywhere makes it full; a file that does not parse cannot be judged.
    assert_eq!(
        built("fn t() { let a = Settings { ..d };\nlet b = mod_api::Settings { height: 2 }; }"),
        Some(Built::Full)
    );
    assert_eq!(
        uses(
            [("src/lib.rs", "fn t() {\nlet s = Settings { height: 1")],
            &manifest(&[]),
            &wider().build()
        ),
        Err(ScanError::Parse {
            file: "src/lib.rs".into(),
            line: 2
        })
    );
    // A tuple struct's constructor is a full literal.
    let pos = |source: &str| entry(&scan_on(wider(), source), "type:Pos").built;
    assert_eq!(pos("fn t() { let p = Pos(1, 2); }"), Some(Built::Full));
    // A `..` inside a nested bracket is no rest.
    assert_eq!(pos("fn t() { let p = Pos((..), 2); }"), Some(Built::Full));
    assert_eq!(pos("fn t() { let Pos(x, ..) = p; }"), Some(Built::None));
    assert_eq!(pos("fn f(p: Pos) {}"), Some(Built::None));
}

#[test]
fn a_type_that_gains_an_impl_fits_and_one_that_loses_one_is_named() {
    let uses = scan_on(wider(), "fn f(s: &Settings) {}");
    assert_eq!(entry(&uses, "type:Settings").impls, ["Clone", "Default"]);
    let mut gained = wider();
    gained.replace_type(settings(
        &[("height", "i32")],
        &["Clone", "Default", "Hash"],
    ));
    assert_eq!(fits(uses.entries(), &gained.build()), Fit::Fits);
    let mut lost = wider();
    lost.replace_type(settings(&[("height", "i32")], &["Default"]));
    assert_eq!(
        words(fits(uses.entries(), &lost.build())),
        ["the type `Settings` no longer implements `Clone`"]
    );
}

#[test]
fn a_name_after_a_path_to_its_owner_is_that_member_not_a_type_of_the_same_name() {
    let uses = scan_on(
        wider(),
        "fn t() { match hover { TextHover::Item { .. } => {}, _ => {} } }",
    );
    assert!(ids(&uses).contains(&"type:TextHover.Item"));
    assert!(!ids(&uses).contains(&"type:Item"));
    // Named on its own, it is the type (and the variant, which its owner does not reach).
    let alone = scan_on(wider(), "fn f(i: Item) {}");
    assert!(ids(&alone).contains(&"type:Item"));
    assert!(!ids(&alone).contains(&"type:TextHover.Item"));
}

#[test]
fn a_type_reached_only_through_a_used_signature_changing_is_named() {
    let mut target = base();
    target.replace_type(api_type("type:Inventory", "<'a>", false, vec![]));
    assert_eq!(
        verdict(target),
        Fit::DoesNotFit(vec![misfit("type:Inventory", Change::Changed)])
    );
}

#[test]
fn several_misfits_come_sorted_and_worded() {
    let mut target = base();
    target.hooks[0] = hook("player_logged_in", "u128", &["type:PlayerLoggedInEvent"]);
    target.types.retain(|t| t.id != "type:Inventory");
    target.replace_type(api_type(
        "type:PlayerLoggedInEvent",
        "",
        true,
        vec![field(
            "type:PlayerLoggedInEvent.player",
            "u128",
            &["type:PlayerLoggedInEvent"],
        )],
    ));
    assert_eq!(
        words(verdict(target)),
        [
            "the argument `player_logged_in.player` (changed)",
            "the type `Inventory` (removed)",
            "the field `PlayerLoggedInEvent.player` (changed)",
        ]
    );
}

#[test]
fn a_record_reached_through_others_names_the_counted_record_it_came_from() {
    // The hook references its event type, whose field references `PlayerRef`.
    let uses = scan("fn f() { on(player_logged_in) }");
    let player = uses
        .items
        .iter()
        .find(|u| u.entry.id == "type:PlayerRef")
        .unwrap();
    assert_eq!(player.origin, Origin::Via("hook:player_logged_in".into()));
}

#[test]
fn changes_are_worded_for_people() {
    let say = |id: &str, change: Change| {
        Misfit {
            id: id.into(),
            change,
        }
        .to_string()
    };
    assert_eq!(
        say("type:Kind.Stone", Change::Removed),
        "the variant `Kind::Stone` (removed)"
    );
    assert_eq!(
        say("type:Text.content", Change::Removed),
        "the field `Text.content` (removed)"
    );
    assert_eq!(
        say("type:Pair.0", Change::Removed),
        "the field `Pair.0` (removed)"
    );
    assert_eq!(
        say("handle:PlayerRef::tell", Change::Removed),
        "the method `PlayerRef::tell` (removed)"
    );
    assert_eq!(
        say("capability:horizon:fluids", Change::Removed),
        "the capability `horizon:fluids` (removed)"
    );
    assert_eq!(
        say(
            "type:Settings",
            Change::Fields {
                gained: vec!["sky".into(), "fog".into()],
                at: "src/world.rs:12".into()
            }
        ),
        "the type `Settings` gained the fields `sky`, `fog`, and `src/world.rs:12` builds it field by field"
    );
    assert_eq!(
        say(
            "type:Pos",
            Change::Fields {
                gained: vec![],
                at: "src/a.rs:3".into()
            }
        ),
        "the fields of the type `Pos` changed, and `src/a.rs:3` builds it field by field"
    );
}

#[test]
fn names_in_comments_and_strings_do_not_count() {
    let uses = scan(MOD);
    for id in ["hook:block_broken", "type:Unrelated", "trait:Particles"] {
        assert!(!ids(&uses).contains(&id), "{id}");
    }
}

#[test]
fn a_name_used_only_in_a_macro_invocation_counts() {
    let uses = scan("fn f() { log!(\"{}\", Unrelated::default()); }");
    assert_eq!(ids(&uses), ["capability:registries", "type:Unrelated"]);
}

#[test]
fn a_same_named_method_of_an_unrelated_trait_is_not_pulled_in() {
    // `tell` is a method of `PlayerRef` (named) and of `Particles` (named nowhere).
    let source =
        format!("{MOD}\nfn greet(p: PlayerRef, h: &mut dyn ServerHost) {{ p.tell(h, t); }}");
    let uses = scan(&source);
    assert!(!ids(&uses).contains(&"method:Particles::tell"));
    assert!(!ids(&uses).contains(&"trait:Particles"));
    assert!(ids(&uses).contains(&"handle:PlayerRef::tell"));
    assert!(ids(&uses).contains(&"type:Text"));
}

#[test]
fn a_member_counts_when_its_owner_is_reached_and_not_otherwise() {
    // `inventory` is a method of `PlayerRef`, which the source never names: it is reached
    // through the hook method's event type's field.
    let uses = scan(MOD);
    assert!(ids(&uses).contains(&"handle:PlayerRef::inventory"));
    assert!(ids(&uses).contains(&"type:Inventory"));
    // A member whose owner nothing reaches is dropped.
    let alone = scan("fn f(x: X) { x.inventory() }");
    assert_eq!(ids(&alone), ["capability:registries"]);
}

#[test]
fn the_list_does_not_depend_on_the_order_of_sources() {
    let a = ("src/a.rs", "use mod_api::ServerHost;");
    let b = ("src/b.rs", "fn f(e: PlayerLoggedInEvent) {}");
    let surface = base().build();
    let one = uses([a, b], &manifest(&["registries"]), &surface).unwrap();
    let two = uses([b, a], &manifest(&["registries"]), &surface).unwrap();
    assert_eq!(one, two);
    assert_eq!(one.table("abc").to_json(), two.table("abc").to_json());
}

#[test]
fn a_table_reads_back_and_judges_like_the_list() {
    let uses = scan(MOD);
    let table = UsesTable::read(uses.table("afe471c").to_json().as_bytes()).unwrap();
    assert_eq!(table.made_for, "afe471c");
    assert_eq!(table.entries, uses.entries().cloned().collect::<Vec<_>>());
    let mut target = base();
    target.traits[1].methods.clear();
    assert_eq!(
        fits(&table.entries, &target.build()),
        Fit::DoesNotFit(vec![misfit(
            "method:ServerHost::give_item",
            Change::Removed
        )])
    );
}

#[test]
fn a_table_refuses_unknown_fields_and_sorts_its_entries() {
    let text = r#"{"schema":1,"made_for":"x","entries":[{"id":"b","hash":"2"},{"id":"a","hash":"1","built":"base"}]}"#;
    let table = UsesTable::read(text.as_bytes()).unwrap();
    let ids: Vec<&str> = table.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
    assert_eq!(table.entries[0].built, Some(Built::Base));
    for unknown in [
        r#"{"schema":1,"made_for":"x","later":true,"entries":[]}"#,
        r#"{"schema":1,"made_for":"x","entries":[{"id":"a","hash":"1","route":"x"}]}"#,
    ] {
        assert!(matches!(
            UsesTable::read(unknown.as_bytes()),
            Err(UsesError::Json(_))
        ));
    }
}

#[test]
fn bad_tables_are_refused_by_kind() {
    let refused = |text: &[u8]| UsesTable::read(text).unwrap_err();
    assert_eq!(
        refused(br#"{"schema":2,"made_for":"x","entries":[]}"#),
        UsesError::Schema(2)
    );
    assert!(matches!(refused(b"not json"), UsesError::Json(_)));
    assert!(matches!(
        refused(b"{\"schema\":1,\"made_for\""),
        UsesError::Json(_)
    ));
    assert!(matches!(
        refused(br#"{"schema":1,"entries":[]}"#),
        UsesError::Json(_)
    ));
    assert!(matches!(
        refused(br#"{"schema":1,"made_for":"x","entries":[{"id":"a"}]}"#),
        UsesError::Json(_)
    ));
    assert!(matches!(
        refused(br#"{"schema":1,"made_for":"x","entries":[{"id":"a","hash":"1","built":"half"}]}"#),
        UsesError::Json(_)
    ));
    assert_eq!(
        refused(br#"{"schema":1,"made_for":"x","entries":[{"id":"a","hash":"1"},{"id":"a","hash":"2"}]}"#),
        UsesError::DuplicateId("a".into())
    );
    let mut big = br#"{"schema":1,"made_for":"x","entries":[]}"#.to_vec();
    big.resize(MAX_USES_BYTES + 1, b' ');
    assert_eq!(refused(&big), UsesError::TooLarge(MAX_USES_BYTES + 1));
    let mut edge = br#"{"schema":1,"made_for":"x","entries":[]}"#.to_vec();
    edge.resize(MAX_USES_BYTES, b' ');
    assert!(UsesTable::read(&edge).is_ok());
}

#[test]
fn check_says_how_each_struct_is_built() {
    let note = |source: &str, id: &str| {
        scan_on(wider(), source)
            .items
            .into_iter()
            .find(|u| u.entry.id == id)
            .unwrap()
            .note
    };
    assert_eq!(
        note(
            "fn t() { let s = Settings { height: 1, ..Default::default() }; }",
            "type:Settings"
        )
        .as_deref(),
        Some(
            "`Settings`: built with `..` (src/lib.rs:1), so a new field in a later game build will not break this mod."
        )
    );
    assert_eq!(
        note(
            "\nfn t() { let s = Settings { height: 1 }; }",
            "type:Settings"
        )
        .as_deref(),
        Some(
            "`Settings`: built field by field (src/lib.rs:2). A new field in a later game build will break this line; end the literal with `..Default::default()` to stay compatible."
        )
    );
    // A struct with no `Default` impl: no advice.
    assert_eq!(
        note(
            "fn t() { let s = ItemStack { count: 1 }; }",
            "type:ItemStack"
        )
        .as_deref(),
        Some(
            "`ItemStack`: built field by field (src/lib.rs:1). A new field in a later game build will break this line."
        )
    );
    // A pattern takes `..`, not a base: no advice.
    assert_eq!(
        note("fn t() { let Settings { height } = s; }", "type:Settings").as_deref(),
        Some(
            "`Settings`: built field by field (src/lib.rs:1). A new field in a later game build will break this line."
        )
    );
    // A tuple struct cannot take a base either.
    assert_eq!(
        note("fn t() { let p = Pos(1, 2); }", "type:Pos").as_deref(),
        Some(
            "`Pos`: built field by field (src/lib.rs:1). A new field in a later game build will break this line."
        )
    );
    assert_eq!(note("fn f(s: Settings) {}", "type:Settings"), None);
}

/// The base surface with `Host` made of a sub-host trait, `PlayersHost`, as the API's hosts are.
fn hosts(players: Vec<Method>, more: &[&str]) -> Pieces {
    let mut pieces = base();
    let mut bounds = vec!["trait:PlayersHost"];
    bounds.extend(more);
    pieces.traits[0] = api_trait("trait:Host", Role::Call, &bounds, vec![]);
    pieces
        .traits
        .push(api_trait("trait:PlayersHost", Role::Call, &[], players));
    for name in more {
        pieces.traits.push(api_trait(name, Role::Call, &[], vec![]));
    }
    pieces
}

fn player_method(returns: &str) -> Method {
    method(
        "method:PlayersHost::player",
        &format!("fn player(&self, _: u128) -> {returns}"),
        &["trait:PlayersHost", "type:PlayerRef"],
    )
}

#[test]
fn a_sub_host_method_called_on_the_host_is_used_and_judged() {
    let source = "fn f(host: &mut dyn ServerHost, id: u128) { host.player(id); }";
    let made_for = hosts(vec![player_method("Option<PlayerRef>")], &[]);
    let uses = scan_on(made_for, source);
    assert!(ids(&uses).contains(&"method:PlayersHost::player"));
    assert!(ids(&uses).contains(&"trait:PlayersHost"));
    let removed = hosts(vec![], &[]);
    assert_eq!(
        words(fits(uses.entries(), &removed.build())),
        ["the method `PlayersHost::player` (removed)"]
    );
    let retyped = hosts(vec![player_method("PlayerRef")], &[]);
    assert_eq!(
        words(fits(uses.entries(), &retyped.build())),
        ["the method `PlayersHost::player` (changed)"]
    );
    // A new sub-host trait refuses nobody.
    let wider = hosts(
        vec![player_method("Option<PlayerRef>")],
        &["trait:FluidsHost"],
    );
    assert_eq!(fits(uses.entries(), &wider.build()), Fit::Fits);
    // The sub-host traits the mod never reaches a method of stay out of its list.
    let with_fluids = scan_on(
        hosts(
            vec![player_method("Option<PlayerRef>")],
            &["trait:FluidsHost"],
        ),
        source,
    );
    assert!(!ids(&with_fluids).contains(&"trait:FluidsHost"));
}

#[test]
fn a_tuple_field_read_by_position_is_used_and_judged() {
    let uses = scan_on(wider(), "fn f(p: Pos) -> bool { p.0 > 70_000 }");
    assert!(ids(&uses).contains(&"type:Pos.0"));
    let mut target = wider();
    target.replace_type(api_tuple("type:Pos", &["i64", "i32"]));
    assert_eq!(
        words(fits(uses.entries(), &target.build())),
        ["the field `Pos.0` (changed)"]
    );
}

#[test]
fn full_builds_by_another_name_are_full() {
    let built = |source: &str, id: &str| entry(&scan_on(wider(), source), id).built;
    let full = Some(Built::Full);
    // `Self` inside an impl for the API struct.
    assert_eq!(
        built(
            "trait Make { fn make() -> Self; }\nimpl Make for mod_api::Settings { fn make() -> Self { Self { height: 1 } } }",
            "type:Settings"
        ),
        full
    );
    // An alias and a renamed import.
    assert_eq!(
        built(
            "type S2 = Settings;\nfn f() { let s = S2 { height: 1 }; }",
            "type:Settings"
        ),
        full
    );
    assert_eq!(
        built(
            "use mod_api::{Settings as S};\nfn f() { let s = S { height: 1 }; }",
            "type:Settings"
        ),
        full
    );
    // A turbofish.
    assert_eq!(
        built(
            "fn t() { let s = Settings::<u8> { height: 1 }; }",
            "type:Settings"
        ),
        full
    );
    // A macro's `$t { … }` and a literal of a name the scan cannot place.
    assert_eq!(
        built(
            "macro_rules! make { ($t:ty) => { $t { height: 1 } } }\nfn f(s: Settings) {}",
            "type:Settings"
        ),
        full
    );
    assert_eq!(
        built(
            "fn f(s: Settings) { let w = Wrapper { a: 1 }; }",
            "type:Settings"
        ),
        full
    );
    // The mod's own struct, or its own enum's variant, is placed.
    assert_eq!(
        built(
            "enum Kind { Custom { a: u8 } }\nuse Kind::Custom;\nfn f(s: Settings) { let k = Custom { a: 1 }; }",
            "type:Settings"
        ),
        Some(Built::None)
    );
    assert_eq!(
        built(
            "struct Wrapper { a: u8 }\nfn f(s: Settings) { let w = Wrapper { a: 1 }; }",
            "type:Settings"
        ),
        Some(Built::None)
    );
    // The mod's own struct of an API struct's name is the mod's.
    assert_eq!(
        built(
            "struct Settings { mine: u8 }\nimpl Settings { fn new() -> Self { Self { mine: 1 } } }",
            "type:Settings"
        ),
        Some(Built::None)
    );
    assert_eq!(
        built(
            "struct Settings { mine: u8 }\nfn f() { let s = mod_api::Settings { height: 1 }; }",
            "type:Settings"
        ),
        full
    );
    // A tuple struct's constructor passed as a function.
    assert_eq!(built("fn t() { let ps = xs.map(Pos); }", "type:Pos"), full);
    assert_eq!(
        built("fn t() { let ps = xs.map_or(none, Pos); }", "type:Pos"),
        full
    );
    // In a tuple type or a parameter list it is a type.
    assert_eq!(
        built("fn t() { let v: Vec<(Pos, i32)> = vec![]; }", "type:Pos"),
        Some(Built::None)
    );
    assert_eq!(
        built("fn f(a: u8, p: Pos) {}", "type:Pos"),
        Some(Built::None)
    );
    assert_eq!(
        built("use mod_api::{Pos, Settings};", "type:Pos"),
        Some(Built::None)
    );
}

#[test]
fn a_union_literal_is_not_a_full_build() {
    let mut pieces = wider();
    let mut union = api_type(
        "type:Bits",
        "",
        true,
        vec![field("type:Bits.word", "u32", &["type:Bits"])],
    );
    union = Type::new(
        union.id.clone(),
        "",
        None,
        TypeContract {
            kind: TypeKind::Union,
            ..union.contract.clone()
        },
        union.fields.clone(),
        vec![],
    );
    pieces.types.push(union);
    let uses = scan_on(pieces, "fn t() { let b = Bits { word: 1 }; }");
    assert_eq!(entry(&uses, "type:Bits").built, Some(Built::Base));
}

#[test]
fn a_struct_the_mod_builds_that_can_no_longer_be_built_is_named() {
    let unbuildable = || {
        let mut target = wider();
        let mut record = settings(&[("height", "i32")], &["Clone", "Default"]);
        record = Type::new(
            record.id.clone(),
            "",
            None,
            TypeContract {
                constructible: false,
                ..record.contract.clone()
            },
            record.fields.clone(),
            vec![],
        );
        record.impls = vec!["Clone".into(), "Default".into()];
        target.replace_type(record);
        target.build()
    };
    let builds = scan_on(
        wider(),
        "\nfn t() { let s = Settings { height: 1, ..Default::default() }; }",
    );
    assert_eq!(
        words(fits(builds.entries(), &unbuildable())),
        ["the type `Settings` can no longer be built with a literal, and `src/lib.rs:2` builds it"]
    );
    let reads = scan_on(wider(), "fn f(s: &Settings) -> i32 { s.height }");
    assert_eq!(fits(reads.entries(), &unbuildable()), Fit::Fits);
}

#[test]
fn a_path_the_mod_writes_must_still_be_exported() {
    let biome = |paths: &[&str]| {
        let mut record = api_type("type:world::Biome", "", false, vec![]);
        record.paths = paths.iter().map(|p| p.to_string()).collect();
        record
    };
    let mut made_for = wider();
    made_for.types.push(biome(&["Biome", "world::Biome"]));
    let uses = scan_on(made_for, "use mod_api::Biome;\nfn f(b: Biome) {}");
    assert_eq!(entry(&uses, "type:world::Biome").paths, ["Biome"]);
    let mut kept = wider();
    kept.types
        .push(biome(&["Biome", "world::Biome", "prelude::Biome"]));
    assert_eq!(fits(uses.entries(), &kept.build()), Fit::Fits);
    let mut dropped = wider();
    dropped.types.push(biome(&["world::Biome"]));
    assert_eq!(
        words(fits(uses.entries(), &dropped.build())),
        ["`mod_api::Biome` is no longer exported under that path"]
    );
    // The id's own path is never listed.
    let mut own = wider();
    own.types.push(biome(&["Biome", "world::Biome"]));
    let direct = scan_on(own, "fn f(b: mod_api::world::Biome) {}");
    assert!(entry(&direct, "type:world::Biome").paths.is_empty());
}

#[test]
fn a_sub_host_trait_leaving_the_host_the_mod_calls_through_is_named() {
    let source = "fn f(host: &mut dyn ServerHost, id: u128) { host.player(id); }";
    let uses = scan_on(hosts(vec![player_method("Option<PlayerRef>")], &[]), source);
    assert_eq!(
        entry(&uses, "method:PlayersHost::player").reached_from,
        ["trait:ServerHost"]
    );
    // `PlayersHost` moves out of `Host`, unchanged.
    let mut moved = hosts(vec![player_method("Option<PlayerRef>")], &[]);
    moved.traits[0] = api_trait("trait:Host", Role::Call, &[], vec![]);
    assert_eq!(
        words(fits(uses.entries(), &moved.build())),
        ["`ServerHost` no longer includes `PlayersHost`, which this mod calls `player` on"]
    );
    // A new sub-host trait beside it still fits.
    let wider = hosts(
        vec![player_method("Option<PlayerRef>")],
        &["trait:FluidsHost"],
    );
    assert_eq!(fits(uses.entries(), &wider.build()), Fit::Fits);
    // A method on the trait the mod names needs no route.
    let direct = scan_on(
        hosts(vec![player_method("Option<PlayerRef>")], &[]),
        "fn f(host: &mut dyn ServerHost) { host.give_item(p, s); }",
    );
    assert!(
        entry(&direct, "method:ServerHost::give_item")
            .reached_from
            .is_empty()
    );
}

pub(crate) fn scan_files(pieces: Pieces, files: &[(&str, &str)]) -> Uses {
    uses(files.iter().copied(), &manifest(&[]), &pieces.build()).unwrap()
}

fn built_in(files: &[(&str, &str)], id: &str) -> Option<Built> {
    entry(&scan_files(wider(), files), id).built
}

#[test]
fn a_macro_literal_in_another_file_builds_every_struct_the_mod_names() {
    let full = Some(Built::Full);
    let macros = "macro_rules! make { ($t:ident, $h:expr) => { $t { height: $h } } }";
    let world = "fn f() -> Settings { make!(Settings, 1) }";
    assert_eq!(
        built_in(
            &[("src/macros.rs", macros), ("src/world.rs", world)],
            "type:Settings"
        ),
        full
    );
    // The repetition form.
    let repeated =
        "macro_rules! make { ($t:ident { $($f:ident: $v:expr),* }) => { $t { $($f: $v),* } } }";
    assert_eq!(
        built_in(
            &[("src/macros.rs", repeated), ("src/world.rs", world)],
            "type:Settings"
        ),
        full
    );
    // A metavariable where a type stands is not a literal.
    let typed = "macro_rules! make { ($t:ty, $h:expr) => { impl Foo for $t { } } }";
    assert_eq!(
        built_in(
            &[("src/macros.rs", typed), ("src/world.rs", world)],
            "type:Settings"
        ),
        Some(Built::None)
    );
}

#[test]
fn aliases_and_renames_in_another_file_resolve() {
    let full = Some(Built::Full);
    let world = "use crate::Dims;\nfn f() { let d = Dims { height: 1 }; }";
    assert_eq!(
        built_in(
            &[
                ("src/lib.rs", "pub type Dims = mod_api::Settings;"),
                ("src/world.rs", world)
            ],
            "type:Settings"
        ),
        full
    );
    assert_eq!(
        built_in(
            &[
                ("src/lib.rs", "pub use mod_api::Settings as Dims;"),
                ("src/world.rs", world)
            ],
            "type:Settings"
        ),
        full
    );
}

#[test]
fn the_mods_own_type_wins_only_when_no_file_can_see_the_apis() {
    let none = Some(Built::None);
    let full = Some(Built::Full);
    let own = (
        "src/own.rs",
        "pub struct Settings { pub mine: u8 }\nfn g() { let s = Settings { mine: 1 }; }",
    );
    assert_eq!(built_in(&[own], "type:Settings"), none);
    // A glob import of the API anywhere: the bare literal may be the API's.
    let glob = (
        "src/world.rs",
        "use mod_api::*;\nfn f() { let s = Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[own, glob], "type:Settings"), full);
    // The crate renamed: `api::Settings { … }` is the API's.
    let renamed = (
        "src/world.rs",
        "use mod_api as api;\nfn f() { let s = api::Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[own, renamed], "type:Settings"), full);
    // A variant or an associated type of the name does not shadow.
    let variant = (
        "src/kinds.rs",
        "enum Kind { Settings }\nfn f() { let s = Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[variant], "type:Settings"), full);
    let associated = (
        "src/kinds.rs",
        "impl It for X { type Settings = u8; }\nfn f() { let s = Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[associated], "type:Settings"), full);
}

#[test]
fn a_glob_import_records_the_paths_of_the_names_the_mod_uses() {
    let biome = |paths: &[&str]| {
        let mut record = api_type("type:world::Biome", "", false, vec![]);
        record.paths = paths.iter().map(|p| p.to_string()).collect();
        record
    };
    let mut made_for = wider();
    made_for.types.push(biome(&["Biome", "world::Biome"]));
    let uses = scan_files(
        made_for,
        &[("src/lib.rs", "use mod_api::*;\nfn f(b: Biome) {}")],
    );
    assert_eq!(entry(&uses, "type:world::Biome").paths, ["Biome"]);
    let mut dropped = wider();
    dropped.types.push(biome(&["world::Biome"]));
    assert_eq!(
        words(fits(uses.entries(), &dropped.build())),
        ["`mod_api::Biome` is no longer exported under that path"]
    );
}

#[test]
fn a_read_pattern_fits_a_struct_that_can_no_longer_be_built() {
    let mut target = wider();
    let mut record = settings(&[("height", "i32")], &["Clone", "Default"]);
    record = Type::new(
        record.id.clone(),
        "",
        None,
        TypeContract {
            constructible: false,
            ..record.contract.clone()
        },
        record.fields.clone(),
        vec![],
    );
    record.impls = vec!["Clone".into(), "Default".into()];
    target.replace_type(record);
    let reads = scan_on(
        wider(),
        "fn f(s: Settings) { let Settings { height, .. } = s; }",
    );
    assert_eq!(fits(reads.entries(), &target.build()), Fit::Fits);
}

#[test]
fn a_tuple_constructor_as_a_value_is_a_full_build() {
    let pos = |source: &str| entry(&scan_on(wider(), source), "type:Pos").built;
    let full = Some(Built::Full);
    // `Maker` is the mod's own, so the literal is placed and only its field value builds.
    assert_eq!(
        pos("struct Maker { ctor: fn(i32, i32) -> Pos }\nfn f() { let m = Maker { ctor: Pos }; }"),
        full
    );
    assert_eq!(pos("fn t() { let c = match k { _ => Pos }; }"), full);
    assert_eq!(pos("fn t() { let cs = [Pos, Pos]; }"), full);
    assert_eq!(pos("fn c() -> fn(i32, i32) -> Pos { return Pos; }"), full);
    // Not as a field's type, an array type or an attribute.
    assert_eq!(pos("struct Maker { ctor: Pos }"), Some(Built::None));
    assert_eq!(pos("fn f(a: [Pos; 2]) {}"), Some(Built::None));
}

#[test]
fn whether_a_name_is_the_mods_own_follows_each_files_scope() {
    let none = Some(Built::None);
    let full = Some(Built::Full);
    // The examples case: the mod's own `Settings`, built with `Self { … }` in its file; a glob of
    // the API in another file does not reach it.
    let own = (
        "src/widget.rs",
        "pub struct Settings { pub mine: u8 }\nimpl Settings { fn new() -> Self { Self { mine: 1 } } }",
    );
    let glob_elsewhere = ("src/reload.rs", "use mod_api::*;\nfn f(s: &Settings) {}");
    assert_eq!(built_in(&[own, glob_elsewhere], "type:Settings"), none);
    // A third file with `use super::*` that builds `Settings { … }`: unsure, so the API's.
    let super_glob = (
        "src/third.rs",
        "use super::*;\nfn g() { let s = Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[own, super_glob], "type:Settings"), full);
    // An explicit import of the mod's own: a `use` binds the name, so the API's.
    let imported = (
        "src/third.rs",
        "use crate::widget::Settings;\nfn g() { let s = Settings { mine: 1 }; }",
    );
    let declared = ("src/lib.rs", "mod widget;\nmod third;");
    assert_eq!(built_in(&[declared, own, imported], "type:Settings"), full);
    // An explicit import of the API's, while the mod defines its own elsewhere: the API's.
    let api_import = (
        "src/third.rs",
        "use mod_api::Settings;\nfn g() { let s = Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[own, api_import], "type:Settings"), full);
    // A path inside the mod written out: still the API's (a literal with a path always is).
    let qualified = (
        "src/third.rs",
        "fn g() { let s = crate::widget::Settings { mine: 1 }; }",
    );
    assert_eq!(built_in(&[declared, own, qualified], "type:Settings"), full);
    // A path inside the mod that leads to an associated type, which may stand for the API's: the
    // API's. So does one through an enum, which is no module the mod declares.
    let associated = (
        "src/third.rs",
        "impl It for X { type Settings = mod_api::Settings; }\nfn g() { let s = crate::X::Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[associated], "type:Settings"), full);
    let variant = (
        "src/third.rs",
        "enum Kind { Settings { mine: u8 } }\nuse crate::Kind::Settings;\nfn g() { let s = Settings { mine: 1 }; }",
    );
    assert_eq!(built_in(&[variant], "type:Settings"), full);
    // A file with an API glob and no definition of its own: the API's.
    let api_glob = (
        "src/third.rs",
        "use mod_api::*;\nfn g() { let s = Settings { height: 1 }; }",
    );
    assert_eq!(built_in(&[own, api_glob], "type:Settings"), full);
    // A file with an inline module is more than one scope: unsure, so the API's.
    let inline = (
        "src/widget.rs",
        "mod inner { pub use mod_api::*; }\npub struct Settings { pub mine: u8 }\nfn g() { let s = Settings { mine: 1 }; }",
    );
    assert_eq!(built_in(&[inline], "type:Settings"), full);
}

#[test]
fn a_path_through_a_renamed_crate_is_recorded() {
    let mut made_for = wider();
    let mut record = api_type("type:world::Biome", "", false, vec![]);
    record.paths = vec!["Biome".into(), "world::Biome".into()];
    made_for.types.push(record.clone());
    let uses = scan_files(
        made_for.clone(),
        &[("src/lib.rs", "use mod_api as api;\nfn f(b: api::Biome) {}")],
    );
    assert_eq!(entry(&uses, "type:world::Biome").paths, ["Biome"]);
    let uses = scan_files(
        made_for,
        &[(
            "src/lib.rs",
            "extern crate mod_api as api;\nfn f(b: api::Biome) {}",
        )],
    );
    assert_eq!(entry(&uses, "type:world::Biome").paths, ["Biome"]);
}

/// `hosts` with a `ClientHost: Host` beside `ServerHost`, and `PlayersHost` a supertrait of `Host`
/// or, with `client_only`, of `ClientHost` alone.
fn two_sides(client_only: bool) -> Pieces {
    let mut pieces = hosts(vec![player_method("Option<PlayerRef>")], &[]);
    let client_bounds: &[&str] = if client_only {
        pieces.traits[0] = api_trait("trait:Host", Role::Call, &[], vec![]);
        &["trait:Host", "trait:PlayersHost"]
    } else {
        &["trait:Host"]
    };
    pieces.traits.push(api_trait(
        "trait:ClientHost",
        Role::Call,
        client_bounds,
        vec![],
    ));
    pieces
}

#[test]
fn every_counted_host_that_leads_to_a_called_methods_trait_must_still_lead_there() {
    // A client and a server hook; the call goes through `ServerHost`, which the scan cannot tell.
    let hooks = "fn tick(host: &mut dyn ClientHost) {}\nfn f(host: &mut dyn ServerHost, id: u128) { host.player(id); }";
    let uses = scan_on(two_sides(false), hooks);
    assert_eq!(
        entry(&uses, "method:PlayersHost::player").reached_from,
        ["trait:ClientHost", "trait:ServerHost"]
    );
    let moved = ["`ServerHost` no longer includes `PlayersHost`, which this mod calls `player` on"];
    assert_eq!(words(fits(uses.entries(), &two_sides(true).build())), moved);
    // The mod also names the sub-host trait and upcasts the host to it.
    let upcast = format!(
        "{hooks}\nfn show(p: &dyn PlayersHost) {{}}\nfn g(host: &mut dyn ServerHost) {{ show(host); }}"
    );
    let uses = scan_on(two_sides(false), &upcast);
    assert_eq!(words(fits(uses.entries(), &two_sides(true).build())), moved);
    assert_eq!(fits(uses.entries(), &two_sides(false).build()), Fit::Fits);
}

#[test]
fn a_table_naming_many_routes_the_target_lacks_is_judged_without_a_walk_each() {
    // A package's `reached_from` strings are its own: each not in the target is a route misfit,
    // found without walking the target's references again.
    let uses = scan_on(
        two_sides(false),
        "fn f(host: &mut dyn ServerHost, id: u128) { host.player(id); }",
    );
    let mut entries: Vec<Entry> = uses.entries().cloned().collect();
    let player = entries
        .iter_mut()
        .find(|e| e.id == "method:PlayersHost::player")
        .unwrap();
    player.reached_from = (0..50_000).map(|i| format!("a{i}")).collect();
    match fits(&entries, &two_sides(false).build()) {
        Fit::DoesNotFit(misfits) => {
            assert_eq!(misfits.len(), 50_000);
            assert!(
                misfits
                    .iter()
                    .all(|m| matches!(m.change, Change::Route { .. }))
            );
        }
        Fit::Fits => panic!("fits, though no named route exists"),
    }
}

pub(crate) fn dims(extra: &[&str]) -> Type {
    let mut names = vec!["min_y", "height", "logical_height"];
    names.extend(extra);
    api_type(
        "type:DimensionSettings",
        "",
        true,
        names
            .iter()
            .map(|name| {
                field(
                    &format!("type:DimensionSettings.{name}"),
                    "i32",
                    &["type:DimensionSettings"],
                )
            })
            .collect(),
    )
}

/// Whether the mod made of `files` still fits once the API's `DimensionSettings` gains a field.
fn fits_a_new_dims_field(files: &[(&str, &str)]) -> Fit {
    let mut made_for = base();
    made_for.types.push(dims(&[]));
    let uses = scan_files(made_for, files);
    let mut target = base();
    target.types.push(dims(&["ultrawarm"]));
    fits(uses.entries(), &target.build())
}

pub(crate) const DIMS_LITERAL: &str =
    "DimensionSettings { min_y: 0, height: 256, logical_height: 256 }";

fn refused_for_the_new_field(fit: Fit) {
    match fit {
        Fit::DoesNotFit(misfits) => assert!(
            misfits.iter().any(|m| m.id == "type:DimensionSettings"
                && matches!(&m.change, Change::Fields { gained, .. } if gained == &["ultrawarm"])),
            "{misfits:?}"
        ),
        Fit::Fits => panic!("fits, though the mod builds the API's struct field by field"),
    }
}

#[test]
fn a_host_reached_through_an_api_signature_records_its_route() {
    // `register` only: the closure's host is typed by `item_capability`'s signature.
    let registrar = |client_only: bool| {
        let mut pieces = two_sides(client_only);
        pieces.traits.push(api_trait(
            "trait:Registrar",
            Role::Call,
            &[],
            vec![method(
                "method:Registrar::item_capability",
                "fn item_capability(&mut self, _: &str, _: Box<dyn Fn(&mut dyn ServerHost) -> Option<u8>>)",
                &["trait:Registrar", "trait:ServerHost"],
            )],
        ));
        pieces
    };
    let source = "fn register(&self, r: &mut Registrar) {\n    r.item_capability(\"mymod:backpack\", Box::new(|host| { let _ = host.player(id); None }));\n}";
    let uses = scan_on(registrar(false), source);
    assert_eq!(
        entry(&uses, "method:PlayersHost::player").reached_from,
        ["trait:ServerHost"]
    );
    assert_eq!(
        words(fits(uses.entries(), &registrar(true).build())),
        ["`ServerHost` no longer includes `PlayersHost`, which this mod calls `player` on"]
    );
}

#[test]
fn a_use_statement_nested_too_deep_cannot_be_judged() {
    let nested = |depth: usize| {
        format!(
            "use mod_api::{}Settings{};\nfn f(s: Settings) {{}}",
            "a::{".repeat(depth),
            "}".repeat(depth)
        )
    };
    let surface = wider().build();
    let at_limit = uses(
        [("src/lib.rs", nested(MAX_USE_NESTING).as_str())],
        &manifest(&[]),
        &surface,
    );
    assert!(at_limit.is_ok());
    let deep = uses(
        [("src/lib.rs", nested(100_000).as_str())],
        &manifest(&[]),
        &surface,
    );
    assert_eq!(
        deep.unwrap_err().to_string(),
        "src/lib.rs:1: a `use` statement nests its groups deeper than 64; the mod's source cannot be judged"
    );
}

fn scan_result(source: &str) -> Result<Uses, ScanError> {
    uses([("src/lib.rs", source)], &manifest(&[]), &wider().build())
}

#[test]
fn source_nested_past_the_limit_cannot_be_judged_and_nothing_overflows() {
    // Each shape the parser reads by recursion, nested `n` deep after a literal on line 1.
    let shapes: [(&str, fn(usize) -> String); 20] = [
        ("parens", |n| {
            format!("let x = {}1{};", "(".repeat(n), ")".repeat(n))
        }),
        ("brackets", |n| {
            format!("let x = {}1{};", "[".repeat(n), "]".repeat(n))
        }),
        ("blocks", |n| {
            format!("let x = {}1{};", "{".repeat(n), "}".repeat(n))
        }),
        ("not", |n| format!("let x = {}y;", "!".repeat(n))),
        ("minus", |n| format!("let x = {}1;", "- ".repeat(n))),
        ("refs", |n| format!("let x = {}y;", "&".repeat(n))),
        ("closures", |n| format!("let x = {}1;", "|y| ".repeat(n))),
        ("empty closures", |n| {
            format!("let x = {}1;", "|| ".repeat(n))
        }),
        ("assignments", |n| format!("{}1;", "a = ".repeat(n))),
        ("returns", |n| format!("{}1;", "return ".repeat(n))),
        ("else if", |n| {
            format!("let x = {}{{ 1 }};", "if a { 1 } else ".repeat(n))
        }),
        ("generics", |n| {
            format!("let x: {}u8{} = y;", "V<".repeat(n), ">".repeat(n))
        }),
        ("fn types", |n| {
            format!("let x: {}u8 = y;", "fn() -> ".repeat(n))
        }),
        ("bindings", |n| format!("let {}_ = 1;", "a @ ".repeat(n))),
        ("mut refs", |n| format!("let {}y = z;", "&mut ".repeat(n))),
        ("mixed", |n| {
            format!("let x = {}y{};", "(!-&".repeat(n), ")".repeat(n))
        }),
        ("two-parameter closures", |n| {
            format!("let x = {}0;", "|a, b| ".repeat(n))
        }),
        ("generics with commas", |n| {
            format!("let x: {}u8{} = y;", "V<u8, ".repeat(n), ">".repeat(n))
        }),
        ("qualified paths after a literal", |n| {
            format!("let x = 1 < {}T{};", "<".repeat(n), " as A>::B".repeat(n))
        }),
        ("prefixes on method calls", |n| {
            format!("let x = {}y{};", "-x.f(".repeat(n), ")".repeat(n))
        }),
    ];
    for (shape, nest) in shapes {
        let source = |n: usize| {
            format!(
                "fn f() {{ let s = Settings {{ height: 1 }};\n{} }}",
                nest(n)
            )
        };
        let too_deep = |n: usize| match scan_result(&source(n)) {
            Ok(uses) => {
                assert_eq!(
                    entry(&uses, "type:Settings").built,
                    Some(Built::Full),
                    "{shape}"
                );
                false
            }
            Err(ScanError::TooDeep { file, line }) => {
                assert_eq!((file.as_str(), line), ("src/lib.rs", 2), "{shape}");
                true
            }
            Err(other) => panic!("{shape}: {other}"),
        };
        // Real code nests well within the limit.
        assert!(!too_deep(16), "{shape}");
        // The deepest nesting accepted parses, on the scan's own stack.
        let mut refused = 4 * crate::syntax::MAX_NESTING;
        assert!(too_deep(refused), "{shape}");
        let mut judged = crate::syntax::MAX_NESTING / 4;
        while refused - judged > 1 {
            let middle = (judged + refused) / 2;
            if too_deep(middle) {
                refused = middle;
            } else {
                judged = middle;
            }
        }
        assert!(judged <= crate::syntax::MAX_NESTING, "{shape}: {judged}");
    }
    // Comments nest without any limit: the tokenizer and the parser skip them in a loop.
    let depth = 100_000;
    let comments = format!(
        "{}{}\nfn f(s: Settings) {{}}",
        "/*".repeat(depth),
        "*/".repeat(depth)
    );
    assert!(ids(&scan_on(wider(), &comments)).contains(&"type:Settings"));
}

#[test]
fn a_struct_name_in_a_value_position_is_a_build() {
    let pos = |source: &str| entry(&scan_on(wider(), source), "type:Pos").built;
    let full = Some(Built::Full);
    // A named struct followed by `(` counts too.
    assert_eq!(
        entry(
            &scan_on(wider(), "fn f() { let s = Settings(1); }"),
            "type:Settings"
        )
        .built,
        full
    );
    // Type-only places are not.
    for source in [
        "fn f(p: Pos) {}",
        "fn f() -> Pos { make() }",
        "fn f(p: &'a mut Pos, q: [Pos; 2], r: (Pos, u8)) {}",
        "struct S { a: Pos, b: Vec<Pos>, c: HashMap<u8, Pos> }",
        "struct T(Pos, u8);",
        "enum E { A(Pos), B { p: Pos }, Pos }",
        "type P = Pos;",
        "impl From<Pos> for Q {}",
        "fn f() -> Box<dyn Fn(Pos) -> u8> { todo() }",
        "fn t() { let v: Vec<(Pos, i32)> = Vec::<Pos>::new(); }",
    ] {
        assert_eq!(pos(source), Some(Built::None), "{source}");
    }
}

#[test]
fn aliases_resolve_to_every_api_item_they_may_name() {
    let settings = |files: &[(&str, &str)]| built_in(files, "type:Settings");
    let full = Some(Built::Full);
    // One local name for different API items in different files counts for each.
    assert_eq!(
        settings(&[
            (
                "src/a.rs",
                "use mod_api::Settings as S;\nfn f() { let s = S { height: 1 }; }"
            ),
            ("src/b.rs", "use mod_api::ItemStack as S;"),
        ]),
        full
    );
    // A local name that is also an API name counts for both.
    assert_eq!(
        settings(&[(
            "src/a.rs",
            "use mod_api::Settings as ItemStack;\nfn f() { let s = ItemStack { height: 1 }; }"
        )]),
        full
    );
    // A name some file aliases to an API struct is that struct in every file, even where a file
    // defines its own of that name.
    assert_eq!(
        settings(&[
            ("src/a.rs", "type S = mod_api::Settings;"),
            ("src/b.rs", "struct S { a: u8 }\nfn f() -> S { S { a: 1 } }"),
        ]),
        full
    );
    // An alias of the mod's own struct is placed, so no literal of it is unsure.
    assert_eq!(
        settings(&[(
            "src/a.rs",
            "struct Mine { a: u8 }\ntype Wrap = Mine;\nfn f(s: Settings) { let w = Wrap { a: 1 }; }"
        )]),
        Some(Built::None)
    );
    // An associated type of the name is no alias of the file's own struct.
    assert_eq!(
        settings(&[(
            "src/a.rs",
            "struct Settings { mine: u8 }\nimpl It for X { type Settings = mod_api::Settings; }\nfn f() -> Settings { Settings { mine: 1 } }"
        )]),
        Some(Built::None)
    );
    // Aliases are followed transitively.
    assert_eq!(
        settings(&[(
            "src/a.rs",
            "type A = mod_api::Settings;\ntype B = A;\nfn f() { let s = B { height: 1 }; }"
        )]),
        full
    );
}

#[test]
fn a_path_through_an_imported_api_module_records_the_public_path() {
    let biome = |paths: &[&str]| {
        let mut record = api_type("type:world::Biome", "", false, vec![]);
        record.paths = paths.iter().map(|p| p.to_string()).collect();
        record
    };
    for source in [
        "use mod_api::prelude as p;\nfn f(_: p::Biome) {}",
        "use mod_api::prelude;\nfn f(_: prelude::Biome) {}",
        "use mod_api::prelude as p;\nuse p::Biome;\nfn f(_: Biome) {}",
        "use mod_api::prelude::{self};\nfn f(_: prelude::Biome) {}",
        "use mod_api::{self as api};\nfn f(_: api::prelude::Biome) {}",
    ] {
        let mut made_for = wider();
        made_for
            .types
            .push(biome(&["prelude::Biome", "world::Biome"]));
        let uses = scan_files(made_for, &[("src/lib.rs", source)]);
        assert_eq!(
            entry(&uses, "type:world::Biome").paths,
            ["prelude::Biome"],
            "{source}"
        );
        let mut dropped = wider();
        dropped.types.push(biome(&["world::Biome"]));
        assert_eq!(
            words(fits(uses.entries(), &dropped.build())),
            ["`mod_api::prelude::Biome` is no longer exported under that path"]
        );
    }
    // A binding through another binding: `w` is `world`, so `p` is `world::prelude`.
    let mut made_for = wider();
    made_for
        .types
        .push(biome(&["world::prelude::Biome", "world::Biome"]));
    let uses = scan_files(
        made_for,
        &[(
            "src/lib.rs",
            "use mod_api::world as w;\nuse w::prelude as p;\nfn f(_: p::Biome) {}",
        )],
    );
    assert_eq!(
        entry(&uses, "type:world::Biome").paths,
        ["world::prelude::Biome"]
    );
}

#[test]
fn a_macro_body_is_read_as_tokens() {
    let in_macro = |body: &str| format!("macro_rules! m {{ () => {{ {body} }}; }}");
    // An API struct's name followed by `{` builds it, in full unless it ends in `..`, wherever
    // a `..` nests inside.
    refused_for_the_new_field(fits_a_new_dims_field(&[(
        "src/lib.rs",
        &in_macro(&format!(
            "fn f() -> mod_api::{DIMS_LITERAL} {{ mod_api::{DIMS_LITERAL} }}"
        )),
    )]));
    refused_for_the_new_field(fits_a_new_dims_field(&[(
        "src/lib.rs",
        &in_macro("mod_api::DimensionSettings { min_y: f(..), height: 1, logical_height: 2 }"),
    )]));
    let based = in_macro("mod_api::Settings { height: 1, ..Default::default() }");
    assert_eq!(
        built_in(&[("src/lib.rs", &based)], "type:Settings"),
        Some(Built::Base)
    );
    // Followed by `(`, a tuple struct is built.
    let pos = in_macro("mod_api::Pos(1, 2)");
    assert_eq!(
        built_in(&[("src/lib.rs", &pos)], "type:Pos"),
        Some(Built::Full)
    );
    // A metavariable followed by `(` may be a tuple struct's name.
    let maker = "macro_rules! make { ($t:ident) => { $t(1, 2) }; }";
    assert_eq!(
        built_in(
            &[("src/m.rs", maker), ("src/lib.rs", "fn f(_: Pos) {}")],
            "type:Pos"
        ),
        Some(Built::Full)
    );
    // A `..` inside a nested bracket is a value, not a rest.
    let nested = in_macro("mod_api::Pos((..), 2)");
    assert_eq!(
        built_in(&[("src/lib.rs", &nested)], "type:Pos"),
        Some(Built::Full)
    );
    // A literal of a name the scan cannot place may build any API struct the mod names; a
    // brace after an unknown name that holds no fields is no literal.
    let unplaced = in_macro("let _ = Done { a: 1 };");
    assert_eq!(
        built_in(
            &[
                ("src/m.rs", &unplaced),
                ("src/lib.rs", "fn f(_: Settings) {}")
            ],
            "type:Settings"
        ),
        Some(Built::Full)
    );
    let block = in_macro("let _ = Done { 1 };");
    assert_eq!(
        built_in(
            &[("src/m.rs", &block), ("src/lib.rs", "fn f(_: Settings) {}")],
            "type:Settings"
        ),
        Some(Built::None)
    );
    // Nor is a trait's name the mod defines.
    let traits = format!("trait Wrapper {{}}\n{}", in_macro("Wrapper { a: 1 }"));
    assert_eq!(
        built_in(
            &[
                ("src/m.rs", &traits),
                ("src/lib.rs", "fn f(_: Settings) {}")
            ],
            "type:Settings"
        ),
        Some(Built::None)
    );
    // A path written in a macro body is a path the mod writes.
    let mut made_for = wider();
    let mut biome = api_type("type:world::Biome", "", false, vec![]);
    biome.paths = vec!["world::Biome".into(), "prelude::Biome".into()];
    made_for.types.push(biome);
    let uses = scan_files(
        made_for,
        &[(
            "src/lib.rs",
            &in_macro("fn f(_: mod_api::prelude::Biome) {}"),
        )],
    );
    assert_eq!(entry(&uses, "type:world::Biome").paths, ["prelude::Biome"]);
}

#[test]
fn parser_names_are_read_as_written() {
    // A raw identifier, after a byte-order mark.
    let raw =
        format!("\u{feff}fn f() -> mod_api::DimensionSettings {{ mod_api::r#{DIMS_LITERAL} }}");
    refused_for_the_new_field(fits_a_new_dims_field(&[("src/lib.rs", &raw)]));
    let settings = |source: &str| built_in(&[("src/lib.rs", source)], "type:Settings");
    assert_eq!(
        settings("\u{feff}fn f() { let s = Settings { height: 1 }; }"),
        Some(Built::Full)
    );
    // A path from the root is never the mod's own, in a literal or an impl's target.
    let own = "struct Settings { mine: u8 }\n";
    assert_eq!(
        settings(&format!(
            "{own}fn f() {{ let s = ::Settings {{ height: 1 }}; }}"
        )),
        Some(Built::Full)
    );
    assert_eq!(
        settings(&format!(
            "{own}impl Make for ::Settings {{ fn make() -> Self {{ Self {{ height: 1 }} }} }}"
        )),
        Some(Built::Full)
    );
    // A named struct's name as a value builds nothing; a unit struct matched by name does.
    assert_eq!(settings("fn f() { let s = Settings; }"), Some(Built::None));
    let mut pieces = wider();
    let mut record = api_type("type:Marker", "", true, vec![]);
    record = Type::new(
        record.id.clone(),
        "",
        None,
        TypeContract {
            kind: TypeKind::UnitStruct,
            ..record.contract.clone()
        },
        vec![],
        vec![],
    );
    pieces.types.push(record);
    let matched = scan_files(
        pieces,
        &[(
            "src/lib.rs",
            "use mod_api::Marker;\nfn f(m: Marker) { match m { Marker => {} } }",
        )],
    );
    assert_eq!(entry(&matched, "type:Marker").built, Some(Built::Full));
}

#[test]
fn a_macro_body_builds_a_tuple_or_unit_struct_named_where_a_value_may_stand() {
    // A tuple struct's constructor as a closure's or a statement's value.
    let pos = |files: &[(&str, &str)]| built_in(files, "type:Pos");
    for source in [
        "use mod_api::Pos;\nmacro_rules! m { () => { |_: u8| Pos }; }",
        "use mod_api::Pos;\nmacro_rules! m { () => { let x = 1; x; Pos }; }",
    ] {
        assert_eq!(
            pos(&[("src/lib.rs", source)]),
            Some(Built::Full),
            "{source}"
        );
    }
    // A unit struct as a block's value or behind `&` in a macro body; as a type it is none.
    let marker = || {
        let mut pieces = wider();
        let mut record = api_type("type:Marker", "", true, vec![]);
        record = Type::new(
            record.id.clone(),
            "",
            None,
            TypeContract {
                kind: TypeKind::UnitStruct,
                ..record.contract.clone()
            },
            vec![],
            vec![],
        );
        pieces.types.push(record);
        pieces
    };
    let marker_built = |source: &str| {
        entry(
            &scan_files(marker(), &[("src/lib.rs", source)]),
            "type:Marker",
        )
        .built
    };
    for source in [
        "use mod_api::Marker;\nmacro_rules! m { () => { Marker }; }",
        "use mod_api::Marker;\nmacro_rules! m { () => { f(&Marker) }; }",
    ] {
        assert_eq!(marker_built(source), Some(Built::Full), "{source}");
    }
    for source in [
        "use mod_api::Marker;\nmacro_rules! m { () => { fn f() -> Marker { make() } }; }",
        "use mod_api::Marker;\nmacro_rules! m { () => { impl Tr for Marker {} }; }",
        "use mod_api::Marker;\nmacro_rules! m { () => { impl Marker {} }; }",
        "use mod_api::Marker;\nmacro_rules! m { () => { let v: Vec<Marker> = x; }; }",
        "macro_rules! m { () => { use mod_api::{Marker, Pos}; }; }",
    ] {
        assert_eq!(marker_built(source), Some(Built::None), "{source}");
    }
    // A literal from the standard library is no doubt.
    assert_eq!(
        built_in(
            &[(
                "src/lib.rs",
                "fn f(_: Settings) { let r = std::ops::Range { start: 0, end: 5 }; }"
            )],
            "type:Settings"
        ),
        Some(Built::None)
    );
    // In a macro body's parentheses, `..` may be a range (`Pos(0, ..)` is a `RangeFull`
    // value), so a pattern's rest there counts as a full build too.
    let in_macro = |body: &str| format!("macro_rules! m {{ () => {{ {body} }}; }}");
    assert_eq!(
        pos(&[("src/lib.rs", &in_macro("vec![mod_api::Pos(0, ..10)]"))]),
        Some(Built::Full)
    );
    assert_eq!(
        pos(&[("src/lib.rs", &in_macro("let mod_api::Pos(x, ..) = p;"))]),
        Some(Built::Full)
    );
}

#[test]
fn large_generated_files_scan_in_linear_time() {
    // Each would take quadratic time in some walk over the file: many macro bodies, many
    // `type` keywords in one body, a long path, and many `Self`s among many impls.
    let n = 20_000;
    for source in [
        format!("fn f(_: Settings) {{ {} }}", "a!(); ".repeat(n)),
        format!("m! {{ {} }}\nfn f(_: Settings) {{}}", "type ".repeat(n)),
        format!("fn f(_: Settings) {{ let x = {}y; }}", "a::".repeat(n)),
        format!(
            "fn f(_: Settings) {{}}\n{}",
            "impl S { fn f() -> Self { Self } }\n".repeat(n / 2)
        ),
    ] {
        let started = std::time::Instant::now();
        assert!(ids(&scan_on(wider(), &source)).contains(&"type:Settings"));
        let took = started.elapsed();
        assert!(
            took < std::time::Duration::from_millis(1000),
            "{took:?}: {}",
            &source[..40]
        );
    }
}

#[test]
fn self_after_a_nested_impl_is_the_outer_impls() {
    // The inner impl's body ends before `Self { … }`, which is the outer impl's own struct.
    let source = "struct Settings { mine: u8 }\nimpl Settings { fn f(_: Pos) -> Self { impl Clone for Q { fn clone(&self) -> Q { Q } } Self { mine: 1 } } }";
    let uses = scan_on(wider(), source);
    assert_eq!(entry(&uses, "type:Pos").built, Some(Built::None));
}
