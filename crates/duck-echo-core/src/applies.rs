//! Fields that do something only for some layers, in one table.
//!
//! `duty` shapes a square wave and nothing else; `noise_mode` only changes
//! noise. Three readers use this table, so they can never disagree:
//!
//! * [`crate::validate`] warns (`unused`) when such a field is set on a
//!   layer it does nothing for;
//! * [`crate::schema`] writes each rule into the JSON Schema as
//!   `x-applies-when`, where agents read it;
//! * Duck Echo Studio hides a setting whose rule does not hold, so the
//!   sliders only show what changes the sound.
//!
//! In the schema a rule reads, on the field it is about:
//!
//! ```json
//! "duty": { ..., "x-applies-when": { "source": ["square"] } }
//! ```
//!
//! "This field does something only when its sibling `source` is
//! `square`." With more than one sibling, any one of them is enough:
//! `pitch` applies when `source` is a wave **or** `noise_mode` is
//! `pitched`. A sibling that is not written has its default value.

/// One rule: `field` of the schema type `owner` does something only when
/// one of `when` holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Applies {
    /// The schema type the field belongs to: `Layer`.
    pub owner: &'static str,
    /// The field's name.
    pub field: &'static str,
    /// Sibling fields and the values that make the field do something.
    /// Any one is enough.
    pub when: &'static [(&'static str, &'static [&'static str])],
}

impl Applies {
    /// True when the field does something, given the value (as written in
    /// a recipe: `"square"`) of each sibling the rule names.
    pub fn holds(&self, value_of: impl Fn(&str) -> String) -> bool {
        self.when
            .iter()
            .any(|(sibling, values)| values.contains(&value_of(sibling).as_str()))
    }

    /// The rule as JSON, for the schema: `{"source": ["square"]}`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (sibling, values) in self.when {
            map.insert(
                (*sibling).to_string(),
                serde_json::Value::from(values.to_vec()),
            );
        }
        serde_json::Value::Object(map)
    }
}

/// `duty`: the width of a square wave's pulse.
pub const LAYER_DUTY: Applies = Applies {
    owner: "Layer",
    field: "duty",
    when: &[("source", &["square"])],
};

/// `aliasing`: how the jumps of a square or saw wave are drawn.
pub const LAYER_ALIASING: Applies = Applies {
    owner: "Layer",
    field: "aliasing",
    when: &[("source", &["square", "saw"])],
};

/// `noise_mode`: plain hiss or pitched noise.
pub const LAYER_NOISE_MODE: Applies = Applies {
    owner: "Layer",
    field: "noise_mode",
    when: &[("source", &["noise"])],
};

/// `pitch`: every wave has one; noise only when it is pitched.
pub const LAYER_PITCH: Applies = Applies {
    owner: "Layer",
    field: "pitch",
    when: &[
        ("source", &["sine", "square", "saw", "triangle"]),
        ("noise_mode", &["pitched"]),
    ],
};

/// Every rule.
pub const ALL: &[Applies] = &[LAYER_DUTY, LAYER_ALIASING, LAYER_NOISE_MODE, LAYER_PITCH];
