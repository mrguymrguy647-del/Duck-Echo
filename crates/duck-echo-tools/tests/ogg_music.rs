//! Phase 3, step 2: Duck Echo writes OGG Vorbis music and checks it
//! (docs/phase3-design.md, section 5).

use std::path::{Path, PathBuf};

use duck_echo_tools::ogg::RenderFormat;
use duck_echo_tools::song::files_next_to;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("duck-echo-ogg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn song(name: &str) -> duck_echo_tools::Document {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/songs")
        .join(format!("{name}.toml"));
    duck_echo_tools::load_document(&path, &files_next_to(&path)).unwrap()
}

fn render(
    doc: &duck_echo_tools::Document,
    out: &Path,
    format: RenderFormat,
) -> duck_echo_tools::Rendered {
    duck_echo_tools::render_document_to_file(doc, out, None, format, &Default::default(), "--")
        .unwrap_or_else(|e| panic!("{:?}", e.into_report()))
}

/// The Ogg stream serial number, from the first page's header.
fn serial_of(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes[14..18].try_into().unwrap())
}

#[test]
fn the_duck_craft_themes_become_small_exact_seamless_ogg_files() {
    let dir = folder("themes");
    // The sizes measured with Xiph's libvorbis at quality 4 when the
    // design was written; aoTuV's may differ a little.
    for (name, measured) in [
        ("duck_craft_day", 298_987.0),
        ("duck_craft_night", 278_871.0),
    ] {
        let doc = song(name);
        let out = dir.join(format!("{name}.ogg"));
        let done = render(&doc, &out, RenderFormat::default());
        let check = done.ogg.as_ref().expect("an OGG check");
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..4], b"OggS");
        assert_eq!(check.bytes, bytes.len());
        assert!(
            ((bytes.len() as f64) / measured - 1.0).abs() < 0.10,
            "{name}: {} bytes, measured {measured}",
            bytes.len()
        );
        assert!(
            check.wav_bytes > 13 * bytes.len(),
            "{name}: about 14 times smaller"
        );
        // Exactly as long as rendered, and the loop still joins cleanly.
        assert_eq!(check.frames, 1_008_000);
        assert_eq!(check.decoded_frames, 1_008_000);
        let seam = check.loop_seam.as_ref().unwrap();
        assert!(seam.clean, "{name}: {seam:?}");
        // The facts describe the OGG as a game hears it.
        let facts: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(format!("{name}.ogg.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(facts["format"], "ogg");
        assert_eq!(facts["quality"], 4.0);
        assert_eq!(facts["frames"], 1_008_000);
        assert_eq!(facts["loops"], true);
        assert!((facts["loudness_lufs"].as_f64().unwrap() + 18.0).abs() < 0.2);
        // The serial number comes from the recipe hash, and the same song
        // gives the same bytes.
        let hash = facts["recipe_hash"].as_str().unwrap();
        assert_eq!(
            serial_of(&bytes),
            duck_echo_audio::ogg::serial_from_hash(hash)
        );
        let again = dir.join(format!("{name}-again.ogg"));
        render(&doc, &again, RenderFormat::default());
        assert_eq!(bytes, std::fs::read(&again).unwrap());
        // DUCKECHO_FRAMES is in the file.
        let decoded = duck_echo_audio::ogg::decode_ogg(&bytes).unwrap();
        assert_eq!(decoded.frames_tag, Some(1_008_000));
    }
}

#[test]
fn quality_changes_the_size() {
    let dir = folder("quality");
    let doc = song("four_bars");
    let small = render(
        &doc,
        &dir.join("q1.ogg"),
        RenderFormat {
            quality: Some(1.0),
            ..Default::default()
        },
    );
    let big = render(
        &doc,
        &dir.join("q8.ogg"),
        RenderFormat {
            quality: Some(8.0),
            ..Default::default()
        },
    );
    let (small, big) = (small.ogg.unwrap(), big.ogg.unwrap());
    assert!(
        small.bytes * 2 < big.bytes,
        "{} vs {}",
        small.bytes,
        big.bytes
    );
    assert_eq!(small.quality, 1.0);
}

#[test]
fn analyze_compare_and_mix_check_read_ogg() {
    let dir = folder("read");
    let doc = song("four_bars");
    let wav = render(&doc, &dir.join("tune.wav"), RenderFormat::default());
    render(&doc, &dir.join("tune.ogg"), RenderFormat::default());
    // analyze: the same length and nearly the same loudness as the WAV.
    let buffer = duck_echo_tools::load_audio(&dir.join("tune.ogg")).unwrap();
    assert_eq!(buffer.frames(), wav.analysis.frames);
    let a = duck_echo_tools::analyze_buffer(&buffer);
    let lu = a.loudness_lufs.unwrap() - wav.analysis.loudness_lufs.unwrap();
    assert!(lu.abs() < 0.3, "{lu} LU");
    // compare: the two files sound the same.
    let c = duck_echo_tools::compare_paths(&dir.join("tune.wav"), &dir.join("tune.ogg")).unwrap();
    assert!(c.summary.contains("closely match"), "{}", c.summary);
    // mix_check: an OGG in a cue list.
    std::fs::write(
        dir.join("cues.json"),
        r#"[{"time": 0.5, "file": "tune.ogg", "name": "tune"}]"#,
    )
    .unwrap();
    let root = duck_echo_tools::root::Root::new(&dir).unwrap();
    let request = duck_echo_tools::mix::Request {
        cues: "cues.json",
        ..Default::default()
    };
    let mix = duck_echo_tools::mix::check(&root, &request, "").unwrap();
    assert_eq!(mix.cues[0].name, "tune");
    assert!(mix.cues[0].loudness_lufs.is_some());
    // list_files calls both audio, and the OGG's facts file facts.
    let listing = duck_echo_tools::files::list(&dir, &dir, false).unwrap();
    let kind = |name: &str| {
        listing
            .entries
            .iter()
            .find(|e| e.path == name)
            .unwrap()
            .kind
    };
    assert_eq!(kind("tune.ogg"), "audio");
    assert_eq!(kind("tune.ogg.json"), "asset_facts");
}

#[test]
fn wrong_settings_and_short_sounds_are_named() {
    let dir = folder("errors");
    let doc = song("four_bars");
    let fail = |out: &str, format: RenderFormat| {
        duck_echo_tools::render_document_to_file(
            &doc,
            &dir.join(out),
            None,
            format,
            &Default::default(),
            "--",
        )
        .unwrap_err()
        .at("--output")
        .into_report()
        .errors[0]
            .path
            .clone()
    };
    let q = |quality: f32| RenderFormat {
        quality: Some(quality),
        ..Default::default()
    };
    assert_eq!(fail("a.wav", q(4.0)), "--quality");
    assert_eq!(fail("a.ogg", q(11.0)), "--quality");
    let pcm24 = RenderFormat {
        wav: duck_echo_audio::WavFormat::Pcm24,
        quality: None,
    };
    assert_eq!(fail("a.ogg", pcm24), "--format");
    assert_eq!(fail("a.mp3", RenderFormat::default()), "--output");
    assert!(!dir.join("a.ogg").exists(), "nothing was written");

    // A short sound effect as OGG is allowed, with a warning.
    let coin =
        duck_echo_tools::load_document(Path::new("preset:coin"), &files_next_to(Path::new(".")))
            .unwrap();
    let done = render(&coin, &dir.join("coin.ogg"), RenderFormat::default());
    assert!(
        done.warnings
            .iter()
            .any(|w| w.message.contains("sound effects stay WAV")),
        "{:?}",
        done.warnings
    );
    assert_eq!(done.asset.format, "ogg");
    // A WAV's facts say wav, with no quality.
    let wav = render(&coin, &dir.join("coin.wav"), RenderFormat::default());
    assert_eq!(wav.asset.format, "wav");
    assert_eq!(wav.asset.quality, None);
}
