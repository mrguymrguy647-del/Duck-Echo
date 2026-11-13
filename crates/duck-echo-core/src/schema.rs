//! JSON Schema generation from the Rust recipe types.

use crate::recipe::Recipe;
use crate::song::Song;

/// The JSON Schema for [`Recipe`], as a JSON value. Field docs become
/// `description`s and serde defaults become `default`s, so an agent can read
/// everything it needs from this one document.
pub fn recipe_schema() -> serde_json::Value {
    let mut schema = schemars::schema_for!(Recipe);
    let obj = schema.ensure_object();
    obj.insert(
        "$id".into(),
        "https://github.com/mrguymrguy647-del/Duck-Echo/schema/recipe.schema.json".into(),
    );
    obj.insert("title".into(), "Duck Echo recipe v0".into());
    let mut value = serde_json::to_value(schema).expect("schema serializes");
    tidy_defaults(&mut value);
    value
}

/// schemars widens `f32` defaults to `f64`, which prints `0.05` as
/// `0.05000000074505806`. Re-parse such numbers from their shortest `f32`
/// text so agents see the value that was written.
fn tidy_defaults(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if k == "default" {
                    narrow_floats(child);
                } else {
                    tidy_defaults(child);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(tidy_defaults),
        _ => {}
    }
}

/// Rewrite every float inside a default value (including nested object
/// defaults such as `envelope`) from its shortest `f32` text.
fn narrow_floats(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or_default();
            let narrow = f as f32;
            if (narrow as f64 - f).abs() <= f.abs() * 1e-7 {
                if let Ok(tidy) = serde_json::from_str::<serde_json::Value>(&format!("{narrow:?}"))
                {
                    *v = tidy;
                }
            }
        }
        serde_json::Value::Object(map) => map.values_mut().for_each(narrow_floats),
        serde_json::Value::Array(items) => items.iter_mut().for_each(narrow_floats),
        _ => {}
    }
}

/// The schema as pretty-printed JSON text.
pub fn recipe_schema_json() -> String {
    serde_json::to_string_pretty(&recipe_schema()).expect("schema serializes")
}

/// The JSON Schema for [`Song`], as a JSON value. Built the same way as
/// [`recipe_schema`].
pub fn song_schema() -> serde_json::Value {
    let mut schema = schemars::schema_for!(Song);
    let obj = schema.ensure_object();
    obj.insert(
        "$id".into(),
        "https://github.com/mrguymrguy647-del/Duck-Echo/schema/song.schema.json".into(),
    );
    obj.insert("title".into(), "Duck Echo song v0".into());
    let mut value = serde_json::to_value(schema).expect("schema serializes");
    tidy_defaults(&mut value);
    value
}

/// The song schema as pretty-printed JSON text.
pub fn song_schema_json() -> String {
    serde_json::to_string_pretty(&song_schema()).expect("schema serializes")
}
