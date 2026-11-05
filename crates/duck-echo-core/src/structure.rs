//! Structural checking: walk a parsed document against the generated JSON
//! Schema and report every unknown field, unknown enum value, missing
//! required field and type mismatch in one pass.
//!
//! serde stops at the first error, so on its own it would make an agent fix
//! one mistake per call. This walker collects all of them, then hands back a
//! pruned copy of the document with the bad parts removed (or replaced by a
//! neutral placeholder where something is required) so the typed decode and
//! the range checks in [`crate::validate`] can still run in the same call.
//!
//! The walker understands exactly the schema constructs `schemars` emits for
//! the recipe types: `$ref`, `type` (single or list), `properties`,
//! `additionalProperties: false`, `required`, `items`, `anyOf`, `oneOf` of
//! string constants (plain enums), `oneOf` of objects with a constant tag
//! property (internally tagged enums), `enum`, `const`, integer `format`s,
//! `minimum`/`maximum`, and the custom string `format: "note-name"`
//! (checked with [`crate::note::parse_note`]). A unit test fails if the
//! schema ever uses anything else.

use crate::load::{suggest, Suggestion};
use crate::report::{Code, Issue};
use crate::tree::{Node, Pos, Value};
use serde_json::{Map, Value as J};
use std::sync::OnceLock;

/// Result of a structural check.
#[derive(Debug, Clone)]
pub struct Checked {
    /// Every structural error, in document order.
    pub issues: Vec<Issue>,
    /// Paths whose contents were replaced by placeholders. Semantic issues
    /// at or under these paths are artifacts of the placeholder and must be
    /// dropped.
    pub poisoned: Vec<String>,
    /// The document with bad parts removed, ready for typed decoding.
    /// `None` only if the root itself is unusable.
    pub pruned: Option<J>,
    /// `(decoded path, written path)` for misspelled entries whose contents
    /// were decoded under the confidently suggested name, innermost first.
    /// Semantic issues under a decoded path belong at the written path.
    pub renames: Vec<(String, String)>,
}

/// The recipe schema, generated once.
fn recipe_schema() -> &'static J {
    static SCHEMA: OnceLock<J> = OnceLock::new();
    SCHEMA.get_or_init(crate::schema::recipe_schema)
}

/// Check a document against the recipe schema.
pub fn check(root: &Node) -> Checked {
    check_against(root, recipe_schema())
}

/// Check a document against an arbitrary schema of the supported subset.
pub fn check_against(root: &Node, schema: &J) -> Checked {
    let empty = Map::new();
    let defs = schema.get("$defs").and_then(J::as_object).unwrap_or(&empty);
    let mut w = Walker {
        root_props: schema
            .get("properties")
            .and_then(J::as_object)
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default(),
        defs,
        issues: Vec::new(),
        poisoned: Vec::new(),
        renames: Vec::new(),
    };
    let pruned = w.walk(schema, root, "");
    Checked {
        issues: w.issues,
        poisoned: w.poisoned,
        pruned,
        renames: w.renames,
    }
}

/// True when `path` is `root` or lies under it.
pub fn is_under(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return true;
    }
    path == root || (path.starts_with(root) && matches!(path.as_bytes()[root.len()], b'.' | b'['))
}

fn join(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn at(mut issue: Issue, pos: Pos) -> Issue {
    if let Some((l, c)) = pos {
        issue = issue.at(l, c);
    }
    issue
}

struct Walker<'s> {
    /// Property names of the document root, to spot top-level fields
    /// written inside a table.
    root_props: Vec<String>,
    defs: &'s Map<String, J>,
    issues: Vec<Issue>,
    poisoned: Vec<String>,
    renames: Vec<(String, String)>,
}

impl<'s> Walker<'s> {
    /// Follow `$ref` chains into `$defs`.
    fn resolve(&self, mut schema: &'s J) -> &'s J {
        for _ in 0..32 {
            match schema.get("$ref").and_then(J::as_str) {
                Some(r) => match r.strip_prefix("#/$defs/").and_then(|n| self.defs.get(n)) {
                    Some(target) => schema = target,
                    None => return schema,
                },
                None => return schema,
            }
        }
        schema
    }

    /// Validate `node` against `schema`. Returns the pruned JSON value, or
    /// `None` when the value must be dropped (an issue was recorded).
    fn walk(&mut self, schema: &'s J, node: &Node, path: &str) -> Option<J> {
        let s = self.resolve(schema);

        if let Some(branches) = s.get("anyOf").and_then(J::as_array) {
            for b in branches {
                if self.accepts_kind(b, &node.value) {
                    return self.walk(b, node, path);
                }
            }
            let expected = branches
                .iter()
                .map(|b| self.expected(b))
                .collect::<Vec<_>>()
                .join(" or ");
            self.type_mismatch(path, node, &expected);
            return None;
        }

        if let Some(branches) = s.get("oneOf").and_then(J::as_array) {
            if let Some(options) = const_options(branches) {
                return self.walk_enum(&options, node, path);
            }
            if let Some(tag) = self.tag_key(branches) {
                return self.walk_tagged(branches, &tag, node, path);
            }
        }

        if let Some(options) = s.get("enum").and_then(J::as_array) {
            let options: Vec<String> = options
                .iter()
                .filter_map(|o| o.as_str().map(str::to_string))
                .collect();
            return self.walk_enum(&options, node, path);
        }

        if let Some(c) = s.get("const") {
            let got = node.to_json();
            if got.as_ref() != Some(c) {
                let opt = c
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| c.to_string());
                self.issues.push(at(
                    Issue::error(Code::UnknownVariant, path, format!("expected `{opt}`"))
                        .got(node.value.to_got())
                        .one_of([opt]),
                    node.pos,
                ));
                return None;
            }
            return got;
        }

        let types = type_list(s);
        if types.is_empty() {
            // No constraint we understand: accept as-is.
            return node.to_json();
        }
        if !types.iter().any(|t| kind_matches(t, &node.value)) {
            let expected = self.expected(s);
            self.type_mismatch(path, node, &expected);
            return None;
        }
        match &node.value {
            Value::Null => Some(J::Null),
            Value::Table(_) => self.walk_object(s, node, path),
            Value::Array(items) => {
                let item_schema = s.get("items");
                let mut out = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    let p = format!("{path}[{i}]");
                    let v = match item_schema {
                        Some(is) => match self.walk(is, item, &p) {
                            Some(v) => v,
                            None => {
                                self.poisoned.push(p);
                                self.placeholder(is)
                            }
                        },
                        None => item.to_json().unwrap_or(J::Null),
                    };
                    out.push(v);
                }
                Some(J::Array(out))
            }
            Value::Int(i) if types.iter().any(|t| t == "integer") => {
                let (min, max) = int_bounds(s);
                if *i < min || *i > max {
                    self.issues.push(at(
                        Issue::error(
                            Code::OutOfRange,
                            path,
                            format!("value {i} is outside the allowed range {min}..={max}"),
                        )
                        .got(node.value.to_got())
                        .range(min as f64, max as f64),
                        node.pos,
                    ));
                    return None;
                }
                node.to_json()
            }
            Value::Float(f) if !f.is_finite() => {
                self.issues.push(at(
                    Issue::error(
                        Code::OutOfRange,
                        path,
                        format!("value {f} is not a finite number"),
                    )
                    .got(node.value.to_got()),
                    node.pos,
                ));
                None
            }
            Value::Str(text)
                if s.get("format").and_then(J::as_str) == Some(crate::note::NOTE_FORMAT) =>
            {
                match crate::note::parse_note(text) {
                    Ok(_) => node.to_json(),
                    Err(e) => {
                        let mut issue =
                            Issue::error(Code::InvalidFormat, path, e.message).got(text.clone());
                        if let Some(sugg) = e.suggestion {
                            issue = issue.suggest(sugg);
                        }
                        self.issues.push(at(issue, node.pos));
                        None
                    }
                }
            }
            _ => node.to_json(),
        }
    }

    fn walk_object(&mut self, s: &'s J, node: &Node, path: &str) -> Option<J> {
        let Value::Table(entries) = &node.value else {
            return None;
        };
        static EMPTY: OnceLock<Map<String, J>> = OnceLock::new();
        let props: &'s Map<String, J> = match s.get("properties").and_then(J::as_object) {
            Some(p) => p,
            None => EMPTY.get_or_init(Map::new),
        };
        let closed = s.get("additionalProperties") == Some(&J::Bool(false));
        let required: Vec<&str> = s
            .get("required")
            .and_then(J::as_array)
            .map(|r| r.iter().filter_map(J::as_str).collect())
            .unwrap_or_default();

        let mut out = Map::new();
        let mut seen: Vec<&str> = Vec::new();
        // Required fields supplied through a misspelled key.
        let mut filled: Vec<String> = Vec::new();
        for e in entries {
            let p = join(path, &e.key);
            if seen.contains(&e.key.as_str()) {
                self.issues.push(at(
                    Issue::error(Code::Other, p, format!("duplicate field `{}`", e.key)),
                    e.key_pos,
                ));
                continue;
            }
            seen.push(&e.key);
            match props.get(&e.key) {
                Some(ps) => match self.walk(ps, &e.node, &p) {
                    Some(v) => {
                        out.insert(e.key.clone(), v);
                    }
                    None => {
                        if required.contains(&e.key.as_str()) {
                            out.insert(e.key.clone(), self.placeholder(ps));
                        }
                        self.poisoned.push(p);
                    }
                },
                None if closed => {
                    let options: Vec<String> = props
                        .iter()
                        .filter(|(_, v)| self.resolve(v).get("const").is_none())
                        .map(|(k, _)| k.clone())
                        .collect();
                    let sugg = suggest(&e.key, &options);
                    let mut message = format!("unknown field `{}`", e.key);
                    if !path.is_empty() && self.root_props.contains(&e.key) {
                        message.push_str(&format!(
                            "; `{}` is a top-level field (in TOML, write it above the first [table] header, or every key after a header belongs to that table)",
                            e.key
                        ));
                    }
                    let mut issue =
                        Issue::error(Code::UnknownField, &p, message).got(e.key.clone());
                    if let Some(sg) = &sugg {
                        issue = issue.suggest(sg.name.clone());
                    }
                    issue = issue.one_of(options);
                    self.issues.push(at(issue, e.key_pos));
                    if let Some(sg) = sugg {
                        let present = entries.iter().any(|x| x.key == sg.name);
                        let target = join(path, &sg.name);
                        match props.get(&sg.name) {
                            Some(ps) if sg.confident => {
                                // Check the contents as the intended field,
                                // reporting at the name actually written.
                                let checked = self.walk(ps, &e.node, &p);
                                if !present && !out.contains_key(&sg.name) {
                                    match checked {
                                        Some(v) => {
                                            out.insert(sg.name.clone(), v);
                                            filled.push(sg.name.clone());
                                            self.renames.push((target, p.clone()));
                                        }
                                        None => self.poisoned.push(target),
                                    }
                                }
                            }
                            // The intended field sits at its default;
                            // complaints about that default are noise.
                            _ if !present => self.poisoned.push(target),
                            _ => {}
                        }
                    }
                }
                None => {
                    if let Some(v) = e.node.to_json() {
                        out.insert(e.key.clone(), v);
                    }
                }
            }
        }
        for r in required {
            if !seen.contains(&r) && !filled.iter().any(|f| f == r) {
                let p = join(path, r);
                let ps = props.get(r);
                let mut issue = Issue::error(
                    Code::MissingField,
                    &p,
                    format!("missing required field `{r}`"),
                );
                if let Some(opts) = ps.and_then(|ps| self.const_or_enum_options(ps)) {
                    issue = issue.one_of(opts);
                }
                self.issues.push(at(issue, node.pos));
                if let Some(ps) = ps {
                    out.insert(r.to_string(), self.placeholder(ps));
                }
                self.poisoned.push(p);
            }
        }
        Some(J::Object(out))
    }

    fn walk_enum(&mut self, options: &[String], node: &Node, path: &str) -> Option<J> {
        let Value::Str(s) = &node.value else {
            self.type_mismatch(path, node, &format!("one of: {}", options.join(", ")));
            return None;
        };
        if options.iter().any(|o| o == s) {
            return Some(J::String(s.clone()));
        }
        let sugg = suggest(s, options);
        let mut issue =
            Issue::error(Code::UnknownVariant, path, format!("unknown value `{s}`")).got(s.clone());
        if let Some(Suggestion { name, confident }) = &sugg {
            issue = issue.suggest(name.clone());
            if *confident {
                // Carry on as if the intended value was written, so checks
                // that depend on it (such as `duty` on `square`) still run.
                self.issues
                    .push(at(issue.one_of(options.iter().cloned()), node.pos));
                self.poisoned.push(path.to_string());
                return Some(J::String(name.clone()));
            }
        }
        self.issues
            .push(at(issue.one_of(options.iter().cloned()), node.pos));
        None
    }

    fn walk_tagged(&mut self, branches: &'s [J], tag: &str, node: &Node, path: &str) -> Option<J> {
        let Value::Table(entries) = &node.value else {
            self.type_mismatch(path, node, "a table");
            return None;
        };
        let options: Vec<String> = branches
            .iter()
            .filter_map(|b| tag_const(self.resolve(b), tag))
            .collect();
        let tag_path = join(path, tag);
        let Some(entry) = entries.iter().find(|e| e.key == tag) else {
            self.issues.push(at(
                Issue::error(
                    Code::MissingField,
                    tag_path,
                    format!("missing required field `{tag}`; it selects the kind of entry"),
                )
                .one_of(options),
                node.pos,
            ));
            return None;
        };
        let Value::Str(name) = &entry.node.value else {
            self.type_mismatch(
                &tag_path,
                &entry.node,
                &format!("one of: {}", options.join(", ")),
            );
            return None;
        };
        for b in branches {
            if tag_const(self.resolve(b), tag).as_deref() == Some(name.as_str()) {
                return self.walk(b, node, path);
            }
        }
        let sugg = suggest(name, &options);
        let mut issue = Issue::error(
            Code::UnknownVariant,
            &tag_path,
            format!("unknown value `{name}`"),
        )
        .got(name.clone());
        if let Some(sg) = &sugg {
            issue = issue.suggest(sg.name.clone());
        }
        self.issues.push(at(issue.one_of(options), entry.node.pos));
        let sg = sugg.filter(|sg| sg.confident)?;
        // Check the rest of the entry as the intended kind, so typos in its
        // fields are reported in the same call.
        let branch = branches
            .iter()
            .find(|b| tag_const(self.resolve(b), tag).as_deref() == Some(sg.name.as_str()))?;
        let mut fixed = node.clone();
        if let Value::Table(entries) = &mut fixed.value {
            for e in entries.iter_mut().filter(|e| e.key == tag) {
                e.node.value = Value::Str(sg.name.clone());
            }
        }
        self.poisoned.push(tag_path);
        self.walk(branch, &fixed, path)
    }

    fn type_mismatch(&mut self, path: &str, node: &Node, expected: &str) {
        let mut message = format!("expected {expected}, got {}", node.value.describe());
        let mut suggestion = None;
        if let Value::Str(text) = &node.value {
            if expected.contains("a number") || expected.contains("an integer") {
                if let Some(number) = crate::note::quoted_number(text) {
                    message.push_str("; write the number without quotes");
                    suggestion = Some(number.to_string());
                }
            }
        }
        let mut issue = Issue::error(Code::TypeMismatch, path, message).got(node.value.to_got());
        if let Some(s) = suggestion {
            issue = issue.suggest(s);
        }
        self.issues.push(at(issue, node.pos));
    }

    /// The property name every branch fixes with a `const`.
    fn tag_key(&self, branches: &'s [J]) -> Option<String> {
        let first = self.resolve(branches.first()?);
        let props = first.get("properties")?.as_object()?;
        props
            .iter()
            .find(|(k, v)| {
                v.get("const").is_some()
                    && branches
                        .iter()
                        .all(|b| tag_const(self.resolve(b), k).is_some())
            })
            .map(|(k, _)| k.clone())
    }

    fn const_or_enum_options(&self, s: &'s J) -> Option<Vec<String>> {
        let s = self.resolve(s);
        if let Some(branches) = s.get("oneOf").and_then(J::as_array) {
            return const_options(branches);
        }
        s.get("enum").and_then(J::as_array).map(|o| {
            o.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
    }

    /// Whether a value's kind fits a schema at the top level (used to pick
    /// an `anyOf` branch before descending).
    fn accepts_kind(&self, schema: &'s J, v: &Value) -> bool {
        let s = self.resolve(schema);
        if let Some(branches) = s.get("anyOf").and_then(J::as_array) {
            return branches.iter().any(|b| self.accepts_kind(b, v));
        }
        if let Some(branches) = s.get("oneOf").and_then(J::as_array) {
            return if const_options(branches).is_some() {
                matches!(v, Value::Str(_))
            } else {
                matches!(v, Value::Table(_))
            };
        }
        if s.get("enum").is_some() {
            return matches!(v, Value::Str(_));
        }
        if s.get("const").is_some() {
            return true;
        }
        let types = type_list(s);
        types.is_empty() || types.iter().any(|t| kind_matches(t, v))
    }

    /// Human description of what a schema accepts.
    fn expected(&self, schema: &'s J) -> String {
        let s = self.resolve(schema);
        if let Some(branches) = s.get("anyOf").and_then(J::as_array) {
            return branches
                .iter()
                .filter(|b| type_list(self.resolve(b)) != ["null"])
                .map(|b| self.expected(b))
                .collect::<Vec<_>>()
                .join(" or ");
        }
        if let Some(opts) = self.const_or_enum_options(s) {
            return format!("one of: {}", opts.join(", "));
        }
        if s.get("oneOf").is_some() {
            return "a table".into();
        }
        let names: Vec<&str> = type_list(s)
            .iter()
            .filter(|t| *t != "null")
            .map(|t| match t.as_str() {
                "integer" => "an integer",
                "number" => "a number",
                "string"
                    if s.get("format").and_then(J::as_str) == Some(crate::note::NOTE_FORMAT) =>
                {
                    "a note name such as \"C5\""
                }
                "string" => "a string",
                "boolean" => "true or false",
                "array" => "an array",
                "object" => "a table",
                _ => "a value",
            })
            .collect();
        if names.is_empty() {
            "a value".into()
        } else {
            names.join(" or ")
        }
    }

    /// A minimal value that deserializes for `schema`, used in place of a
    /// broken required value so decoding can continue.
    fn placeholder(&self, schema: &'s J) -> J {
        let s = self.resolve(schema);
        if let Some(branches) = s.get("anyOf").and_then(J::as_array) {
            return branches
                .iter()
                .find(|b| type_list(self.resolve(b)).iter().any(|t| t != "null"))
                .map(|b| self.placeholder(b))
                .unwrap_or(J::Null);
        }
        if let Some(branches) = s.get("oneOf").and_then(J::as_array) {
            if let Some(opts) = const_options(branches) {
                return J::String(opts.into_iter().next().unwrap_or_default());
            }
            if let (Some(tag), Some(first)) = (self.tag_key(branches), branches.first()) {
                let mut obj = Map::new();
                if let Some(c) = tag_const(self.resolve(first), &tag) {
                    obj.insert(tag, J::String(c));
                }
                return J::Object(obj);
            }
        }
        if let Some(c) = s.get("const") {
            return c.clone();
        }
        let types = type_list(s);
        match types.first().map(String::as_str) {
            Some("object") => {
                let mut obj = Map::new();
                if let (Some(req), Some(props)) = (
                    s.get("required").and_then(J::as_array),
                    s.get("properties").and_then(J::as_object),
                ) {
                    for r in req.iter().filter_map(J::as_str) {
                        if let Some(ps) = props.get(r) {
                            obj.insert(r.to_string(), self.placeholder(ps));
                        }
                    }
                }
                J::Object(obj)
            }
            Some("array") => J::Array(Vec::new()),
            Some("integer") => J::from(int_bounds(s).0.max(0) as i64),
            Some("number") => J::from(0.0),
            Some("string") => J::String(String::new()),
            Some("boolean") => J::Bool(false),
            _ => J::Null,
        }
    }
}

fn type_list(s: &J) -> Vec<String> {
    match s.get("type") {
        Some(J::String(t)) => vec![t.clone()],
        Some(J::Array(ts)) => ts
            .iter()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn kind_matches(t: &str, v: &Value) -> bool {
    matches!(
        (t, v),
        ("null", Value::Null)
            | ("boolean", Value::Bool(_))
            | ("integer", Value::Int(_))
            | ("number", Value::Int(_) | Value::Float(_))
            | ("string", Value::Str(_))
            | ("array", Value::Array(_))
            | ("object", Value::Table(_))
    )
}

/// Options of a `oneOf` made only of string constants.
fn const_options(branches: &[J]) -> Option<Vec<String>> {
    branches
        .iter()
        .map(|b| b.get("const").and_then(J::as_str).map(str::to_string))
        .collect()
}

fn tag_const(branch: &J, tag: &str) -> Option<String> {
    branch
        .get("properties")?
        .get(tag)?
        .get("const")?
        .as_str()
        .map(str::to_string)
}

fn int_bounds(s: &J) -> (i128, i128) {
    let (mut min, mut max) = match s.get("format").and_then(J::as_str) {
        Some("uint8") => (0, u8::MAX as i128),
        Some("uint16") => (0, u16::MAX as i128),
        Some("uint32") => (0, u32::MAX as i128),
        Some("uint64") | Some("uint") => (0, u64::MAX as i128),
        Some("int8") => (i8::MIN as i128, i8::MAX as i128),
        Some("int16") => (i16::MIN as i128, i16::MAX as i128),
        Some("int32") => (i32::MIN as i128, i32::MAX as i128),
        _ => (i64::MIN as i128, i64::MAX as i128),
    };
    if let Some(m) = s.get("minimum").and_then(J::as_f64) {
        min = min.max(m.ceil() as i128);
    }
    if let Some(m) = s.get("maximum").and_then(J::as_f64) {
        max = max.min(m.floor() as i128);
    }
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every keyword in the generated schema must be one the walker handles
    /// (or pure documentation). Fails loudly if a type change makes
    /// schemars emit something new.
    #[test]
    fn walker_supports_every_schema_keyword() {
        const SUPPORTED: &[&str] = &[
            "$ref",
            "$defs",
            "$schema",
            "$id",
            "title",
            "description",
            "default",
            "examples",
            "type",
            "properties",
            "additionalProperties",
            "required",
            "items",
            "anyOf",
            "oneOf",
            "enum",
            "const",
            "format",
            "minimum",
            "maximum",
            "pattern",
        ];
        fn visit(v: &J, under_props: bool, bad: &mut Vec<String>) {
            match v {
                J::Object(map) => {
                    for (k, child) in map {
                        if !under_props && !SUPPORTED.contains(&k.as_str()) {
                            bad.push(k.clone());
                        }
                        if k == "default" || k == "examples" || k == "const" || k == "enum" {
                            continue;
                        }
                        visit(child, k == "properties" || k == "$defs", bad);
                    }
                }
                J::Array(items) => items.iter().for_each(|i| visit(i, false, bad)),
                _ => {}
            }
        }
        let mut bad = Vec::new();
        visit(recipe_schema(), false, &mut bad);
        assert!(bad.is_empty(), "unsupported schema keywords: {bad:?}");
    }

    #[test]
    fn is_under_respects_segment_boundaries() {
        assert!(is_under("layers[0].pitch", "layers[0]"));
        assert!(is_under("layers[0]", "layers[0]"));
        assert!(!is_under("layers[01]", "layers[0]"));
        assert!(!is_under("layersx", "layers"));
        assert!(is_under("anything", ""));
    }
}
