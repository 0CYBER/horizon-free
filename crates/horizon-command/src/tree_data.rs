//! The `/data` and `/tag` command trees (DataCommands, TagCommand).
use super::{
    Dispatcher,
    args::ArgType,
    vanilla::{ENTITIES, GAMEMASTERS},
};

const ENTITY: ArgType = ArgType::Entity {
    single: true,
    players: false,
};

/// `ArgProvider.wrap` for each of entity, block and storage: calls `then`
/// with the node holding the provider's argument.
fn providers(
    d: &mut Dispatcher,
    parent: usize,
    role: &str,
    mut then: impl FnMut(&mut Dispatcher, usize),
) {
    let e = d.literal(parent, "entity");
    let a = d.argument(e, role, ENTITY);
    then(d, a);
    let b = d.literal(parent, "block");
    let a = d.argument(b, &format!("{role}Pos"), ArgType::BlockPos);
    then(d, a);
    let s = d.literal(parent, "storage");
    let a = d.argument(s, role, ArgType::Id("storages"));
    then(d, a);
}

/// LootContextSources.addContextSources: `default`, `block <computePos>`
/// and `entity <computeTarget>`, each completed by `then`.
pub fn context_sources(
    d: &mut Dispatcher,
    node: usize,
    then: &mut dyn FnMut(&mut Dispatcher, usize),
) {
    let default = d.literal(node, "default");
    then(d, default);
    let block = d.literal(node, "block");
    let pos = d.argument(block, "computePos", ArgType::BlockPos);
    then(d, pos);
    let entity = d.literal(node, "entity");
    let target = d.argument(entity, "computeTarget", ENTITY);
    then(d, target);
}

pub fn register(d: &mut Dispatcher, root: usize) {
    // compute (default | block <pos> | entity <target>) (float <provider> [<scale>] | integer <provider>)
    let compute = d.literal(root, "compute");
    d.requires(compute, GAMEMASTERS);
    context_sources(d, compute, &mut |d, node| {
        let f = d.literal(node, "float");
        let p = d.argument(
            f,
            "provider",
            ArgType::ResourceOrId("minecraft:context_float_provider"),
        );
        d.executes(p, "compute");
        let scale = d.argument(
            p,
            "scale",
            ArgType::Float {
                min: f32::MIN,
                max: f32::MAX,
            },
        );
        d.executes(scale, "compute");
        let i = d.literal(node, "integer");
        let p = d.argument(
            i,
            "provider",
            ArgType::ResourceOrId("minecraft:context_int_provider"),
        );
        d.executes(p, "compute");
    });

    let data = d.literal(root, "data");
    d.requires(data, GAMEMASTERS);

    let merge = d.literal(data, "merge");
    providers(d, merge, "target", |d, a| {
        let nbt = d.argument(a, "nbt", ArgType::Nbt);
        d.executes(nbt, "data.merge");
    });
    let get = d.literal(data, "get");
    providers(d, get, "target", |d, a| {
        d.executes(a, "data.get");
        let path = d.argument(a, "path", ArgType::NbtPath);
        d.executes(path, "data.get");
        let scale = d.argument(
            path,
            "scale",
            ArgType::Double {
                min: f64::MIN,
                max: f64::MAX,
            },
        );
        d.executes(scale, "data.get");
    });
    let remove = d.literal(data, "remove");
    providers(d, remove, "target", |d, a| {
        let path = d.argument(a, "path", ArgType::NbtPath);
        d.executes(path, "data.remove");
    });
    let modify = d.literal(data, "modify");
    providers(d, modify, "target", |d, a| {
        let target_path = d.argument(a, "targetPath", ArgType::NbtPath);
        // insert <index> | prepend | append | set | merge, then the source.
        let mut ops = Vec::new();
        let insert = d.literal(target_path, "insert");
        let index = d.argument(
            insert,
            "index",
            ArgType::Integer {
                min: i32::MIN,
                max: i32::MAX,
            },
        );
        ops.push(index);
        for op in ["prepend", "append", "set", "merge"] {
            ops.push(d.literal(target_path, op));
        }
        for op in ops {
            let from = d.literal(op, "from");
            providers(d, from, "source", |d, s| {
                d.executes(s, "data.modify");
                let sp = d.argument(s, "sourcePath", ArgType::NbtPath);
                d.executes(sp, "data.modify");
            });
            let string = d.literal(op, "string");
            providers(d, string, "source", |d, s| {
                d.executes(s, "data.modify");
                let sp = d.argument(s, "sourcePath", ArgType::NbtPath);
                d.executes(sp, "data.modify");
                let int = || ArgType::Integer {
                    min: i32::MIN,
                    max: i32::MAX,
                };
                let start = d.argument(sp, "start", int());
                d.executes(start, "data.modify");
                let end = d.argument(start, "end", int());
                d.executes(end, "data.modify");
            });
            let compute = d.literal(op, "compute");
            context_sources(d, compute, &mut |d, node| {
                for (kind, registry) in [
                    ("float", "minecraft:context_float_provider"),
                    ("integer", "minecraft:context_int_provider"),
                ] {
                    let k = d.literal(node, kind);
                    let p = d.argument(k, "provider", ArgType::ResourceOrId(registry));
                    d.executes(p, "data.modify");
                }
            });
            let value = d.literal(op, "value");
            let v = d.argument(value, "value", ArgType::NbtTag);
            d.executes(v, "data.modify");
        }
    });

    // tag <targets> add|remove <name> | list
    let tag = d.literal(root, "tag");
    d.requires(tag, GAMEMASTERS);
    let targets = d.argument(tag, "targets", ENTITIES);
    for word in ["add", "remove"] {
        let w = d.literal(targets, word);
        let name = d.argument(w, "name", ArgType::Word);
        d.executes(
            name,
            if word == "add" {
                "tag.add"
            } else {
                "tag.remove"
            },
        );
    }
    let list = d.literal(targets, "list");
    d.executes(list, "tag.list");
}
