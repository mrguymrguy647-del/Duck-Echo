//! Roadmap task 10: every example in docs/AGENT_GUIDE.md works exactly
//! as the guide says, so the guide cannot quietly go out of date.

use duck_echo_core::Report;
use duck_echo_tools::edit::{apply_edits, load_value, EditOp};
use duck_echo_tools::measure::{measure_document, Options};
use duck_echo_tools::song::files_next_to;
use duck_echo_tools::{presets, Document};
use std::path::Path;

fn guide() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/AGENT_GUIDE.md"))
        .unwrap()
}

/// The code blocks of one language in the guide, in order.
fn blocks(language: &str) -> Vec<String> {
    let text = guide();
    let fence = format!("```{language}\n");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find(&fence) {
        let body = &rest[start + fence.len()..];
        let end = body.find("```").expect("closed block");
        out.push(body[..end].to_string());
        rest = &body[end..];
    }
    out
}

fn no_problems(report: &Report, what: &str) {
    assert!(report.errors.is_empty(), "{what}: {report}");
    assert!(report.warnings.is_empty(), "{what}: {report}");
}

#[test]
fn every_toml_example_is_valid_with_no_warnings() {
    let examples = blocks("toml");
    assert_eq!(examples.len(), 2, "a recipe and a song");
    let files = files_next_to(Path::new("guide.toml"));
    for (i, src) in examples.iter().enumerate() {
        let dir = std::env::temp_dir().join(format!("duck-echo-guide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("example{i}.toml"));
        std::fs::write(&path, src).unwrap();
        let doc = duck_echo_tools::load_document(&path, &files)
            .unwrap_or_else(|e| panic!("example {i}: {e}"));
        no_problems(doc.report(), &format!("example {i}"));
        let options = Options {
            check_notes: true,
            ..Options::default()
        };
        let m = measure_document(&doc, None, &options, "").unwrap().measured;
        let a = &m.analysis;
        assert!(!a.clipping.clipped, "example {i}");
        match &doc {
            Document::Recipe(_) => {
                // The coin example: at the pickup target.
                assert!(
                    (a.loudness_lufs.unwrap() + 19.0).abs() < 0.1,
                    "{}",
                    a.summary
                );
            }
            Document::Song(_) => {
                // The song example: a clean loop, every note heard, at
                // the music target, and nothing reported as extra.
                assert!(a.summary.contains("loop joins cleanly"), "{}", a.summary);
                let check = m.note_check.unwrap();
                assert_eq!(check.notes_heard, check.notes_written, "{check:?}");
                assert!(
                    check.tracks.iter().all(|t| t.extra_onsets.is_empty()),
                    "{check:?}"
                );
                assert!((a.loudness_lufs.unwrap() + 18.0).abs() < 0.1);
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn the_levelling_edit_works_on_a_preset_as_the_guide_says() {
    let ops_text = blocks("json");
    assert_eq!(ops_text.len(), 1);
    let ops: Vec<EditOp> = serde_json::from_str(&ops_text[0]).unwrap();
    let coin = presets::get("coin").unwrap();
    let mut doc = duck_echo_core::tree::parse_toml(coin.source)
        .unwrap()
        .to_json()
        .unwrap();
    apply_edits(&mut doc, &ops).unwrap();
    let loaded = load_value(&doc).unwrap();
    no_problems(&loaded.report, "levelled coin");
    assert_eq!(loaded.recipe.master.normalize_lufs, Some(-19.0));

    // From then on, a change to the sound keeps the level.
    let mut louder = doc.clone();
    let op: Vec<EditOp> =
        serde_json::from_str(r#"[{"path": "layers[0].source", "value": "saw"}]"#).unwrap();
    apply_edits(&mut louder, &op).unwrap();
    for d in [&doc, &louder] {
        let r = load_value(d).unwrap().recipe;
        let buf = duck_echo_synth::render(&r).unwrap();
        let a = duck_echo_analyze::analyze(&buf.samples, buf.sample_rate, buf.channels);
        assert!(
            (a.loudness_lufs.unwrap() + 19.0).abs() < 0.1,
            "{}",
            a.summary
        );
    }
}

#[test]
fn the_tables_in_the_guide_match_the_built_ins() {
    let text = guide();
    // Every preset and every category target is in the guide.
    for p in presets::list(None).unwrap() {
        assert!(text.contains(&p.name), "preset {} is missing", p.name);
        let target = format!("| {} | {} LUFS |", p.category, p.target_lufs.unwrap());
        assert!(text.contains(&target), "{target}");
    }
    // Every instrument with its range.
    for i in duck_echo_tools::instruments::list().unwrap() {
        if let Some([lo, hi]) = &i.range {
            let row = format!("| `{}` |", i.name);
            let line = text
                .lines()
                .find(|l| l.starts_with(&row))
                .unwrap_or_else(|| panic!("{row}"));
            assert!(line.contains(&format!("{lo} to {hi}")), "{line}");
        }
    }
    for kit in duck_echo_tools::instruments::KITS {
        for (piece, _) in kit.pieces {
            assert!(text.contains(&format!("`{piece}`")), "{piece}");
        }
    }
}
