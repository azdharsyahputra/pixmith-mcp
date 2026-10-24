//! Tool input schemas: generated with schemars, then reduced to the plain JSON Schema subset every MCP client
//! understands (Gemini and some IDEs reject `$ref`, type arrays, integer `format`s and `const` unions).

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

pub fn input_schema<T: JsonSchema>() -> Map<String, Value> {
    let settings = SchemaSettings::draft07().with(|s| {
        s.inline_subschemas = true;
        s.meta_schema = None;
    });
    let schema = settings.into_generator().into_root_schema_for::<T>();
    let mut v = schema.to_value();
    clean(&mut v);
    let mut obj = match v {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    obj.remove("title");
    obj.remove("definitions");
    obj.remove("$defs");
    obj.insert("type".into(), "object".into());
    obj.entry("properties").or_insert_with(|| Value::Object(Map::new()));
    obj
}

fn is_null_schema(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("null") || v.get("const").is_some_and(Value::is_null)
}

fn clean(v: &mut Value) {
    match v {
        Value::Array(a) => a.iter_mut().for_each(clean),
        Value::Object(m) => {
            m.remove("$schema");
            // Only the keyword (a string), never a property that happens to be called "format".
            if m.get("format").is_some_and(Value::is_string) {
                m.remove("format");
            }
            if let Some(Value::Array(e)) = m.get_mut("enum") {
                e.retain(|v| !v.is_null());
            }
            if m.get("default").is_some_and(Value::is_null) {
                m.remove("default");
            }
            // ["integer", "null"] -> "integer"
            if let Some(Value::Array(types)) = m.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    let t = types[0].clone();
                    m.insert("type".into(), t);
                }
            }
            for key in ["anyOf", "oneOf"] {
                let Some(Value::Array(mut alts)) = m.remove(key) else { continue };
                alts.retain(|a| !is_null_schema(a));
                // A union of constants is a string enum.
                let consts = |a: &Value| -> Option<Vec<Value>> {
                    match (a.get("const"), a.get("enum")) {
                        (Some(c), _) if c.is_string() => Some(vec![c.clone()]),
                        (None, Some(Value::Array(e))) if e.iter().all(Value::is_string) => Some(e.clone()),
                        _ => None,
                    }
                };
                if !alts.is_empty() && alts.iter().all(|a| consts(a).is_some()) {
                    let values: Vec<Value> = alts.iter().filter_map(consts).flatten().collect();
                    let notes: Vec<String> = alts
                        .iter()
                        .filter_map(|a| {
                            let d = a.get("description")?.as_str()?;
                            Some(format!("{}: {d}", a.get("const")?.as_str()?))
                        })
                        .collect();
                    m.insert("type".into(), "string".into());
                    m.insert("enum".into(), Value::Array(values));
                    if !notes.is_empty() {
                        let base = m.get("description").and_then(Value::as_str).unwrap_or("").to_string();
                        let joined = notes.join("; ");
                        let d = if base.is_empty() { joined } else { format!("{base} ({joined})") };
                        m.insert("description".into(), d.into());
                    }
                } else if alts.len() == 1 {
                    if let Value::Object(inner) = alts.remove(0) {
                        for (k, val) in inner {
                            if k == "description" && m.contains_key("description") {
                                continue;
                            }
                            m.insert(k, val);
                        }
                    }
                } else if !alts.is_empty() {
                    m.insert(key.into(), Value::Array(alts));
                }
            }
            for val in m.values_mut() {
                clean(val);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    #[allow(dead_code)]
    enum Mode {
        /// first
        A,
        B,
    }

    #[derive(Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Args {
        /// A count.
        n: Option<u32>,
        /// The mode.
        mode: Option<Mode>,
        list: Vec<u8>,
    }

    #[test]
    fn schemas_are_flat_and_plain() {
        let s = Value::Object(input_schema::<Args>());
        let text = s.to_string();
        assert!(!text.contains("$ref"), "{text}");
        assert!(!text.contains("null"), "{text}");
        assert!(!text.contains("format"), "{text}");
        assert_eq!(s["properties"]["n"]["type"], "integer");
        let mut e: Vec<String> = serde_json::from_value(s["properties"]["mode"]["enum"].clone()).unwrap();
        e.sort();
        assert_eq!(e, ["a", "b"]);
        assert!(s["properties"]["mode"]["description"].as_str().unwrap().contains("a: first"));
        assert_eq!(s["required"], serde_json::json!(["list"]));
    }
}
