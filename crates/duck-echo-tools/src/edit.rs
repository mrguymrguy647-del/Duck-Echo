//! Recipe edits by path, and writing recipe documents.
//!
//! An edit is a list of operations on the recipe document as written, using
//! the same path syntax as validation errors (`layers[0].pitch.start`).
//! Applying edits to the written document, rather than to the fully
//! defaulted recipe, keeps a short recipe short.

use crate::ToolError;
use duck_echo_core::{load_str, Code, Format, Issue, Loaded, Report};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

/// One edit operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditOp {
    /// Where to edit, as in error messages: `layers[0].pitch.start`,
    /// `effects[1]`, `master.gain`. An index equal to the array length
    /// appends. Missing tables on the way are created.
    pub path: String,
    /// The new value (any JSON). Required unless `remove` is true.
    #[serde(default)]
    pub value: Option<Value>,
    /// Remove the field or array item at `path` instead of setting it.
    #[serde(default)]
    pub remove: bool,
}

enum Seg {
    Key(String),
    Index(usize),
}

fn parse_path(path: &str) -> Result<Vec<Seg>, String> {
    let mut segs = Vec::new();
    for part in path.split('.') {
        if part.is_empty() {
            return Err("empty segment (two dots, or a leading or trailing dot)".into());
        }
        let (key, mut rest) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (part, ""),
        };
        if !key.is_empty() {
            segs.push(Seg::Key(key.to_string()));
        } else if segs.is_empty() {
            return Err("a path starts with a field name".into());
        }
        while !rest.is_empty() {
            let close = rest
                .find(']')
                .ok_or_else(|| format!("unclosed `[` in `{part}`"))?;
            let index = rest[1..close]
                .parse::<usize>()
                .map_err(|_| format!("`{}` is not an array index", &rest[1..close]))?;
            segs.push(Seg::Index(index));
            rest = &rest[close + 1..];
            if !rest.is_empty() && !rest.starts_with('[') {
                return Err(format!("unexpected `{rest}` after an index"));
            }
        }
    }
    if segs.is_empty() {
        return Err("empty path".into());
    }
    Ok(segs)
}

fn op_error(op: &EditOp, i: usize, why: impl Into<String>) -> ToolError {
    ToolError::Invalid(Report::from_error(
        Issue::error(
            Code::Invalid,
            op.path.clone(),
            format!("edit {i} cannot apply: {}", why.into()),
        )
        .got(serde_json::json!({ "op": i })),
    ))
}

/// Apply edit operations to a recipe document, in order. Fails on the
/// first operation that cannot apply (bad path, index past the end); the
/// document is then left partly edited, so callers apply edits to a copy.
pub fn apply_edits(doc: &mut Value, ops: &[EditOp]) -> Result<(), ToolError> {
    for (i, op) in ops.iter().enumerate() {
        if op.remove == op.value.is_some() {
            return Err(op_error(
                op,
                i,
                "give either `value` or `remove: true`, not both or neither",
            ));
        }
        let segs = parse_path(&op.path).map_err(|e| op_error(op, i, e))?;
        let (last, parents) = segs.split_last().expect("non-empty");
        let mut node = &mut *doc;
        for (depth, seg) in parents.iter().enumerate() {
            let next_is_index = matches!(segs[depth + 1], Seg::Index(_));
            node = match seg {
                Seg::Key(k) => {
                    let obj = node
                        .as_object_mut()
                        .ok_or_else(|| op_error(op, i, format!("`{k}` is not inside a table")))?;
                    if !obj.contains_key(k) {
                        if op.remove {
                            return Ok(());
                        }
                        let fresh = if next_is_index {
                            Value::Array(Vec::new())
                        } else {
                            Value::Object(Map::new())
                        };
                        obj.insert(k.clone(), fresh);
                    }
                    obj.get_mut(k).expect("just ensured")
                }
                Seg::Index(n) => {
                    let arr = node
                        .as_array_mut()
                        .ok_or_else(|| op_error(op, i, format!("[{n}] is not inside an array")))?;
                    let len = arr.len();
                    if *n == len && !op.remove {
                        arr.push(if next_is_index {
                            Value::Array(Vec::new())
                        } else {
                            Value::Object(Map::new())
                        });
                    }
                    arr.get_mut(*n).ok_or_else(|| {
                        op_error(op, i, format!("index {n} is past the end (length {len})"))
                    })?
                }
            };
        }
        match last {
            Seg::Key(k) => {
                let obj = node
                    .as_object_mut()
                    .ok_or_else(|| op_error(op, i, format!("`{k}` is not inside a table")))?;
                if op.remove {
                    obj.remove(k);
                } else {
                    obj.insert(k.clone(), op.value.clone().expect("checked"));
                }
            }
            Seg::Index(n) => {
                let arr = node
                    .as_array_mut()
                    .ok_or_else(|| op_error(op, i, format!("[{n}] is not inside an array")))?;
                let len = arr.len();
                if op.remove {
                    if *n >= len {
                        return Err(op_error(
                            op,
                            i,
                            format!("index {n} is past the end (length {len})"),
                        ));
                    }
                    arr.remove(*n);
                } else if *n == len {
                    arr.push(op.value.clone().expect("checked"));
                } else if *n < len {
                    arr[*n] = op.value.clone().expect("checked");
                } else {
                    return Err(op_error(
                        op,
                        i,
                        format!("index {n} is past the end (length {len}); use {len} to append"),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Validate a recipe given as a JSON value. Line and column numbers are
/// dropped from the report, since they would refer to re-serialized text
/// the caller never saw; paths are exact.
pub fn load_value(doc: &Value) -> Result<Loaded, ToolError> {
    let text = serde_json::to_string(doc).expect("json");
    let strip = |mut r: Report| {
        for i in r.errors.iter_mut().chain(r.warnings.iter_mut()) {
            i.line = None;
            i.column = None;
        }
        r
    };
    match load_str(&text, Format::Json) {
        Ok(mut l) => {
            l.report = strip(l.report);
            Ok(l)
        }
        Err(r) => Err(ToolError::Invalid(strip(r))),
    }
}

/// The recipe document of a file, as written (no defaults filled in).
pub fn read_document(path: &Path) -> Result<Value, ToolError> {
    let src = std::fs::read_to_string(path).map_err(|e| {
        ToolError::failed(
            crate::io_code(&e),
            format!("cannot read {}: {e}", path.display()),
        )
    })?;
    let tree = match Format::from_path(path) {
        Format::Toml => duck_echo_core::tree::parse_toml(&src),
        Format::Json => duck_echo_core::tree::parse_json(&src),
    }
    .map_err(|issue| ToolError::Invalid(Report::from_error(*issue)))?;
    tree.to_json().ok_or_else(|| {
        // Only non-finite numbers or datetimes get here; validation names them.
        match duck_echo_core::load_str(&src, Format::from_path(path)) {
            Err(r) => ToolError::Invalid(r),
            Ok(_) => ToolError::failed(
                Code::Invalid,
                format!("{} holds values JSON cannot represent", path.display()),
            ),
        }
    })
}

/// Top-level keys in the order a recipe file shows them.
const TOP_ORDER: &[&str] = &[
    "version",
    "name",
    "description",
    "tags",
    "seed",
    "sample_rate",
    "channels",
    "variation",
    "duration",
    "master",
    "layers",
    "effects",
];

fn drop_nulls(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|_, x| !x.is_null());
            m.values_mut().for_each(drop_nulls);
        }
        Value::Array(a) => a.iter_mut().for_each(drop_nulls),
        _ => {}
    }
}

/// A recipe document as TOML, top-level keys in reading order (`version`
/// and the plain fields first, then `[master]`, `[[layers]]`,
/// `[[effects]]`). Comments cannot be kept.
pub fn to_toml_document(doc: &Value) -> Result<String, ToolError> {
    let mut doc = doc.clone();
    drop_nulls(&mut doc);
    let obj = doc.as_object().ok_or_else(|| {
        ToolError::failed(
            Code::TypeMismatch,
            "a recipe must be a table (a JSON object)",
        )
    })?;
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort_by_key(|k| TOP_ORDER.iter().position(|o| o == k).unwrap_or(usize::MAX));
    // A table, or a list of tables, is written as `[name]` or `[[name]]`
    // after the plain values. Anything else (an empty list included) is a
    // plain value: written after a `[table]` it would belong to that table.
    let (plain, tables): (Vec<&String>, Vec<&String>) =
        keys.into_iter().partition(|k| match &obj[k.as_str()] {
            Value::Object(_) => false,
            Value::Array(items) => {
                *k == "tags" || items.is_empty() || !items.iter().all(Value::is_object)
            }
            _ => true,
        });
    let mut out = String::new();
    for group in [plain, tables] {
        for k in group {
            let mut one = Map::new();
            one.insert(k.clone(), obj[k.as_str()].clone());
            let text = toml::to_string(&Value::Object(one)).map_err(|e| {
                ToolError::issue(Issue::error(
                    Code::Invalid,
                    k.clone(),
                    format!("cannot write `{k}` as TOML: {e}"),
                ))
            })?;
            if !out.is_empty() && !out.ends_with("\n\n") && text.starts_with('[') {
                out.push('\n');
            }
            out.push_str(&text);
        }
    }
    Ok(out)
}

/// A recipe or song document as the text of a file at `path`: TOML, or
/// pretty JSON for a `.json` path.
pub fn document_text(path: &Path, doc: &Value) -> Result<String, ToolError> {
    Ok(match Format::from_path(path) {
        Format::Toml => to_toml_document(doc)?,
        Format::Json => {
            let mut d = doc.clone();
            drop_nulls(&mut d);
            serde_json::to_string_pretty(&d).expect("json") + "\n"
        }
    })
}

/// The new text of the file at `path`, which held `old_text`, now that
/// its data is `doc`. For TOML only the values that changed are rewritten:
/// comments, blank lines, inline tables and the order of everything else
/// stay as written. JSON (which has no comments) is written whole, as
/// [`document_text`] does.
///
/// If the old text cannot be kept (it does not parse), or keeping it would
/// not give exactly `doc`, the file is written whole instead: the data is
/// always right, and the layout is kept whenever it can be.
pub fn document_text_from(path: &Path, old_text: &str, doc: &Value) -> Result<String, ToolError> {
    if Format::from_path(path) == Format::Toml {
        if let Some(text) = keep_layout(old_text, doc) {
            return Ok(text);
        }
    }
    document_text(path, doc)
}

/// `old_text` changed to hold `doc`, keeping what did not change.
fn keep_layout(old_text: &str, doc: &Value) -> Option<String> {
    let mut want = doc.clone();
    drop_nulls(&mut want);
    let Value::Object(map) = &want else {
        return None;
    };
    let mut file: toml_edit::DocumentMut = old_text.parse().ok()?;
    merge_table(file.as_table_mut(), map, true);
    let text = file.to_string();
    // The check that makes this safe: the text must read back as exactly
    // the data asked for.
    let back = parse_document(&text, Format::Toml).ok()?;
    (back == want).then_some(text)
}

/// Make `table` hold `map`. `top` is true for the document itself, where a
/// new table is written as `[name]` (or `[[name]]`) rather than inline.
fn merge_table(table: &mut toml_edit::Table, map: &Map<String, Value>, top: bool) {
    let gone: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !map.contains_key(k))
        .collect();
    for k in gone {
        table.remove(&k);
    }
    for (k, v) in map {
        match table.get_mut(k) {
            Some(item) => merge_item(item, v),
            None => {
                table.insert(k, new_item(v, top));
            }
        }
    }
}

fn merge_inline(table: &mut toml_edit::InlineTable, map: &Map<String, Value>) {
    let gone: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !map.contains_key(k))
        .collect();
    for k in gone {
        table.remove(&k);
    }
    for (k, v) in map {
        match table.get_mut(k) {
            Some(value) => merge_value(value, v),
            None => {
                table.insert(k, new_value(v));
            }
        }
    }
}

/// Make `item` hold `v`, changing as little of its text as possible.
fn merge_item(item: &mut toml_edit::Item, v: &Value) {
    use toml_edit::Item;
    if item_json(item).as_ref() == Some(v) {
        return;
    }
    match (item, v) {
        (Item::Value(value), _) => merge_value(value, v),
        (Item::Table(table), Value::Object(map)) => merge_table(table, map, false),
        (Item::ArrayOfTables(tables), Value::Array(items))
            if !items.is_empty() && items.iter().all(Value::is_object) =>
        {
            while tables.len() > items.len() {
                tables.remove(tables.len() - 1);
            }
            for (i, it) in items.iter().enumerate() {
                let Value::Object(map) = it else { continue };
                match tables.get_mut(i) {
                    Some(table) => merge_table(table, map, false),
                    None => {
                        let mut table = toml_edit::Table::new();
                        merge_table(&mut table, map, false);
                        tables.push(table);
                    }
                }
            }
        }
        (item, v) => *item = new_item(v, false),
    }
}

/// Make `value` hold `v`, keeping the comments and spaces around it.
fn merge_value(value: &mut toml_edit::Value, v: &Value) {
    use toml_edit::Value as T;
    if value_json(value).as_ref() == Some(v) {
        return;
    }
    match (value, v) {
        (T::InlineTable(table), Value::Object(map)) => merge_inline(table, map),
        (T::Array(array), Value::Array(items)) if array.len() == items.len() => {
            for (i, it) in items.iter().enumerate() {
                if let Some(slot) = array.get_mut(i) {
                    merge_value(slot, it);
                }
            }
        }
        (value, v) => {
            let decor = value.decor().clone();
            *value = new_value(v);
            *value.decor_mut() = decor;
        }
    }
}

fn new_item(v: &Value, top: bool) -> toml_edit::Item {
    match v {
        Value::Object(map) if top => {
            let mut table = toml_edit::Table::new();
            merge_table(&mut table, map, false);
            toml_edit::Item::Table(table)
        }
        Value::Array(items) if top && !items.is_empty() && items.iter().all(Value::is_object) => {
            let mut tables = toml_edit::ArrayOfTables::new();
            for it in items {
                let mut table = toml_edit::Table::new();
                if let Value::Object(map) = it {
                    merge_table(&mut table, map, false);
                }
                tables.push(table);
            }
            toml_edit::Item::ArrayOfTables(tables)
        }
        _ => toml_edit::Item::Value(new_value(v)),
    }
}

fn new_value(v: &Value) -> toml_edit::Value {
    match v {
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().unwrap_or_default().into(),
        },
        Value::String(s) => s.as_str().into(),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for it in items {
                array.push(new_value(it));
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (k, it) in map {
                table.insert(k, new_value(it));
            }
            toml_edit::Value::InlineTable(table)
        }
        // Nulls were dropped before; an empty string can never be read
        // back as null, so the final check would refuse it.
        Value::Null => "".into(),
    }
}

/// What an item holds, as JSON (`None` for something JSON cannot hold).
fn item_json(item: &toml_edit::Item) -> Option<Value> {
    use toml_edit::Item;
    match item {
        Item::None => None,
        Item::Value(v) => value_json(v),
        Item::Table(t) => t
            .iter()
            .map(|(k, it)| item_json(it).map(|j| (k.to_string(), j)))
            .collect::<Option<Map<String, Value>>>()
            .map(Value::Object),
        Item::ArrayOfTables(a) => a
            .iter()
            .map(|t| item_json(&toml_edit::Item::Table(t.clone())))
            .collect::<Option<Vec<Value>>>()
            .map(Value::Array),
    }
}

fn value_json(v: &toml_edit::Value) -> Option<Value> {
    use toml_edit::Value as T;
    Some(match v {
        T::String(s) => Value::String(s.value().clone()),
        T::Integer(i) => Value::from(*i.value()),
        T::Float(f) => Value::from(serde_json::Number::from_f64(*f.value())?),
        T::Boolean(b) => Value::Bool(*b.value()),
        T::Datetime(_) => return None,
        T::Array(a) => Value::Array(a.iter().map(value_json).collect::<Option<_>>()?),
        T::InlineTable(t) => Value::Object(
            t.iter()
                .map(|(k, it)| value_json(it).map(|j| (k.to_string(), j)))
                .collect::<Option<_>>()?,
        ),
    })
}

/// Write a recipe or song document to `path` as TOML or (for `.json`)
/// JSON, in one step (see [`crate::save`]).
pub fn write_document(path: &Path, doc: &Value) -> Result<(), ToolError> {
    let text = document_text(path, doc)?;
    crate::save::write_atomic(path, text.as_bytes())
}

/// The document in `text` (TOML, or JSON for `format`), as written: no
/// defaults filled in. Syntax errors come back as a report with the line.
pub fn parse_document(text: &str, format: Format) -> Result<Value, ToolError> {
    let tree = match format {
        Format::Toml => duck_echo_core::tree::parse_toml(text),
        Format::Json => duck_echo_core::tree::parse_json(text),
    }
    .map_err(|issue| ToolError::Invalid(Report::from_error(*issue)))?;
    tree.to_json().ok_or_else(|| {
        ToolError::failed(
            Code::Invalid,
            "the file holds values JSON cannot represent (a date, or a number that is not finite)",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn set(path: &str, value: Value) -> EditOp {
        EditOp {
            path: path.into(),
            value: Some(value),
            remove: false,
        }
    }

    #[test]
    fn set_append_create_and_remove() {
        let mut doc = json!({"version": 0, "layers": [{"source": "sine"}]});
        apply_edits(
            &mut doc,
            &[
                set("layers[0].pitch.start", json!("C5")),
                set("layers[1]", json!({"source": "noise"})),
                set("effects[0]", json!({"type": "echo"})),
                set("master.gain", json!(0.5)),
                EditOp {
                    path: "layers[0].source".into(),
                    value: None,
                    remove: true,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            doc,
            json!({"version": 0, "master": {"gain": 0.5},
                   "layers": [{"pitch": {"start": "C5"}}, {"source": "noise"}],
                   "effects": [{"type": "echo"}]})
        );
    }

    #[test]
    fn bad_ops_name_the_op_and_path() {
        let mut doc = json!({"version": 0, "layers": []});
        let err = apply_edits(&mut doc, &[set("layers[3].volume", json!(1))]).unwrap_err();
        let ToolError::Invalid(r) = err else { panic!() };
        assert_eq!(r.errors[0].path, "layers[3].volume");
        assert!(
            r.errors[0].message.contains("past the end"),
            "{}",
            r.errors[0].message
        );
        for bad in ["", "a..b", "layers[x]", "layers[0", "[0]"] {
            assert!(
                apply_edits(&mut doc, &[set(bad, json!(1))]).is_err(),
                "{bad}"
            );
        }
        let both = EditOp {
            path: "seed".into(),
            value: Some(json!(1)),
            remove: true,
        };
        assert!(apply_edits(&mut doc, &[both]).is_err());
    }

    #[test]
    fn toml_document_reads_in_order_and_round_trips() {
        let doc = json!({"layers": [{"source": "sine", "pitch": {"start": "A4"}}],
                         "version": 0, "seed": 3, "tags": ["ui"],
                         "master": {"gain": 0.5}, "name": "x", "duration": null});
        let text = to_toml_document(&doc).unwrap();
        assert!(
            text.starts_with("version = 0\nname = \"x\"\ntags = [\"ui\"]\nseed = 3\n"),
            "{text}"
        );
        let i_master = text.find("[master]").unwrap();
        let i_layers = text.find("[[layers]]").unwrap();
        assert!(i_master < i_layers, "{text}");
        let back = duck_echo_core::tree::parse_toml(&text)
            .unwrap()
            .to_json()
            .unwrap();
        let mut want = doc.clone();
        want.as_object_mut().unwrap().remove("duration");
        assert_eq!(back, want);
    }

    #[test]
    fn a_changed_value_keeps_every_comment_and_the_layout() {
        let old = "# Coin: a blip.\nversion = 0\nname = \"coin\" # its name\n\n[master]\nnormalize = true\n\n# The one layer.\n[[layers]]\nsource = \"square\"\nenvelope = { attack = 0.002, decay = 0.25 } # short\npitch = { start = \"B5\", arpeggio = { steps = [0, 5] } }\n";
        let path = Path::new("coin.toml");
        let mut doc = parse_document(old, Format::Toml).unwrap();
        // One number inside an inline table.
        doc["layers"][0]["envelope"]["decay"] = json!(0.6);
        let new = document_text_from(path, old, &doc).unwrap();
        assert_eq!(new, old.replace("decay = 0.25", "decay = 0.6"));
        // A new field, a removed field, a new table and a new list item.
        doc["layers"][0]["volume"] = json!(0.5);
        doc["master"].as_object_mut().unwrap().remove("normalize");
        doc["master"]["gain"] = json!(2);
        doc["layers"][0]["filter"] = json!({"cutoff": 800});
        doc["layers"][0]["pitch"]["arpeggio"]["steps"] = json!([0, 5, 7]);
        doc["layers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"source": "noise"}));
        doc["effects"] = json!([{"type": "reverb"}]);
        let new = document_text_from(path, old, &doc).unwrap();
        assert_eq!(parse_document(&new, Format::Toml).unwrap(), doc);
        for kept in [
            "# Coin: a blip.",
            "# its name",
            "# The one layer.",
            "# short",
        ] {
            assert!(new.contains(kept), "{kept} went:\n{new}");
        }
        // A removed layer goes with its own lines; the rest stays.
        let mut doc = parse_document(old, Format::Toml).unwrap();
        doc["layers"] = json!([]);
        let new = document_text_from(path, old, &doc).unwrap();
        assert_eq!(parse_document(&new, Format::Toml).unwrap(), doc);
        assert!(new.starts_with("# Coin: a blip.\n"), "{new}");
        // Text that does not parse is written whole from the data.
        let doc = json!({"version": 0, "layers": [{"source": "sine"}]});
        let new = document_text_from(path, "this is [ not toml", &doc).unwrap();
        assert_eq!(new, to_toml_document(&doc).unwrap());
        // JSON is written whole.
        let json_path = Path::new("coin.json");
        assert_eq!(
            document_text_from(json_path, "{}", &doc).unwrap(),
            document_text(json_path, &doc).unwrap()
        );
    }
}
