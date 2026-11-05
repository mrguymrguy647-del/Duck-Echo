//! A small generic document tree with source positions.
//!
//! TOML and JSON are both parsed into [`Node`] so the structural checker in
//! [`crate::structure`] can walk either one and report every problem with a
//! line and column, instead of stopping at the first serde error.

use crate::report::{Code, Issue};

/// 1-based line and column.
pub type Pos = Option<(usize, usize)>;

/// A value with its source position.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// The value.
    pub value: Value,
    /// Where the value starts in the source.
    pub pos: Pos,
}

/// One key in a table, with the position of the key itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Key text.
    pub key: String,
    /// Where the key starts in the source.
    pub key_pos: Pos,
    /// The value.
    pub node: Node,
}

/// A generic document value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// JSON `null` (TOML has none).
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i128),
    /// Floating point number.
    Float(f64),
    /// String.
    Str(String),
    /// TOML date or time, kept as text.
    Datetime(String),
    /// Array.
    Array(Vec<Node>),
    /// Table or object, in source order.
    Table(Vec<Entry>),
}

impl Value {
    /// A short type name for messages, such as `string "loud"`.
    pub fn describe(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Bool(b) => format!("boolean `{b}`"),
            Value::Int(i) => format!("integer `{i}`"),
            Value::Float(f) => format!("number `{f}`"),
            Value::Str(s) => format!("string {s:?}"),
            Value::Datetime(d) => format!("datetime `{d}`"),
            Value::Array(_) => "an array".into(),
            Value::Table(_) => "a table".into(),
        }
    }

    /// The value as JSON, for the `got` field of an issue. Containers are
    /// summarized rather than copied.
    pub fn to_got(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => (*b).into(),
            Value::Int(i) => i64::try_from(*i)
                .map(Into::into)
                .unwrap_or_else(|_| i.to_string().into()),
            Value::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| f.to_string().into()),
            Value::Str(s) => s.clone().into(),
            Value::Datetime(d) => d.clone().into(),
            Value::Array(_) => "array".into(),
            Value::Table(_) => "table".into(),
        }
    }
}

impl Node {
    /// Convert to a `serde_json::Value` for typed deserialization. Returns
    /// `None` for values JSON cannot hold (non-finite floats, datetimes,
    /// integers beyond 64 bits); the checker rejects those first.
    pub fn to_json(&self) -> Option<serde_json::Value> {
        use serde_json::Value as J;
        Some(match &self.value {
            Value::Null => J::Null,
            Value::Bool(b) => J::Bool(*b),
            Value::Int(i) => {
                if let Ok(v) = i64::try_from(*i) {
                    J::from(v)
                } else {
                    J::from(u64::try_from(*i).ok()?)
                }
            }
            Value::Float(f) => J::Number(serde_json::Number::from_f64(*f)?),
            Value::Str(s) => J::String(s.clone()),
            Value::Datetime(_) => return None,
            Value::Array(items) => J::Array(
                items
                    .iter()
                    .map(Node::to_json)
                    .collect::<Option<Vec<_>>>()?,
            ),
            Value::Table(entries) => {
                let mut map = serde_json::Map::new();
                for e in entries {
                    map.insert(e.key.clone(), e.node.to_json()?);
                }
                J::Object(map)
            }
        })
    }
}

/// Position of the value at `path` (such as `layers[1].pitch.start`), or
/// of its nearest ancestor that exists in the document. For a table entry
/// the key's position is used, since that is the line a reader looks at.
/// Returns `None` when not even the first segment exists.
pub fn position_of(root: &Node, path: &str) -> Pos {
    let mut node = root;
    let mut best: Pos = None;
    for seg in path_segments(path) {
        match (seg, &node.value) {
            (Seg::Key(k), Value::Table(entries)) => match entries.iter().find(|e| e.key == k) {
                Some(e) => {
                    best = e.key_pos.or(e.node.pos);
                    node = &e.node;
                }
                None => break,
            },
            (Seg::Index(i), Value::Array(items)) => match items.get(i) {
                Some(item) => {
                    best = item.pos;
                    node = item;
                }
                None => break,
            },
            _ => break,
        }
    }
    best
}

enum Seg<'a> {
    Key(&'a str),
    Index(usize),
}

fn path_segments(path: &str) -> Vec<Seg<'_>> {
    let mut out = Vec::new();
    for part in path.split('.').filter(|p| !p.is_empty()) {
        let (key, rest) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (part, ""),
        };
        if !key.is_empty() {
            out.push(Seg::Key(key));
        }
        for idx in rest.split('[').filter(|s| !s.is_empty()) {
            if let Ok(i) = idx.trim_end_matches(']').parse() {
                out.push(Seg::Index(i));
            }
        }
    }
    out
}

/// 1-based line and column of a byte offset.
pub fn line_col(src: &str, byte_offset: usize) -> (usize, usize) {
    let mut offset = byte_offset.min(src.len());
    while !src.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &src[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before
        .rsplit('\n')
        .next()
        .map(|s| s.chars().count())
        .unwrap_or(0)
        + 1;
    (line, col)
}

// ---------------------------------------------------------------- TOML

/// Parse TOML into a tree. Syntax errors become a single `syntax` issue.
pub fn parse_toml(src: &str) -> Result<Node, Box<Issue>> {
    let table = toml::de::DeTable::parse(src).map_err(|e| {
        let mut issue = Issue::error(Code::Syntax, "", e.message().to_string());
        if let Some(span) = e.span() {
            let (l, c) = line_col(src, span.start);
            issue = issue.at(l, c);
        }
        Box::new(issue)
    })?;
    let span = table.span();
    Ok(Node {
        value: toml_table(src, table.into_inner()),
        pos: Some(line_col(src, span.start)),
    })
}

fn toml_table(src: &str, table: toml::de::DeTable<'_>) -> Value {
    let entries = table
        .into_iter()
        .map(|(k, v)| {
            let key_pos = Some(line_col(src, k.span().start));
            Entry {
                key: k.into_inner().into_owned(),
                key_pos,
                node: toml_node(src, v),
            }
        })
        .collect();
    Value::Table(entries)
}

fn toml_node(src: &str, v: toml::Spanned<toml::de::DeValue<'_>>) -> Node {
    use toml::de::DeValue;
    let pos = Some(line_col(src, v.span().start));
    let value = match v.into_inner() {
        DeValue::String(s) => Value::Str(s.into_owned()),
        DeValue::Integer(i) => {
            match i128::from_str_radix(i.as_str(), i.radix()) {
                Ok(n) => Value::Int(n),
                // Out of i128 range: keep the text so it fails as a type error.
                Err(_) => Value::Str(i.as_str().to_string()),
            }
        }
        DeValue::Float(f) => Value::Float(parse_toml_float(f.as_str())),
        DeValue::Boolean(b) => Value::Bool(b),
        DeValue::Datetime(d) => Value::Datetime(d.to_string()),
        DeValue::Array(items) => Value::Array(
            items
                .iter()
                .cloned()
                .map(|item| toml_node(src, item))
                .collect(),
        ),
        DeValue::Table(t) => Value::Table(match toml_table(src, t) {
            Value::Table(e) => e,
            _ => unreachable!(),
        }),
    };
    Node { value, pos }
}

fn parse_toml_float(text: &str) -> f64 {
    match text.trim_start_matches('+') {
        "inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        "nan" | "-nan" => f64::NAN,
        other => other.parse().unwrap_or(f64::NAN),
    }
}

// ---------------------------------------------------------------- JSON

/// Parse JSON into a tree. Syntax is checked by `serde_json` first so its
/// messages are used for syntax errors; the span parser below then only
/// ever sees valid JSON.
pub fn parse_json(src: &str) -> Result<Node, Box<Issue>> {
    if let Err(e) = serde_json::from_str::<serde::de::IgnoredAny>(src) {
        let msg = e.to_string();
        let msg = match msg.rfind(" at line ") {
            Some(i) => msg[..i].to_string(),
            None => msg,
        };
        return Err(Box::new(
            Issue::error(Code::Syntax, "", msg).at(e.line(), e.column()),
        ));
    }
    let mut p = JsonSpans { src, i: 0 };
    p.ws();
    Ok(p.value())
}

struct JsonSpans<'a> {
    src: &'a str,
    i: usize,
}

impl JsonSpans<'_> {
    fn bytes(&self) -> &[u8] {
        self.src.as_bytes()
    }

    fn peek(&self) -> u8 {
        self.bytes().get(self.i).copied().unwrap_or(0)
    }

    fn ws(&mut self) {
        while matches!(self.peek(), b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn pos(&self) -> Pos {
        Some(line_col(self.src, self.i))
    }

    fn value(&mut self) -> Node {
        let pos = self.pos();
        let value = match self.peek() {
            b'{' => {
                self.i += 1;
                let mut entries = Vec::new();
                self.ws();
                if self.peek() == b'}' {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        let key_pos = self.pos();
                        let key = self.string();
                        self.ws();
                        self.i += 1; // ':'
                        self.ws();
                        let node = self.value();
                        entries.push(Entry { key, key_pos, node });
                        self.ws();
                        let c = self.peek();
                        self.i += 1; // ',' or '}'
                        if c == b'}' {
                            break;
                        }
                    }
                }
                Value::Table(entries)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        items.push(self.value());
                        self.ws();
                        let c = self.peek();
                        self.i += 1; // ',' or ']'
                        if c == b']' {
                            break;
                        }
                    }
                }
                Value::Array(items)
            }
            b'"' => Value::Str(self.string()),
            b't' => {
                self.i += 4;
                Value::Bool(true)
            }
            b'f' => {
                self.i += 5;
                Value::Bool(false)
            }
            b'n' => {
                self.i += 4;
                Value::Null
            }
            _ => self.number(),
        };
        Node { value, pos }
    }

    fn string(&mut self) -> String {
        let start = self.i;
        self.i += 1;
        while self.peek() != b'"' {
            if self.peek() == b'\\' {
                self.i += 1;
            }
            self.i += 1;
        }
        self.i += 1;
        // Let serde_json decode escapes; the slice is a valid JSON string.
        serde_json::from_str(&self.src[start..self.i]).unwrap_or_default()
    }

    fn number(&mut self) -> Value {
        let start = self.i;
        while matches!(self.peek(), b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.i += 1;
        }
        let text = &self.src[start..self.i];
        if !text.contains(['.', 'e', 'E']) {
            if let Ok(n) = text.parse::<i128>() {
                return Value::Int(n);
            }
        }
        Value::Float(text.parse().unwrap_or(f64::NAN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_tree_has_positions_and_decodes_escapes() {
        let n = parse_json("{\n  \"a\": [1, 2.5, \"x\\ny\"],\n  \"b\": {\"c\": null}\n}").unwrap();
        let Value::Table(entries) = &n.value else {
            panic!()
        };
        assert_eq!(entries[0].key, "a");
        assert_eq!(entries[0].key_pos, Some((2, 3)));
        let Value::Array(items) = &entries[0].node.value else {
            panic!()
        };
        assert_eq!(items[0].value, Value::Int(1));
        assert_eq!(items[1].value, Value::Float(2.5));
        assert_eq!(items[2].value, Value::Str("x\ny".into()));
        assert_eq!(entries[1].key_pos, Some((3, 3)));
    }

    #[test]
    fn position_of_finds_values_and_nearest_ancestor() {
        let src = "version = 0\n[[layers]]\n[[layers]]\nvolume = 3\n[layers.pitch]\nstart = 9\n";
        let n = parse_toml(src).unwrap();
        assert_eq!(position_of(&n, "layers[1].volume"), Some((4, 1)));
        assert_eq!(position_of(&n, "layers[1].pitch.start"), Some((6, 1)));
        // `end` is absent: fall back to the pitch table.
        assert_eq!(position_of(&n, "layers[1].pitch.end").map(|p| p.0), Some(5));
        assert_eq!(position_of(&n, "effects[0]"), None);
    }

    #[test]
    fn toml_tree_has_key_positions() {
        let n = parse_toml("version = 0\n[[layers]]\nvolume = 0.5\n").unwrap();
        let Value::Table(entries) = &n.value else {
            panic!()
        };
        let layers = entries.iter().find(|e| e.key == "layers").unwrap();
        let Value::Array(items) = &layers.node.value else {
            panic!()
        };
        let Value::Table(fields) = &items[0].value else {
            panic!()
        };
        assert_eq!(fields[0].key, "volume");
        assert_eq!(fields[0].key_pos, Some((3, 1)));
        assert_eq!(fields[0].node.value, Value::Float(0.5));
    }
}
