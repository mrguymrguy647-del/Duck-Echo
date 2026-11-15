//! JSON Schema generation from the Rust recipe types.

use crate::applies::{Applies, ALL as APPLIES};
use crate::ranges::{Range, ALL};
use crate::recipe::Recipe;
use crate::song::Song;

/// The JSON Schema for [`Recipe`], as a JSON value. Field docs become
/// `description`s and serde defaults become `default`s, so an agent can read
/// everything it needs from this one document.
pub fn recipe_schema() -> serde_json::Value {
    let mut value = recipe_structure();
    apply_ranges(&mut value, "Recipe");
    apply_applies(&mut value);
    value
}

/// The recipe schema without the ranges: what the Rust types alone say.
/// The structure walker ([`crate::structure`]) reads this one, so that
/// every range is checked in one place, [`crate::validate`], with one
/// wording, exactly as before the ranges were written into the schema.
pub(crate) fn recipe_structure() -> serde_json::Value {
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

/// Write every range of [`crate::ranges::ALL`] into the schema as `minimum`
/// and `maximum` (or `exclusiveMinimum`), so agents and the Studio's
/// sliders read the same limits that validation enforces. `root` is the
/// type at the top of the schema (`Recipe` or `Song`); every other type is
/// under `$defs`. Returns the ranges that found no place in this schema.
fn apply_ranges(schema: &mut serde_json::Value, root: &str) -> Vec<Range> {
    let mut unused = Vec::new();
    for range in ALL {
        let mut found = false;
        for target in targets(schema, root, range) {
            // The number itself, or each item of a list of numbers.
            let number = match range.field.strip_suffix("[]") {
                Some(_) => target.get_mut("items"),
                None => Some(target),
            };
            if let Some(serde_json::Value::Object(number)) = number {
                number.remove("minimum");
                if range.above_min {
                    number.insert("exclusiveMinimum".into(), tidy(range.min));
                } else {
                    number.insert("minimum".into(), tidy(range.min));
                }
                number.insert("maximum".into(), tidy(range.max));
                found = true;
            }
        }
        if !found {
            unused.push(*range);
        }
    }
    unused
}

/// Write every rule of [`crate::applies::ALL`] into the schema, on the
/// field it is about, as `x-applies-when` (see [`crate::applies`]).
/// Returns the rules that found no place in this schema.
fn apply_applies(schema: &mut serde_json::Value) -> Vec<Applies> {
    let mut unused = Vec::new();
    for rule in APPLIES {
        match schema["$defs"][rule.owner]["properties"].get_mut(rule.field) {
            Some(serde_json::Value::Object(field)) => {
                field.insert("x-applies-when".into(), rule.to_json());
            }
            _ => unused.push(*rule),
        }
    }
    unused
}

/// A whole number as an integer (`2`, not `2.0`), anything else as it is.
fn tidy(v: f64) -> serde_json::Value {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        serde_json::Value::from(v as i64)
    } else {
        serde_json::Value::from(v)
    }
}

/// The places in the schema that hold `range`'s number.
fn targets<'s>(
    schema: &'s mut serde_json::Value,
    root: &str,
    range: &Range,
) -> Vec<&'s mut serde_json::Value> {
    let field = range.field.trim_end_matches("[]");
    let effect = ["Echo", "Reverb", "Distortion", "Bitcrush"].contains(&range.owner);
    let mut out = Vec::new();
    let Some(top) = schema.as_object_mut() else {
        return out;
    };
    for (key, value) in top.iter_mut() {
        match key.as_str() {
            // The top level of the document.
            "properties" if range.owner == root => out.extend(value.get_mut(field)),
            "$defs" => {
                let Some(defs) = value.as_object_mut() else {
                    continue;
                };
                for (name, def) in defs.iter_mut() {
                    if effect && name == "Effect" {
                        // An effect is one of four shapes, told apart by `type`.
                        let kind = range.owner.to_lowercase();
                        for shape in def["oneOf"].as_array_mut().into_iter().flatten() {
                            if shape["properties"]["type"]["const"] == kind.as_str() {
                                out.extend(shape["properties"].get_mut(field));
                            }
                        }
                    } else if name == range.owner && field.is_empty() {
                        // A type that is a number or a name: the number part.
                        for shape in def["anyOf"].as_array_mut().into_iter().flatten() {
                            if shape["type"] == "number" {
                                out.push(shape);
                            }
                        }
                    } else if name == range.owner {
                        out.extend(def["properties"].get_mut(field));
                    }
                }
            }
            _ => {}
        }
    }
    out
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
    let mut value = song_structure();
    apply_ranges(&mut value, "Song");
    apply_applies(&mut value);
    value
}

/// The song schema without the ranges (see [`recipe_structure`]).
pub(crate) fn song_structure() -> serde_json::Value {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_applies_rule_is_written_into_both_schemas() {
        let mut recipe = recipe_structure();
        let mut song = song_structure();
        assert_eq!(apply_applies(&mut recipe), []);
        assert_eq!(apply_applies(&mut song), []);
        let duty = &recipe_schema()["$defs"]["Layer"]["properties"]["duty"];
        assert_eq!(
            duty["x-applies-when"],
            serde_json::json!({"source": ["square"]})
        );
    }

    #[test]
    fn every_range_is_written_into_a_schema() {
        let mut recipe = serde_json::to_value(schemars::schema_for!(Recipe)).unwrap();
        let mut song = serde_json::to_value(schemars::schema_for!(Song)).unwrap();
        let not_in_recipe = apply_ranges(&mut recipe, "Recipe");
        let not_in_song = apply_ranges(&mut song, "Song");
        // The song schema holds the recipe's types too (instruments), so it
        // has a place for every range.
        assert_eq!(not_in_song, []);
        // The recipe schema lacks only the song's own types.
        for r in &not_in_recipe {
            assert!(
                [
                    "Song",
                    "Track",
                    "Placement",
                    "Note",
                    "PatternNote",
                    "NotePitch"
                ]
                .contains(&r.owner),
                "{}.{} has no place in the recipe schema",
                r.owner,
                r.field
            );
        }
    }

    #[test]
    fn the_schema_holds_the_numbers_of_the_table() {
        let s = recipe_schema();
        let volume = &s["$defs"]["Layer"]["properties"]["volume"];
        assert_eq!(volume["minimum"], 0);
        assert_eq!(volume["maximum"], 2);
        assert_eq!(s["properties"]["sample_rate"]["minimum"], 8000);
        assert_eq!(s["properties"]["sample_rate"]["maximum"], 192_000);
        assert_eq!(s["properties"]["channels"]["maximum"], 2);
        let duty = &s["$defs"]["Layer"]["properties"]["duty"];
        assert_eq!(duty["minimum"], 0.01);
        assert_eq!(duty["maximum"], 0.99);
        // A number of Hz, where a pitch is a number or a name.
        let hz = &s["$defs"]["Frequency"]["anyOf"][0];
        assert_eq!(
            (hz["minimum"].clone(), hz["maximum"].clone()),
            (1.into(), 20_000.into())
        );
        // Each step of an arpeggio, and each effect by its `type`.
        let step = &s["$defs"]["Arpeggio"]["properties"]["steps"]["items"];
        assert_eq!(step["minimum"], -96);
        let echo = s["$defs"]["Effect"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["properties"]["type"]["const"] == "echo")
            .unwrap();
        assert_eq!(echo["properties"]["feedback"]["maximum"], 0.99);
        // A song: its own fields, and a note's length that must be above 0.
        let song = song_schema();
        assert_eq!(song["properties"]["tempo"]["minimum"], 20);
        let length = &song["$defs"]["Note"]["properties"]["length"];
        assert_eq!(length["exclusiveMinimum"], 0);
        assert!(length.get("minimum").is_none());
        assert_eq!(
            song["$defs"]["Recipe"]["properties"]["variation"]["maximum"],
            1
        );
    }
}
