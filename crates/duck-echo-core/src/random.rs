//! Random ranges: any number in a sound's recipe may be written as
//! `{ min = x, max = y }`, and each render picks a value between the two
//! from the seed (roadmap task 5).
//!
//! The structure walker ([`crate::structure`]) finds every range, checks it
//! is a table of exactly `min` and `max`, and decodes the recipe with each
//! range at its `min`. The ranges themselves ride along on
//! [`crate::Recipe::ranges`]. Validation checks the recipe twice, with
//! every range at its `min` and at its `max` (see [`crate::load`]), so both
//! ends must be allowed values and an error names the end at fault, such
//! as `layers[0].volume.max`.
//!
//! [`resolve`] then picks the values for one seed. Each field has its own
//! stream: the seed mixed with the field's path. So adding or removing a
//! range never changes the value another field picks, and the same seed
//! always picks the same values. The picks use integer math only (no
//! platform `libm`), so they are the same on every computer.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::recipe::Recipe;

/// One field written as a range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RandomRange {
    /// The field, such as `layers[0].volume`, inside the recipe.
    pub path: String,
    /// The lowest value it may take.
    pub min: f64,
    /// The highest value it may take.
    pub max: f64,
}

impl RandomRange {
    /// The value this range takes for `seed`: `min` plus a share of the
    /// way to `max`, from 0 up to (not quite) 1.
    pub fn pick(&self, seed: u64) -> f64 {
        let mut state = seed ^ fnv1a(self.path.as_bytes());
        // Two rounds, so seeds 0, 1, 2 ... are far apart.
        splitmix(&mut state);
        let bits = splitmix(&mut state) >> 11; // 53 bits
        let share = bits as f64 / (1u64 << 53) as f64;
        self.min + (self.max - self.min) * share
    }
}

/// Which value every range takes in [`at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// Every range at its `min`.
    Min,
    /// Every range at its `max`.
    Max,
}

/// The recipe that plays for `seed`: every range replaced by its pick. A
/// recipe without ranges comes back unchanged (and without a round trip,
/// so its renders stay byte-identical).
pub fn resolve(recipe: &Recipe, seed: u64) -> Recipe {
    set_all(recipe, |r| r.pick(seed))
}

/// The recipe with every range at one end, for validation.
pub fn at(recipe: &Recipe, end: End) -> Recipe {
    set_all(recipe, |r| match end {
        End::Min => r.min,
        End::Max => r.max,
    })
}

fn set_all(recipe: &Recipe, value: impl Fn(&RandomRange) -> f64) -> Recipe {
    if recipe.ranges.is_empty() {
        return recipe.clone();
    }
    let mut json = serde_json::to_value(recipe).expect("a recipe serializes");
    for range in &recipe.ranges {
        set(&mut json, &range.path, value(range));
    }
    let mut out: Recipe = serde_json::from_value(json).expect("a recipe with numbers decodes");
    out.ranges = Vec::new();
    out
}

/// Put `value` at `path` (`a.b[2].c`), creating the last key if the
/// serializer left it out.
fn set(json: &mut serde_json::Value, path: &str, value: f64) {
    let mut node = json;
    let parts = split(path);
    let Some((last, parents)) = parts.split_last() else {
        return;
    };
    for part in parents {
        node = match part {
            Part::Key(k) => match node.get_mut(k.as_str()) {
                Some(n) => n,
                None => return,
            },
            Part::Index(i) => match node.get_mut(*i) {
                Some(n) => n,
                None => return,
            },
        };
    }
    let number = serde_json::Value::from(value);
    match last {
        Part::Key(k) => {
            if let Some(obj) = node.as_object_mut() {
                obj.insert(k.clone(), number);
            }
        }
        Part::Index(i) => {
            if let Some(slot) = node.get_mut(*i) {
                *slot = number;
            }
        }
    }
}

enum Part {
    Key(String),
    Index(usize),
}

fn split(path: &str) -> Vec<Part> {
    let mut out = Vec::new();
    for piece in path.split('.') {
        let mut rest = piece;
        if let Some(open) = rest.find('[') {
            if open > 0 {
                out.push(Part::Key(rest[..open].to_string()));
            }
            rest = &rest[open..];
            while let Some(stripped) = rest.strip_prefix('[') {
                let close = stripped.find(']').unwrap_or(stripped.len());
                if let Ok(i) = stripped[..close].parse() {
                    out.push(Part::Index(i));
                }
                rest = stripped.get(close + 1..).unwrap_or("");
            }
        } else {
            out.push(Part::Key(rest.to_string()));
        }
    }
    out
}

/// FNV-1a, 64 bits: a field's path as a number.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// SplitMix64, one step.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_stays_inside_its_range_and_depends_on_the_seed_and_the_field() {
        let r = RandomRange {
            path: "layers[0].volume".into(),
            min: 0.6,
            max: 0.9,
        };
        let picks: Vec<f64> = (0..100).map(|s| r.pick(s)).collect();
        assert!(picks.iter().all(|v| (0.6..0.9).contains(v)));
        assert!(picks.windows(2).any(|w| w[0] != w[1]));
        assert_eq!(r.pick(7), r.pick(7));
        let other = RandomRange {
            path: "layers[1].volume".into(),
            ..r.clone()
        };
        assert_ne!(r.pick(7), other.pick(7));
    }

    #[test]
    fn paths_split_into_keys_and_indexes() {
        let mut v = serde_json::json!({"layers": [{"pitch": {"start": 1.0}}]});
        set(&mut v, "layers[0].pitch.start", 2.5);
        set(&mut v, "layers[0].volume", 0.5);
        assert_eq!(v["layers"][0]["pitch"]["start"], 2.5);
        assert_eq!(v["layers"][0]["volume"], 0.5);
    }
}
