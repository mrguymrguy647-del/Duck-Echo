//! The sliders of the sound editor, made from the recipe's schema.
//!
//! Nothing here knows what a recipe holds. It walks the JSON Schema that
//! `duck_echo_core::schema::recipe_schema` writes (the same document agents
//! read) next to the recipe as written, and turns each field into a
//! control:
//!
//! * a number with a `minimum` and `maximum` becomes a slider with that
//!   range, so a slider can never reach a value validation refuses;
//! * a field with fixed choices (`oneOf` of `const`s) becomes a drop-down;
//! * a pitch (`Frequency`: a number of Hz or a note name) becomes a slider
//!   in Hz with a note picker beside it;
//! * a yes-or-no field becomes a tick box;
//! * a table that may be left out (`filter`, `lfo`) gets an "Add" button,
//!   and a remove button once it is there;
//! * the lists of layers and effects get their own add and remove buttons.
//!
//! So a field added to the recipe types appears here by itself.
//!
//! The panel changes nothing. It returns what the person did as a list of
//! [`Edit`]s, and the editor turns each into a command.

use duck_echo_core::note;
use eframe::egui::{self, RichText};
use serde_json::{Map, Value, json};

use crate::theme;

/// Something the person did in the panel.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// A field changed to `value`. `held` is true while a slider is
    /// still being dragged (more values follow, then [`Edit::Release`]).
    Set {
        /// The field, as in error messages: `layers[0].envelope.decay`.
        path: String,
        /// The new value.
        value: Value,
        /// True while the mouse still holds the slider.
        held: bool,
    },
    /// The slider being dragged was let go.
    Release,
    /// A field (or a whole table) was taken out.
    Remove {
        /// The field, as in error messages.
        path: String,
    },
    /// "+ Add a layer".
    AddLayer,
    /// The remove button of a layer.
    RemoveLayer(usize),
    /// "+ Add effect": `kind` is `echo`, `reverb`, `distortion` or
    /// `bitcrush`; `layer` is `None` for the master effects.
    AddEffect {
        /// The layer, or `None` for the master effects.
        layer: Option<usize>,
        /// The effect's `type`.
        kind: String,
    },
    /// The remove button of an effect.
    RemoveEffect {
        /// The layer, or `None` for the master effects.
        layer: Option<usize>,
        /// The effect's place in its list.
        index: usize,
    },
}

/// Fields that are words, not settings: they are edited in the text.
const WORDS: &[&str] = &["name", "description", "tags", "version"];

/// What a new optional table starts as, when the schema's own defaults
/// alone would not make a sound that works.
fn starting_value(field: &str) -> Value {
    match field {
        "arpeggio" => json!({"steps": [0, 12], "step_time": 0.08}),
        "vibrato" => json!({"rate": 6.0, "depth": 0.3}),
        _ => json!({}),
    }
}

/// Draw the whole panel for `doc` (the recipe as written) with `schema`.
pub fn panel(ui: &mut egui::Ui, schema: &Value, doc: &Value) -> Vec<Edit> {
    let mut edits = Vec::new();
    let walker = Walker { schema };
    let empty = Map::new();
    let top = doc.as_object().unwrap_or(&empty);
    let props = schema["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();

    // The layers first: they are what most changes touch.
    let layers = top
        .get("layers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let layer_schema = walker.resolve(&props["layers"]["items"]);
    for (i, layer) in layers.iter().enumerate() {
        if i > 0 {
            ui.add_space(8.0);
        }
        ui.push_id(("layer", i), |ui| {
            ui.horizontal(|ui| {
                let source = layer["source"].as_str().unwrap_or("square");
                heading(ui, &format!("LAYER {}  {source}", i + 1));
                if layers.len() > 1
                    && ui
                        .small_button("x")
                        .on_hover_text(format!("Remove layer {}", i + 1))
                        .clicked()
                {
                    edits.push(Edit::RemoveLayer(i));
                }
            });
            let path = format!("layers[{i}]");
            walker.object_fields(ui, &path, &layer_schema, layer, &mut edits);
            walker.effects(ui, Some(i), &path, layer.get("effects"), &mut edits);
        });
    }
    ui.add_space(8.0);
    if ui.button("+ Add a layer").clicked() {
        edits.push(Edit::AddLayer);
    }

    ui.add_space(8.0);
    heading(ui, "MASTER");
    let master = top.get("master").cloned().unwrap_or_else(|| json!({}));
    walker.object_fields(
        ui,
        "master",
        &walker.resolve(&props["master"]),
        &master,
        &mut edits,
    );
    ui.add_space(8.0);
    heading(ui, "MASTER EFFECTS");
    walker.effects(ui, None, "", top.get("effects"), &mut edits);

    ui.add_space(8.0);
    heading(ui, "SOUND");
    for (field, spec) in &props {
        if WORDS.contains(&field.as_str())
            || matches!(field.as_str(), "master" | "layers" | "effects")
        {
            continue;
        }
        walker.field(ui, field, field, spec, top.get(field), &mut edits);
    }
    edits
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .strong()
            .small()
            .color(theme::NIGHT_INDIGO_TEXT),
    );
}

/// "attack", "sustain time": a field's name for people.
fn label_of(field: &str) -> String {
    field.replace('_', " ")
}

/// The first sentence of a schema description, for a tooltip.
fn hint(spec: &Value) -> Option<String> {
    let text = spec["description"].as_str()?;
    let one_line = text.replace('\n', " ");
    let first = one_line.split(". ").next().unwrap_or(&one_line);
    Some(first.trim_end_matches('.').to_string() + ".")
}

/// Join a path and a field: `layers[0]` and `volume` make
/// `layers[0].volume`; at the top a path is just the field.
fn join(path: &str, field: &str) -> String {
    if path.is_empty() {
        field.to_string()
    } else {
        format!("{path}.{field}")
    }
}

/// Round a slider's value to what a person would write: four significant
/// figures at most, so `0.2534` and `1235`, never `0.25341987`.
pub fn tidy(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return 0.0;
    }
    let digits = (3 - v.abs().log10().floor() as i32).clamp(0, 4);
    let scale = 10f64.powi(digits);
    (v * scale).round() / scale
}

/// A number as JSON: a whole number for an integer field, else a float.
fn number(v: f64, integer: bool) -> Value {
    if integer {
        json!(v.round() as i64)
    } else {
        json!(tidy(v))
    }
}

/// What a schema says one field may hold.
enum Shape {
    /// A number in a range (`integer`: whole numbers only).
    Number { min: f64, max: f64, integer: bool },
    /// A whole number with no top (a seed).
    Count,
    /// A pitch: Hz or a note name.
    Pitch,
    /// One of these words.
    Choice(Vec<String>),
    /// Yes or no.
    Flag,
    /// A table of fields.
    Table(Value),
    /// Something edited only in the text (lists of numbers, words).
    TextOnly,
}

struct Walker<'s> {
    schema: &'s Value,
}

impl Walker<'_> {
    /// Follow a `$ref` (`#/$defs/Layer`) to its definition.
    fn resolve(&self, spec: &Value) -> Value {
        match spec["$ref"].as_str() {
            Some(r) => {
                let name = r.rsplit('/').next().unwrap_or_default();
                let mut def = self.schema["$defs"][name].clone();
                // Keep the field's own description and default.
                for key in ["description", "default"] {
                    if let (Some(v), Some(obj)) = (spec.get(key), def.as_object_mut()) {
                        obj.insert(key.into(), v.clone());
                    }
                }
                def
            }
            None => spec.clone(),
        }
    }

    /// The field's shape, and whether it may be left out.
    fn shape(&self, spec: &Value) -> (Shape, bool) {
        let spec = self.resolve(spec);
        // `anyOf: [X, null]`: an optional X.
        if let Some(any) = spec["anyOf"].as_array() {
            let real: Vec<&Value> = any.iter().filter(|s| s["type"] != "null").collect();
            let optional = real.len() < any.len();
            if real.len() == 1 {
                let (shape, inner_optional) = self.shape(real[0]);
                return (shape, optional || inner_optional);
            }
            // A number or a note name: a pitch.
            // (A number may also be written as a random range, so its
            // `type` can be `["number", "object"]`.)
            let takes_number = |s: &&Value| match &s["type"] {
                Value::Array(types) => types.iter().any(|t| t == "number"),
                t => t == "number",
            };
            if real.iter().any(takes_number) && real.iter().any(|s| s["format"] == "note-name") {
                return (Shape::Pitch, optional);
            }
            return (Shape::TextOnly, optional);
        }
        if let Some(one) = spec["oneOf"].as_array() {
            let words: Vec<String> = one
                .iter()
                .filter_map(|s| s["const"].as_str().map(str::to_string))
                .collect();
            if words.len() == one.len() {
                return (Shape::Choice(words), false);
            }
            return (Shape::TextOnly, false);
        }
        let (kind, optional) = match &spec["type"] {
            Value::String(t) => (t.clone(), false),
            Value::Array(types) => (
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .find(|t| *t != "null")
                    .unwrap_or("null")
                    .to_string(),
                types.iter().any(|t| t == "null"),
            ),
            _ => (String::new(), false),
        };
        let shape = match kind.as_str() {
            "number" | "integer" => {
                let integer = kind == "integer";
                let min = spec["minimum"]
                    .as_f64()
                    .or(spec["exclusiveMinimum"].as_f64());
                match (min, spec["maximum"].as_f64()) {
                    (Some(min), Some(max)) => Shape::Number { min, max, integer },
                    _ if integer => Shape::Count,
                    _ => Shape::TextOnly,
                }
            }
            "boolean" => Shape::Flag,
            "object" if spec["properties"].is_object() => Shape::Table(spec.clone()),
            _ => Shape::TextOnly,
        };
        (shape, optional)
    }

    /// Every field of the table `spec`, whose value is `value`.
    fn object_fields(
        &self,
        ui: &mut egui::Ui,
        path: &str,
        spec: &Value,
        value: &Value,
        edits: &mut Vec<Edit>,
    ) {
        let Some(props) = spec["properties"].as_object() else {
            return;
        };
        let empty = Map::new();
        let values = value.as_object().unwrap_or(&empty);
        // Choices first (the wave, the kind of filter), then numbers and
        // tick boxes, then the tables inside.
        let mut fields: Vec<(&String, &Value)> = props
            .iter()
            .filter(|(f, _)| !WORDS.contains(&f.as_str()) && f.as_str() != "effects")
            .filter(|(_, s)| applies(s, props, values))
            .collect();
        fields.sort_by_key(|(_, s)| match self.shape(s).0 {
            Shape::Choice(_) => 0,
            Shape::Table(_) => 2,
            _ => 1,
        });
        for (field, spec) in fields {
            self.field(
                ui,
                &join(path, field),
                field,
                spec,
                values.get(field),
                edits,
            );
        }
    }

    /// One field.
    fn field(
        &self,
        ui: &mut egui::Ui,
        path: &str,
        field: &str,
        spec: &Value,
        value: Option<&Value>,
        edits: &mut Vec<Edit>,
    ) {
        let (shape, optional) = self.shape(spec);
        let spec = self.resolve(spec);
        let default = spec.get("default").filter(|d| !d.is_null());
        let value = value.filter(|v| !v.is_null());
        match shape {
            Shape::TextOnly => {}
            Shape::Table(table) => {
                ui.add_space(2.0);
                match value {
                    Some(v) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label_of(field).to_uppercase()).small().weak());
                            if optional
                                && ui
                                    .small_button("x")
                                    .on_hover_text(format!("Take out the {}", label_of(field)))
                                    .clicked()
                            {
                                edits.push(Edit::Remove { path: path.into() });
                            }
                        });
                        ui.indent(path, |ui| self.object_fields(ui, path, &table, v, edits));
                    }
                    None if optional => {
                        let add = ui
                            .small_button(format!("+ {}", label_of(field)))
                            .on_hover_text(hint(&spec).unwrap_or_default());
                        if add.clicked() {
                            edits.push(Edit::Set {
                                path: path.into(),
                                value: starting_value(field),
                                held: false,
                            });
                        }
                    }
                    None => {
                        // A table with defaults (envelope, pitch): show the
                        // defaults; moving one writes the table.
                        let shown = default.cloned().unwrap_or_else(|| json!({}));
                        ui.label(RichText::new(label_of(field).to_uppercase()).small().weak());
                        ui.indent(path, |ui| {
                            self.object_fields(ui, path, &table, &shown, edits)
                        });
                    }
                }
            }
            shape => {
                ui.horizontal(|ui| {
                    // The name first, so every control can be labelled by
                    // it for screen readers.
                    let name =
                        ui.add_sized([120.0, 18.0], egui::Label::new(label_of(field)).truncate());
                    let name_id = name.id;
                    if let Some(h) = hint(&spec) {
                        name.on_hover_text(h);
                    }
                    let mut on = value.is_some();
                    if optional {
                        let tick = ui
                            .checkbox(&mut on, "")
                            .labelled_by(name_id)
                            .on_hover_text(if on { "Take it out" } else { "Put it in" });
                        if tick.changed() {
                            if on {
                                let start = start_of(&shape, default);
                                edits.push(Edit::Set {
                                    path: path.into(),
                                    value: start,
                                    held: false,
                                });
                            } else {
                                edits.push(Edit::Remove { path: path.into() });
                            }
                        }
                    }
                    let current = value.or(default);
                    ui.add_enabled_ui(!optional || on, |ui| {
                        control(ui, path, name_id, &shape, current, edits);
                    });
                });
            }
        }
    }

    /// A list of effects, with add and remove buttons.
    fn effects(
        &self,
        ui: &mut egui::Ui,
        layer: Option<usize>,
        path: &str,
        list: Option<&Value>,
        edits: &mut Vec<Edit>,
    ) {
        let effects = list.and_then(Value::as_array).cloned().unwrap_or_default();
        let effect_schema = &self.schema["$defs"]["Effect"]["oneOf"];
        for (i, effect) in effects.iter().enumerate() {
            let kind = effect["type"].as_str().unwrap_or("?");
            let shape = effect_schema
                .as_array()
                .into_iter()
                .flatten()
                .find(|s| s["properties"]["type"]["const"] == kind);
            ui.push_id(("effect", i), |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("EFFECT {}  {kind}", i + 1))
                            .small()
                            .weak(),
                    );
                    if ui
                        .small_button("x")
                        .on_hover_text(format!("Remove the {kind}"))
                        .clicked()
                    {
                        edits.push(Edit::RemoveEffect { layer, index: i });
                    }
                });
                if let Some(shape) = shape {
                    let mut spec = shape.clone();
                    if let Some(p) = spec["properties"].as_object_mut() {
                        p.remove("type");
                    }
                    let effect_path = join(path, &format!("effects[{i}]"));
                    ui.indent(&effect_path, |ui| {
                        self.object_fields(ui, &effect_path, &spec, effect, edits)
                    });
                }
            });
        }
        let kinds: Vec<String> = effect_schema
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                s["properties"]["type"]["const"]
                    .as_str()
                    .map(str::to_string)
            })
            .collect();
        ui.push_id(("add effect", layer), |ui| {
            ui.menu_button("+ Add effect", |ui| {
                for kind in kinds {
                    if ui.button(&kind).clicked() {
                        edits.push(Edit::AddEffect { layer, kind });
                        ui.close();
                    }
                }
            });
        });
    }
}

/// True when a field does something here. A field whose schema has
/// `x-applies-when` (written from `duck_echo_core::applies`) does
/// something only when one of the siblings it names holds one of the
/// listed values; a sibling that is not written has its default. So a
/// square layer shows no noise settings, and a noise layer no duty.
fn applies(spec: &Value, props: &Map<String, Value>, values: &Map<String, Value>) -> bool {
    let Some(rule) = spec["x-applies-when"].as_object() else {
        return true;
    };
    rule.iter().any(|(sibling, allowed)| {
        let now = values
            .get(sibling)
            .or_else(|| props.get(sibling).and_then(|s| s.get("default")))
            .and_then(Value::as_str)
            .unwrap_or_default();
        allowed
            .as_array()
            .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(now)))
    })
}

/// The value a field that was left out starts at when it is ticked.
fn start_of(shape: &Shape, default: Option<&Value>) -> Value {
    if let Some(d) = default {
        return d.clone();
    }
    match shape {
        Shape::Number { min, max, integer } => {
            // The middle of the range, as the slider shows it.
            let mid = if logarithmic(*min, *max) {
                (min.max(0.001) * max).sqrt()
            } else {
                (min + max) / 2.0
            };
            number(mid, *integer)
        }
        Shape::Pitch => json!(440.0),
        Shape::Count => json!(0),
        Shape::Flag => json!(false),
        Shape::Choice(words) => json!(words.first().cloned().unwrap_or_default()),
        _ => json!({}),
    }
}

/// True for a range that spans many times over (times, Hz): the slider
/// moves through it evenly by ratio, so short times are as easy to reach
/// as long ones.
fn logarithmic(min: f64, max: f64) -> bool {
    min >= 0.0 && max >= 5.0 && max / min.max(0.01) >= 100.0
}

/// The control for one field that is not a table.
fn control(
    ui: &mut egui::Ui,
    path: &str,
    name: egui::Id,
    shape: &Shape,
    value: Option<&Value>,
    edits: &mut Vec<Edit>,
) {
    let set = |edits: &mut Vec<Edit>, value: Value, held: bool| {
        edits.push(Edit::Set {
            path: path.into(),
            value,
            held,
        })
    };
    // A random range, `{ min, max }`: each play picks a value between the
    // two. Shown in words, so a slider cannot quietly replace it; the text
    // changes it.
    if let Some(range) = value.and_then(Value::as_object)
        && let (Some(lo), Some(hi)) = (range.get("min"), range.get("max"))
        && matches!(shape, Shape::Number { .. } | Shape::Pitch)
    {
        ui.label(format!("random, {lo} to {hi}"))
            .on_hover_text("Each play picks a value between the two. Change it in the text.");
        return;
    }
    match shape {
        Shape::Number { min, max, integer } => {
            let mut v = value
                .and_then(Value::as_f64)
                .unwrap_or(*min)
                .clamp(*min, *max);
            let log = logarithmic(*min, *max);
            let mut slider = egui::Slider::new(&mut v, *min..=*max)
                .clamping(egui::SliderClamping::Always)
                .show_value(false);
            if *integer {
                slider = slider.integer();
            }
            if log {
                slider = slider.logarithmic(true).smallest_positive(0.001);
            }
            let response = ui.add(slider).labelled_by(name);
            slid(edits, &response, number(v, *integer), set);
            // The number beside it, to read and to type: egui's own box
            // inside the slider cannot be named for screen readers.
            let speed = if log {
                v.abs().max(0.001) * 0.01
            } else {
                (max - min) / 300.0
            };
            let typed = ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(*min..=*max)
                        .speed(speed)
                        .max_decimals(if *integer { 0 } else { 4 })
                        // As the file writes it: 0.06, not 0.0600.
                        .custom_formatter(|v, _| tidy(v).to_string()),
                )
                .labelled_by(name);
            slid(edits, &typed, number(v, *integer), set);
        }
        Shape::Count => {
            let mut v = value.and_then(Value::as_u64).unwrap_or(0);
            if ui
                .add(egui::DragValue::new(&mut v))
                .labelled_by(name)
                .changed()
            {
                set(edits, json!(v), false);
            }
        }
        Shape::Pitch => pitch(ui, path, name, value, edits),
        Shape::Choice(words) => {
            let current = value
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut chosen = current.clone();
            egui::ComboBox::from_id_salt(path)
                .selected_text(&chosen)
                .show_ui(ui, |ui| {
                    for word in words {
                        ui.selectable_value(&mut chosen, word.clone(), word);
                    }
                })
                .response
                .labelled_by(name);
            if chosen != current {
                set(edits, json!(chosen), false);
            }
        }
        Shape::Flag => {
            let mut on = value.and_then(Value::as_bool).unwrap_or(false);
            if ui.checkbox(&mut on, "").labelled_by(name).changed() {
                set(edits, json!(on), false);
            }
        }
        Shape::Table(_) | Shape::TextOnly => {}
    }
}

/// Turn a slider's response into edits: while dragged, held values and
/// then a release; a click or a key press, one value.
fn slid(
    edits: &mut Vec<Edit>,
    response: &egui::Response,
    value: Value,
    set: impl Fn(&mut Vec<Edit>, Value, bool),
) {
    // egui may round a value as it draws the slider (987.77 Hz to 987.8)
    // and call that a change. Only a change the person makes counts, so
    // opening a sound never changes it.
    let touched =
        response.dragged() || response.drag_stopped() || response.clicked() || response.has_focus();
    if response.changed() && touched {
        set(edits, value, response.dragged());
    }
    if response.drag_stopped() {
        edits.push(Edit::Release);
    }
}

/// A pitch: a slider in Hz and a note picker. The slider writes a number
/// of Hz; the picker writes a note name such as `"B5"`.
fn pitch(
    ui: &mut egui::Ui,
    path: &str,
    name: egui::Id,
    value: Option<&Value>,
    edits: &mut Vec<Edit>,
) {
    let (hz, written_note) = match value {
        Some(Value::String(name)) => (
            note::parse_note(name).map(f64::from).ok(),
            Some(name.clone()),
        ),
        Some(v) => (v.as_f64(), None),
        None => (None, None),
    };
    let mut v = hz.unwrap_or(440.0).clamp(20.0, 20_000.0);
    let set = |edits: &mut Vec<Edit>, value: Value, held: bool| {
        edits.push(Edit::Set {
            path: path.into(),
            value,
            held,
        })
    };
    let response = ui
        .add(
            egui::Slider::new(&mut v, 20.0..=20_000.0)
                .logarithmic(true)
                .clamping(egui::SliderClamping::Always)
                .show_value(false),
        )
        .labelled_by(name);
    slid(edits, &response, json!((v * 10.0).round() / 10.0), set);
    let speed = v * 0.005;
    let typed = ui
        .add(
            egui::DragValue::new(&mut v)
                .range(20.0..=20_000.0)
                .speed(speed)
                .suffix(" Hz")
                .max_decimals(1),
        )
        .labelled_by(name);
    slid(edits, &typed, json!((v * 10.0).round() / 10.0), set);
    let nearest = nearest_note(hz.unwrap_or(440.0));
    let shown = written_note.unwrap_or_else(|| format!("~{nearest}"));
    let mut chosen: Option<String> = None;
    egui::ComboBox::from_id_salt(format!("{path} note"))
        .selected_text(shown)
        .width(64.0)
        .show_ui(ui, |ui| {
            for number in 24..=108 {
                if let Some(note) = note::note_name(number)
                    && ui.selectable_label(note == nearest, &note).clicked()
                {
                    chosen = Some(note);
                }
            }
        })
        .response
        .labelled_by(name);
    if let Some(name) = chosen {
        edits.push(Edit::Set {
            path: path.into(),
            value: json!(name),
            held: false,
        });
    }
}

/// The name of the note nearest to `hz`.
pub fn nearest_note(hz: f64) -> String {
    (0..=127)
        .min_by(|a, b| {
            let da = (f64::from(note::note_number_to_hz(*a)) / hz).ln().abs();
            let db = (f64::from(note::note_number_to_hz(*b)) / hz).ln().abs();
            da.total_cmp(&db)
        })
        .and_then(note::note_name)
        .unwrap_or_else(|| "A4".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel drawn for `doc`, with no window.
    fn panel_for(doc: Value) -> egui_kittest::Harness<'static> {
        let schema = duck_echo_core::schema::recipe_schema();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(500.0, 2000.0))
            .build_ui(move |ui| {
                panel(ui, &schema, &doc);
            });
        harness.run();
        harness
    }

    fn shows(harness: &egui_kittest::Harness<'static>, label: &str) -> bool {
        use egui_kittest::kittest::Queryable;
        // The name and the controls it labels all answer to it.
        harness.query_all_by_label(label).next().is_some()
    }

    #[test]
    fn a_layer_shows_only_the_settings_its_source_uses() {
        use egui_kittest::kittest::Queryable;
        let square = panel_for(json!({"version": 0, "layers": [{"source": "square"}]}));
        assert!(shows(&square, "duty"));
        assert!(shows(&square, "aliasing"));
        assert!(shows(&square, "PITCH"));
        assert!(!shows(&square, "noise mode"));
        assert_eq!(square.query_all_by_label_contains("noise").count(), 0);

        let noise = panel_for(json!({"version": 0, "layers": [{"source": "noise"}]}));
        assert!(shows(&noise, "noise mode"));
        assert!(!shows(&noise, "duty"));
        assert!(!shows(&noise, "aliasing"));
        // Plain hiss has no pitch; pitched noise has one.
        assert!(!shows(&noise, "PITCH"));
        let pitched = panel_for(json!({"version": 0, "layers": [
            {"source": "noise", "noise_mode": "pitched"}
        ]}));
        assert!(shows(&pitched, "PITCH"));

        // A layer with no `source` is a square (the default).
        let plain = panel_for(json!({"version": 0, "layers": [{}]}));
        assert!(shows(&plain, "duty"));
    }

    #[test]
    fn slider_values_are_tidy() {
        assert_eq!(tidy(0.253_419_87), 0.2534);
        assert_eq!(tidy(1234.567), 1235.0);
        assert_eq!(tidy(12.345_6), 12.35);
        assert_eq!(tidy(0.002_345_6), 0.0023);
        assert_eq!(tidy(0.0), 0.0);
        assert_eq!(tidy(-0.5), -0.5);
    }

    #[test]
    fn notes_are_found_by_their_pitch() {
        assert_eq!(nearest_note(440.0), "A4");
        assert_eq!(nearest_note(987.8), "B5");
        assert_eq!(nearest_note(450.0), "A4");
    }

    #[test]
    fn times_and_hz_slide_by_ratio_and_levels_do_not() {
        assert!(logarithmic(0.0, 30.0)); // attack
        assert!(logarithmic(10.0, 20000.0)); // cutoff
        assert!(!logarithmic(0.0, 2.0)); // volume
        assert!(!logarithmic(-1.0, 1.0)); // pan
        assert!(!logarithmic(1.0, 16.0)); // bits
    }
}
