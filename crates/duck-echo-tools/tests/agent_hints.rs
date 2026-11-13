//! Two small helps for agents: the summary of a layered recipe says how
//! many layers it has, and an unknown preset tag suggests a real one.

use duck_echo_core::{load_str, Code, Format, Recipe};
use duck_echo_tools::measure::{measure_recipe, Extras};
use duck_echo_tools::{presets, ToolError};

fn recipe(src: &str) -> Recipe {
    load_str(src, Format::Toml).expect("valid recipe").recipe
}

const THREE_LAYERS: &str = "version = 0\n\
    [[layers]]\nsource = \"sine\"\npitch = { start = \"C5\" }\n\
    [[layers]]\nsource = \"sine\"\npitch = { start = \"E5\" }\n\
    [[layers]]\nsource = \"sine\"\npitch = { start = \"G5\" }\n";

fn summary(r: &Recipe, extras: &Extras) -> String {
    measure_recipe(r, &[], extras)
        .unwrap()
        .measured
        .analysis
        .summary
}

#[test]
fn summary_counts_layers_and_points_at_per_layer() {
    let r = recipe(THREE_LAYERS);
    let plain = summary(&r, &Extras::default());
    assert!(
        plain.ends_with(", 3 layers; per_layer describes each"),
        "{plain}"
    );

    // With per_layer on, the hint is not needed.
    let with_layers = Extras {
        per_layer: true,
        ..Extras::default()
    };
    let s = summary(&r, &with_layers);
    assert!(s.ends_with(", 3 layers"), "{s}");

    // Solo and mute say how many of them were heard.
    let two = Extras {
        mute: vec![1],
        ..Extras::default()
    };
    let s = summary(&r, &two);
    assert!(
        s.ends_with(", 2 of 3 layers heard; per_layer describes each"),
        "{s}"
    );
    let one = Extras {
        solo: vec![2],
        ..Extras::default()
    };
    let s = summary(&r, &one);
    assert!(s.ends_with(", 1 of 3 layers heard"), "{s}");

    // A single layer needs no note.
    let single = recipe("version = 0\n[[layers]]\nsource = \"sine\"\n");
    let s = summary(&single, &Extras::default());
    assert!(!s.contains("layer"), "{s}");
}

#[test]
fn unknown_tag_suggests_the_closest_real_one() {
    for (typo, want) in [
        ("explsion", "explosion"),
        ("UI", "ui"),
        ("pickups", "pickup"),
    ] {
        match presets::list(Some(typo)) {
            Err(ToolError::Failed(issue)) => {
                assert_eq!(issue.code, Code::NotFound, "{typo}");
                assert_eq!(issue.suggestion.as_deref(), Some(want), "{typo}");
                let allowed = serde_json::to_value(&issue.allowed).unwrap();
                let options = allowed["options"].as_array().unwrap();
                // Categories come first, then every other tag once.
                assert_eq!(options[0], "ui");
                assert!(options.iter().any(|o| o == "explosion"));
            }
            other => panic!("{typo}: expected an error, got {other:?}"),
        }
    }
    // Real tags still list their presets, and no tag lists them all.
    assert!(!presets::list(Some("explosion")).unwrap().is_empty());
    assert_eq!(presets::list(None).unwrap().len(), presets::PRESETS.len());
}
