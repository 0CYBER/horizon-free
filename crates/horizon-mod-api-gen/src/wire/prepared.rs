//! The prepared transport is part of the Wire ABI, beyond the public callback signature.
use super::{Method, exports};

pub(super) const PREPARE: u32 = 0xffff_ff10;
pub(super) const RELEASE: u32 = 0xffff_ff11;
pub(super) const SLOTS: [&str; 8] = [
    "head", "chest", "legs", "feet", "mainhand", "offhand", "body", "saddle",
];

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub name: String,
    pub id: u32,
    pub input: Vec<&'static str>,
    pub output: Vec<&'static str>,
}
#[derive(Clone, Debug)]
pub(super) struct Abi {
    pub hook: String,
    pub parameter: String,
    pub entries: Vec<Entry>,
}

pub(super) fn describe(
    methods: &[Method],
    markers: &[(String, String)],
) -> Result<Vec<Abi>, String> {
    markers
        .iter()
        .map(|(hook, parameter)| {
            let ask = methods
                .iter()
                .filter(|m| exports().contains(&m.owner))
                .position(|m| m.owner == "ItemPart" && m.name == *hook)
                .ok_or_else(|| format!("prepared hook {hook} has no ItemPart export"))?
                as u32;
            Ok(Abi {
                hook: hook.clone(),
                parameter: parameter.clone(),
                entries: vec![
                    Entry {
                        name: format!("prepare_{hook}"),
                        id: PREPARE,
                        input: vec![
                            "slot:u32",
                            "generation:u32",
                            "stack:crate::ItemStack",
                            "kinds:Vec<String>",
                            "reads:Vec<crate::EquipReadRow>",
                        ],
                        output: vec!["token:(u32,u32)"],
                    },
                    Entry {
                        name: format!("ask_{hook}"),
                        id: ask,
                        input: vec![
                            "part:crate::wire::Id",
                            "slot:u32",
                            "generation:u32",
                            "equipment_slot:u32",
                            "entity:crate::LivingRef",
                            "kind:u32",
                            "initial:bool",
                        ],
                        output: vec!["answer:()", "entity:crate::LivingRef", "can_equip:bool"],
                    },
                    Entry {
                        name: format!("release_{hook}"),
                        id: RELEASE,
                        input: vec!["slot:u32", "generation:u32"],
                        output: vec!["answer:()"],
                    },
                ],
            })
        })
        .collect()
}

pub(super) fn fingerprint(abis: &[Abi]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |text: &str| {
        for byte in text.bytes().chain(std::iter::once(0)) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(
        "prepared ABI: horizon_in(u32)->u32; named entries()->u64(pointer,length); instance-scoped slot and generation; kind ids=index in preparation",
    );
    feed(&format!("equipment slot ids:{SLOTS:?}"));
    for abi in abis {
        feed(&abi.hook);
        feed(&abi.parameter);
        for entry in &abi.entries {
            feed(&entry.name);
            feed(&entry.id.to_string());
            feed("input");
            for ty in &entry.input {
                feed(ty);
            }
            feed("output");
            for ty in &entry.output {
                feed(ty);
            }
        }
    }
    hash
}

/// Also mix handwritten Wire layouts, so an unchanged type name cannot hide a codec change.
pub(super) fn layout_code(abis: &[Abi]) -> String {
    if abis.is_empty() {
        return format!("{}u64", fingerprint(abis));
    }
    let mut code = format!("let mut hash = {}u64;", fingerprint(abis));
    for abi in abis {
        for entry in &abi.entries {
            for argument in entry.input.iter().chain(&entry.output) {
                let (_, ty) = argument
                    .split_once(':')
                    .expect("prepared argument has a role and type");
                code.push_str(&format!("hash=crate::wire::layout_mix(hash,<{ty} as crate::wire::Wire>::layout(&mut vec![]));"));
            }
        }
    }
    code.push_str("hash");
    code
}
