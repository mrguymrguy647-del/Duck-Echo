//! Duck Echo core: recipe types, loading, validation and JSON Schema.
//!
//! The recipe is the source of truth for every sound. This crate owns the
//! types ([`recipe`]), turns TOML or JSON text into them with exact error
//! paths ([`load`]), checks ranges and structure ([`validate`]), reports
//! problems in a machine-readable shape ([`report`]) and exports a JSON
//! Schema for agents ([`schema`]).
//!
//! It does no audio work. See `duck-echo-synth` for rendering.

pub mod load;
pub mod note;
pub mod recipe;
pub mod report;
pub mod schema;
pub mod structure;
pub mod tree;
pub mod validate;

pub use load::{load_path, load_str, parse_str, to_json, to_toml, Format, Loaded};
pub use note::Frequency;
pub use recipe::*;
pub use report::{Allowed, Code, Issue, Report, Severity};
pub use validate::validate;
