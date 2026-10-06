use horizon_assets::blockstate::{BlockStateDefinition, Condition, Variant, VariantSet};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
fn variant(v: &Variant) -> Value {
    json!({"model":v.model,"rotations":v.rotations,"uvlock":v.uv_lock})
}
fn model(v: &VariantSet) -> Value {
    match v {
        VariantSet::Single(v) => json!({"single":variant(v)}),
        VariantSet::Weighted(entries) => {
            json!({"weighted":entries.iter().map(|(v,w)|json!([variant(v),w])).collect::<Vec<_>>()})
        }
    }
}
fn condition(v: &Condition) -> Value {
    match v {
        Condition::Or(v) => json!({"or":v.iter().map(condition).collect::<Vec<_>>()}),
        Condition::And(v) => json!({"and":v.iter().map(condition).collect::<Vec<_>>()}),
        Condition::Properties(v) => {
            json!({"properties":v.iter().map(|(key,terms)| (key, terms.iter().map(|t|json!({"value":t.value,"negated":t.negated})).collect::<Vec<_>>())).collect::<BTreeMap<_,_>>()})
        }
    }
}
fn definition(v: &BlockStateDefinition) -> Value {
    json!({"variants":v.variants.as_ref().map(|v|v.iter().map(|(k,v)|json!([k,model(v)])).collect::<Vec<_>>()),"multipart":v.multipart.as_ref().map(|v|v.iter().map(|s|json!({"when":s.condition.as_ref().map(condition),"apply":model(&s.variant)})).collect::<Vec<_>>())})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let row: Vec<String> = serde_json::from_str(&line?)?;
        let result = match BlockStateDefinition::from_json(&row[1]) {
            Ok(v) => json!({"name":row[0],"definition":definition(&v)}),
            Err(_) => json!({"name":row[0],"error":true}),
        };
        writeln!(output, "{result}")?;
    }
    Ok(())
}
