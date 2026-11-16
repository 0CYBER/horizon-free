// Shared by the Game regression and the standalone mod proof; only public mod API calls.
use mod_api::{ComponentId, Host, Identifier, ItemResource, ItemStack};
use serde_json::{Value, json};

pub struct Outcome {
    pub item: ItemResource,
    pub material: ItemResource,
    pub values: Value,
}

pub fn evaluate(host: &dyn Host, input: &Value) -> Outcome {
    let key = host
        .item_key(&Identifier::parse(input["stack"]["id"].as_str().unwrap()).unwrap())
        .unwrap();
    let mut item = ItemResource::of(&ItemStack::of(key));
    if let Some(components) = input["stack"]["components"].as_object() {
        for (key, value) in components {
            if let Some(id) = key.strip_prefix('!') {
                item = item.without_component(host, &ComponentId::new(id).unwrap());
            } else {
                item = item
                    .with_component(host, &ComponentId::new(key).unwrap(), json_tag(value))
                    .unwrap();
            }
        }
    }
    let key = host
        .item_key(&Identifier::parse(input["repair"].as_str().unwrap()).unwrap())
        .unwrap();
    let material = ItemResource::of(&ItemStack::of(key));
    let bar = item
        .bar(host)
        .map(|bar| json!({"width":bar.width,"color":bar.color}));
    let values = json!({"name":input["name"],"damageable":item.damageable(host),
        "foil":item.foil(host),"use_duration":item.use_duration(host),"bar":bar,
        "rarity":item.rarity(host).as_str(),"repairable":item.repairable(host,&material),
        "enchantable":item.enchantable(host)});
    Outcome {
        item,
        material,
        values,
    }
}

fn json_tag(value: &Value) -> mod_api::Tag {
    use mod_api::Tag;
    match value {
        Value::Null => Tag::End,
        Value::Bool(value) => Tag::Byte(i8::from(*value)),
        Value::Number(value) => match value.as_i64() {
            Some(value) if i32::try_from(value).is_ok() => Tag::Int(value as i32),
            Some(value) => Tag::Long(value),
            None => Tag::Double(value.as_f64().unwrap().to_bits()),
        },
        Value::String(value) => Tag::String(value.as_str().into()),
        Value::Array(values) => Tag::List(values.iter().map(json_tag).collect()),
        Value::Object(values) => Tag::compound(
            values
                .iter()
                .map(|(key, value)| (key.as_str(), json_tag(value))),
        ),
    }
}
