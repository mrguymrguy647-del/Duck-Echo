//! Analysis with extras: each layer on its own, only some layers heard
//! (`solo`, `mute`), and a picture of the sound.
//!
//! Layers are isolated by rendering the recipe with the other layers
//! silenced, finished with the full mix's gain and length
//! ([`duck_echo_synth::render_layers`]), so a layer analyzed alone reads at
//! the level and times it has in the mix.

use crate::notecheck::{check_track, NoteCheck};
use crate::song::{Document, SongFacts};
use crate::ToolError;
use duck_echo_analyze::{Analysis, PictureInfo};
use duck_echo_audio::AudioBuffer;
use duck_echo_core::{Arrangement, Code, Issue, Recipe, Song};
use serde::Serialize;

/// What to measure besides the whole sound.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extras {
    /// Also analyze each heard layer on its own. Recipes only.
    pub per_layer: bool,
    /// Hear only these layers (indices into `layers`); empty hears all.
    /// Recipes only.
    pub solo: Vec<usize>,
    /// Silence these layers. Recipes only.
    pub mute: Vec<usize>,
    /// Draw a picture (level panel and spectrogram) as PNG.
    pub picture: bool,
}

impl Extras {
    /// True when solo or mute changes what is heard.
    pub fn isolates(&self) -> bool {
        !self.solo.is_empty() || !self.mute.is_empty()
    }

    /// True when these extras need a recipe (a WAV file has no layers).
    pub fn needs_recipe(&self) -> bool {
        self.per_layer || self.isolates()
    }

    /// The name of the first extra that needs a recipe, for errors.
    fn recipe_only_argument(&self) -> Option<&'static str> {
        if self.per_layer {
            Some("per_layer")
        } else if !self.solo.is_empty() {
            Some("solo")
        } else if !self.mute.is_empty() {
            Some("mute")
        } else {
            None
        }
    }

    /// The error for asking a WAV file for layers, at `{prefix}per_layer`,
    /// `{prefix}solo` or `{prefix}mute`.
    pub fn wav_error(&self, prefix: &str) -> Option<ToolError> {
        self.recipe_only_argument().map(|arg| {
            ToolError::issue(Issue::error(
                Code::InvalidArgument,
                format!("{prefix}{arg}"),
                format!("`{arg}` needs a recipe (a .toml/.json file, preset:NAME or an inline recipe); a WAV file has no layers"),
            ))
        })
    }

    /// The indices of the layers heard, in order, for a recipe with
    /// `count` layers: `solo` (or every layer when it is empty) minus
    /// `mute`. Indices past the end are `out_of_range` errors at
    /// `{prefix}solo[i]` or `{prefix}mute[i]`; hearing nothing is an
    /// `invalid_argument` error.
    pub fn heard(&self, count: usize, prefix: &str) -> Result<Vec<usize>, ToolError> {
        let mut report = duck_echo_core::Report::new();
        for (arg, list) in [("solo", &self.solo), ("mute", &self.mute)] {
            for (i, &layer) in list.iter().enumerate() {
                if layer >= count {
                    report.push(
                        Issue::error(
                            Code::OutOfRange,
                            format!("{prefix}{arg}[{i}]"),
                            format!(
                                "layer {layer} does not exist; the recipe has {count} layer(s), numbered from 0"
                            ),
                        )
                        .got(layer)
                        .range(0.0, count.saturating_sub(1) as f64),
                    );
                }
            }
        }
        if !report.is_ok() {
            return Err(ToolError::Invalid(report));
        }
        let heard: Vec<usize> = (0..count)
            .filter(|i| self.solo.is_empty() || self.solo.contains(i))
            .filter(|i| !self.mute.contains(i))
            .collect();
        if heard.is_empty() {
            let arg = if self.mute.is_empty() { "solo" } else { "mute" };
            return Err(ToolError::issue(Issue::error(
                Code::InvalidArgument,
                format!("{prefix}{arg}"),
                "solo and mute leave no layer to hear",
            )));
        }
        Ok(heard)
    }
}

/// One layer analyzed on its own.
#[derive(Debug, Clone, Serialize)]
pub struct LayerAnalysis {
    /// Position in the recipe's `layers`.
    pub index: usize,
    /// The layer's `name`, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The layer as heard in the mix: same gain, timing and length.
    pub analysis: Analysis,
}

/// An analysis with the extras asked for. Serializes as the analysis
/// fields with the extras next to them, so a plain analysis keeps its shape.
#[derive(Debug, Clone, Serialize)]
pub struct Measured {
    /// The analysis of the sound (or of the heard layers).
    #[serde(flatten)]
    pub analysis: Analysis,
    /// Recipe validation and render warnings; omitted when there are none
    /// or the sound is a WAV file.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Issue>,
    /// With `solo` or `mute`: the layers heard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_layers: Option<Vec<usize>>,
    /// With `per_layer`: each heard layer analyzed on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<LayerAnalysis>>,
    /// For a song: tempo, bars and exact length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub song: Option<SongFacts>,
    /// For a song with `solo` or `mute`: the names of the tracks heard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heard_tracks: Option<Vec<String>>,
    /// For a song with `per_track`: each heard track analyzed on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracks: Option<Vec<TrackAnalysis>>,
    /// For a looping song: how the end of the file joins its start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_seam: Option<LoopSeam>,
    /// For a song, when its notes were checked: the notes written next to
    /// the notes heard, per track.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_check: Option<NoteCheck>,
    /// With `picture`: what the picture shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<PictureInfo>,
    /// The picture as PNG bytes; not serialized.
    #[serde(skip)]
    pub png: Option<Vec<u8>>,
}

/// A measured recipe and the audio that was measured.
pub struct RecipeMeasurement {
    /// The audio analyzed: the whole render, or only the heard layers.
    pub buffer: AudioBuffer,
    /// The analysis and extras.
    pub measured: Measured,
}

/// Measure a WAV buffer. Extras that need layers are an error.
pub fn measure_audio(buffer: &AudioBuffer, extras: &Extras) -> Result<Measured, ToolError> {
    if let Some(e) = extras.wav_error("") {
        return Err(e);
    }
    let analysis = crate::analyze_buffer(buffer);
    let (picture, png) = picture_of(buffer, &analysis, extras);
    Ok(Measured {
        analysis,
        warnings: Vec::new(),
        heard_layers: None,
        layers: None,
        song: None,
        heard_tracks: None,
        tracks: None,
        loop_seam: None,
        note_check: None,
        picture,
        png,
    })
}

/// Render and measure a validated recipe. `warnings` are its validation
/// warnings, passed through; a `normalize_lufs` target the peak ceiling
/// did not allow adds one.
pub fn measure_recipe(
    recipe: &Recipe,
    warnings: &[Issue],
    extras: &Extras,
) -> Result<RecipeMeasurement, ToolError> {
    let heard = extras.heard(recipe.layers.len(), "")?;
    let (full, mastering) = duck_echo_synth::render_mastered(recipe)?;
    let mut warnings = warnings.to_vec();
    if mastering.ceiling_limited {
        warnings.push(ceiling_warning(
            recipe.master.normalize_lufs.unwrap_or_default(),
            &full,
        ));
    }
    let buffer = if extras.isolates() {
        duck_echo_synth::render_layers(recipe, &heard, &mastering)?
    } else {
        full
    };
    let mut analysis = crate::analyze_buffer(&buffer);
    if let Some(note) = layer_note(recipe.layers.len(), heard.len(), extras.per_layer) {
        analysis.summary.push_str(", ");
        analysis.summary.push_str(&note);
    }
    let layers = if extras.per_layer {
        let mut out = Vec::with_capacity(heard.len());
        for &i in &heard {
            let alone = duck_echo_synth::render_layers(recipe, &[i], &mastering)?;
            out.push(LayerAnalysis {
                index: i,
                name: recipe.layers[i].name.clone(),
                analysis: crate::analyze_buffer(&alone),
            });
        }
        Some(out)
    } else {
        None
    };
    let (picture, png) = picture_of(&buffer, &analysis, extras);
    Ok(RecipeMeasurement {
        buffer,
        measured: Measured {
            analysis,
            warnings,
            heard_layers: extras.isolates().then_some(heard),
            layers,
            song: None,
            heard_tracks: None,
            tracks: None,
            loop_seam: None,
            note_check: None,
            picture,
            png,
        },
    })
}

/// What the summary adds about layers, for a recipe with `total` layers
/// of which `heard` are heard. A single layer needs no note. A mix of
/// several describes only the whole, so it says how many layers there are
/// and points at `per_layer`, unless `per_layer` is already on.
fn layer_note(total: usize, heard: usize, per_layer: bool) -> Option<String> {
    if total <= 1 {
        return None;
    }
    let count = if heard < total {
        format!("{heard} of {total} layers heard")
    } else {
        format!("{total} layers")
    };
    Some(if per_layer || heard <= 1 {
        count
    } else {
        format!("{count}; per_layer describes each")
    })
}

fn picture_of(
    buffer: &AudioBuffer,
    analysis: &Analysis,
    extras: &Extras,
) -> (Option<PictureInfo>, Option<Vec<u8>>) {
    if !extras.picture {
        return (None, None);
    }
    let onsets: Vec<f64> = analysis.onsets.iter().map(|o| o.time_seconds).collect();
    let pic = duck_echo_analyze::picture::draw(
        &buffer.samples,
        buffer.sample_rate,
        buffer.channels,
        &onsets,
    );
    (Some(pic.info), Some(pic.png))
}

/// The warning for a `normalize_lufs` target that needed more gain than
/// the peak ceiling allowed.
fn ceiling_warning(target: f32, full: &AudioBuffer) -> Issue {
    let ch = full.channels.max(1) as usize;
    let planar: Vec<Vec<f32>> = (0..ch)
        .map(|c| full.samples.iter().skip(c).step_by(ch).copied().collect())
        .collect();
    let slices: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
    let reached = duck_echo_analyze::loudness(&slices, full.sample_rate)
        .map(|(l, _)| format!("{l:.1} LUFS"))
        .unwrap_or_else(|| "too quiet to measure".into());
    Issue::warning(
        Code::Suspicious,
        "master.normalize_lufs",
        format!(
            "the target {target} LUFS needs the peak above the {} dBFS ceiling, so the gain stopped there and the render measures {reached}; lower the target, or make the sound less peaky (a longer attack, less resonance, a fuller wave) so it is louder at the same peak",
            duck_echo_core::LOUDNESS_CEILING_DBFS
        ),
    )
}

/// The measuring options as a front end receives them, before it is known
/// whether the document is a recipe, a song or a WAV file. `solo` and
/// `mute` are text: layer numbers for a recipe, track names (or numbers)
/// for a song.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Analyze each heard layer of a recipe on its own.
    pub per_layer: bool,
    /// Analyze each heard track of a song on its own.
    pub per_track: bool,
    /// Hear only these layers or tracks.
    pub solo: Vec<String>,
    /// Hear all layers or tracks but these.
    pub mute: Vec<String>,
    /// Draw a picture of the sound.
    pub picture: bool,
    /// For a song: listen for every written note (`note_check` in the
    /// result). `analyze` turns this on; it costs one extra render per
    /// track. Does nothing for a recipe or a WAV file.
    pub check_notes: bool,
    /// For a song: list every note in `note_check`, not only the
    /// problems. Turns the note check on.
    pub list_notes: bool,
}

impl Options {
    fn wrong_kind(prefix: &str, arg: &str, why: &str) -> ToolError {
        ToolError::issue(Issue::error(
            Code::InvalidArgument,
            format!("{prefix}{arg}"),
            why,
        ))
    }

    /// The options for a recipe. `solo` and `mute` must be layer numbers.
    pub fn for_recipe(&self, prefix: &str) -> Result<Extras, ToolError> {
        if self.per_track {
            return Err(Self::wrong_kind(
                prefix,
                "per_track",
                "`per_track` needs a song; a recipe has layers, so use `per_layer`",
            ));
        }
        if self.list_notes {
            return Err(Self::wrong_kind(
                prefix,
                "notes",
                "`notes` needs a song; a recipe has no written notes to list (its `onsets` show what is heard)",
            ));
        }
        let numbers = |arg: &str, list: &[String]| -> Result<Vec<usize>, ToolError> {
            list.iter()
                .enumerate()
                .map(|(i, text)| {
                    text.trim().parse::<usize>().map_err(|_| {
                        ToolError::issue(
                            Issue::error(
                                Code::TypeMismatch,
                                format!("{prefix}{arg}[{i}]"),
                                format!(
                                    "`{text}` is not a layer number; layers are numbered from 0"
                                ),
                            )
                            .got(text.clone()),
                        )
                    })
                })
                .collect()
        };
        Ok(Extras {
            per_layer: self.per_layer,
            solo: numbers("solo", &self.solo)?,
            mute: numbers("mute", &self.mute)?,
            picture: self.picture,
        })
    }

    /// The options for a WAV file, which has neither layers nor tracks.
    pub fn for_wav(&self, prefix: &str) -> Result<Extras, ToolError> {
        for (arg, on) in [
            ("per_layer", self.per_layer),
            ("per_track", self.per_track),
            ("solo", !self.solo.is_empty()),
            ("mute", !self.mute.is_empty()),
            ("notes", self.list_notes),
        ] {
            if on {
                return Err(Self::wrong_kind(
                    prefix,
                    arg,
                    &format!("`{arg}` needs a recipe or a song (a .toml/.json file, preset:NAME or an inline document); a WAV file has no layers or tracks"),
                ));
            }
        }
        Ok(Extras {
            picture: self.picture,
            ..Extras::default()
        })
    }

    /// The options for a song with tracks named `names`. `solo` and `mute`
    /// take track names, or track numbers from 0.
    pub fn for_song(&self, names: &[String], prefix: &str) -> Result<SongExtras, ToolError> {
        if self.per_layer {
            return Err(Self::wrong_kind(
                prefix,
                "per_layer",
                "`per_layer` needs a recipe; a song has tracks, so use `per_track`",
            ));
        }
        let mut report = duck_echo_core::Report::new();
        let mut pick = |arg: &str, list: &[String]| -> Vec<usize> {
            let mut out = Vec::new();
            for (i, text) in list.iter().enumerate() {
                let found = names.iter().position(|n| n == text).or_else(|| {
                    text.trim()
                        .parse::<usize>()
                        .ok()
                        .filter(|n| *n < names.len())
                });
                match found {
                    Some(index) => out.push(index),
                    None => {
                        let mut issue = Issue::error(
                            Code::NotFound,
                            format!("{prefix}{arg}[{i}]"),
                            format!("the song has no track named `{text}`"),
                        )
                        .got(text.clone())
                        .one_of(names.iter().cloned());
                        if let Some(s) = duck_echo_core::load::did_you_mean(text, names) {
                            issue = issue.suggest(s);
                        }
                        report.push(issue);
                    }
                }
            }
            out
        };
        let solo = pick("solo", &self.solo);
        let mute = pick("mute", &self.mute);
        if !report.is_ok() {
            return Err(ToolError::Invalid(report));
        }
        let heard: Vec<usize> = (0..names.len())
            .filter(|i| solo.is_empty() || solo.contains(i))
            .filter(|i| !mute.contains(i))
            .collect();
        if heard.is_empty() {
            let arg = if self.mute.is_empty() { "solo" } else { "mute" };
            return Err(Self::wrong_kind(
                prefix,
                arg,
                "solo and mute leave no track to hear",
            ));
        }
        Ok(SongExtras {
            per_track: self.per_track,
            isolates: !self.solo.is_empty() || !self.mute.is_empty(),
            heard,
            picture: self.picture,
            check_notes: self.check_notes || self.list_notes,
            list_notes: self.list_notes,
        })
    }
}

/// Measuring options for a song, checked against its tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongExtras {
    /// Analyze each heard track on its own.
    pub per_track: bool,
    /// True when `solo` or `mute` was given.
    pub isolates: bool,
    /// The indices of the tracks heard.
    pub heard: Vec<usize>,
    /// Draw a picture of the sound.
    pub picture: bool,
    /// Listen for every written note.
    pub check_notes: bool,
    /// List every note, not only the problems.
    pub list_notes: bool,
}

/// One track analyzed on its own, as heard in the mix.
#[derive(Debug, Clone, Serialize)]
pub struct TrackAnalysis {
    /// The track's position in the song's `tracks`.
    pub index: usize,
    /// The track's name.
    pub name: String,
    /// The instrument as written on the track.
    pub instrument: String,
    /// Notes on the track, with patterns placed and chords split.
    pub notes: usize,
    /// The analysis of the track alone.
    pub analysis: Analysis,
}

/// Render a song and analyze it.
///
/// A track heard alone (`solo`, `mute`, `per_track`) is a **stem**: it is
/// finished with the full song's master gain and length
/// ([`duck_echo_synth::song::render_song_tracks`]), the same rule as
/// `per_layer`. So a stem reads at the level and times it has in the mix,
/// and the stems of all tracks add up to the mix.
pub fn measure_song(
    song: &Song,
    arrangement: &Arrangement,
    facts: &SongFacts,
    warnings: &[Issue],
    extras: &SongExtras,
) -> Result<RecipeMeasurement, ToolError> {
    use duck_echo_synth::song::{render_song, render_song_tracks};
    let (full, mastering) = render_song(song, arrangement);
    let mut warnings = warnings.to_vec();
    if mastering.ceiling_limited {
        warnings.push(ceiling_warning(
            song.master.normalize_lufs.unwrap_or_default(),
            &full,
        ));
    }
    let buffer = if extras.isolates {
        render_song_tracks(song, arrangement, &extras.heard, &mastering)
    } else {
        full
    };
    let mut analysis = crate::analyze_buffer(&buffer);
    let loop_seam = song.loop_.then(|| loop_seam(&buffer));
    // Each heard track alone, when something needs it. One render per
    // track serves both the per-track analysis and the note check.
    let mut tracks = extras.per_track.then(Vec::new);
    let mut checks = extras.check_notes.then(Vec::new);
    let mut track_levels: Vec<(String, Option<f32>)> = Vec::new();
    if extras.per_track || extras.check_notes {
        for &i in &extras.heard {
            let alone = render_song_tracks(song, arrangement, &[i], &mastering);
            let track_analysis = crate::analyze_buffer(&alone);
            track_levels.push((song.tracks[i].name.clone(), track_analysis.loudness_lufs));
            if let Some(checks) = &mut checks {
                checks.push(check_track(
                    song,
                    &arrangement.tracks[i],
                    &song.tracks[i].instrument,
                    &alone,
                    &track_analysis.onsets,
                    extras.list_notes,
                ));
            }
            if let Some(tracks) = &mut tracks {
                tracks.push(TrackAnalysis {
                    index: i,
                    name: song.tracks[i].name.clone(),
                    instrument: song.tracks[i].instrument.clone(),
                    notes: arrangement.tracks[i].events.len(),
                    analysis: track_analysis,
                });
            }
        }
    }
    let note_check = checks.map(NoteCheck::of);
    analysis.summary = song_summary(
        facts,
        extras,
        &analysis,
        loop_seam.as_ref(),
        note_check.as_ref(),
        &track_levels,
    );

    // The picture of a song has a line at the start of each bar.
    let (picture, png) = if extras.picture {
        let onsets: Vec<f64> = analysis.onsets.iter().map(|o| o.time_seconds).collect();
        let bar_lines: Vec<f64> = (0..facts.bars)
            .map(|bar| bar as f64 * facts.seconds_per_bar)
            .collect();
        let pic = duck_echo_analyze::picture::draw_with_bars(
            &buffer.samples,
            buffer.sample_rate,
            buffer.channels,
            &onsets,
            &bar_lines,
        );
        (Some(pic.info), Some(pic.png))
    } else {
        (None, None)
    };
    Ok(RecipeMeasurement {
        buffer,
        measured: Measured {
            analysis,
            warnings,
            heard_layers: None,
            layers: None,
            song: Some(facts.clone()),
            heard_tracks: extras.isolates.then(|| {
                extras
                    .heard
                    .iter()
                    .map(|&i| song.tracks[i].name.clone())
                    .collect()
            }),
            tracks,
            loop_seam,
            note_check,
            picture,
            png,
        },
    })
}

/// A song's summary line, in the words that matter for a mix:
///
/// `8 bars looping at 140 BPM, 4 tracks; 13.71 s, -18.0 LUFS, no clipping,
/// loop joins cleanly, 155 of 155 notes heard; loudest track lead (-22.1
/// LUFS), quietest drums (-27.4 LUFS)`
///
/// A sound effect's words (brightness, attack, pitch, onsets) say little
/// about a whole mix, so they are left out; the fields are all still in
/// the analysis. `track_levels` holds each track heard alone, when the
/// tracks were rendered alone (for the note check or `per_track`).
fn song_summary(
    facts: &SongFacts,
    extras: &SongExtras,
    analysis: &Analysis,
    loop_seam: Option<&LoopSeam>,
    note_check: Option<&NoteCheck>,
    track_levels: &[(String, Option<f32>)],
) -> String {
    let mut head = facts.summary();
    if extras.isolates {
        head.push_str(&format!(" ({} heard)", extras.heard.len()));
    }
    if analysis.is_silent {
        return format!("{head}; {:.2} s, silent", analysis.duration_seconds);
    }
    let mut parts = vec![format!("{:.2} s", analysis.duration_seconds)];
    if let Some(l) = analysis.loudness_lufs {
        let approx = if analysis.loudness_method
            == Some(duck_echo_analyze::LoudnessMethod::KWeightedUngated)
        {
            "~"
        } else {
            ""
        };
        parts.push(format!("{approx}{l:.1} LUFS"));
    }
    if analysis.clipping.clipped {
        parts.push(format!(
            "CLIPPING ({} samples)",
            analysis.clipping.clipped_samples
        ));
    } else {
        parts.push("no clipping".into());
    }
    match loop_seam {
        Some(seam) if seam.clean => parts.push("loop joins cleanly".into()),
        Some(_) => parts.push("LOOP CLICKS AT THE JOIN".into()),
        None => parts.push(duck_echo_analyze::features::ending_words(analysis).into()),
    }
    if let Some(check) = note_check {
        parts.push(check.summary());
    }
    let mut line = format!("{head}; {}", parts.join(", "));
    let heard: Vec<(&str, f32)> = track_levels
        .iter()
        .filter_map(|(name, l)| l.map(|l| (name.as_str(), l)))
        .collect();
    let loudest = heard
        .iter()
        .copied()
        .fold(None, |best: Option<(&str, f32)>, t| match best {
            Some(b) if b.1 >= t.1 => Some(b),
            _ => Some(t),
        });
    let quietest = heard
        .iter()
        .copied()
        .fold(None, |best: Option<(&str, f32)>, t| match best {
            Some(b) if b.1 <= t.1 => Some(b),
            _ => Some(t),
        });
    match (loudest, quietest) {
        (Some(lo), Some(qu)) if heard.len() >= 2 => line.push_str(&format!(
            "; loudest track {} ({:.1} LUFS), quietest {} ({:.1} LUFS)",
            lo.0, lo.1, qu.0, qu.1
        )),
        _ if facts.tracks > 1 && !extras.per_track && extras.heard.len() > 1 => {
            line.push_str("; per_track describes each")
        }
        _ => {}
    }
    line
}

/// How the end of a looping file joins its start, so an agent can check a
/// loop without hearing it.
///
/// A click is a sudden jump between two samples. `seam_step` is the jump
/// from the last sample back to the first (the place the loop repeats);
/// `largest_step` is the biggest jump between two neighbouring samples
/// anywhere inside the file. When the jump at the join is no bigger than
/// the music's own, the join cannot click.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LoopSeam {
    /// The jump from the last sample to the first, largest of the
    /// channels, linear (1.0 is half of full scale to full scale).
    pub seam_step: f32,
    /// The largest jump between neighbouring samples inside the file.
    pub largest_step: f32,
    /// True when `seam_step` is no larger than `largest_step`.
    pub clean: bool,
}

/// Measure the join of a looping buffer.
pub fn loop_seam(buffer: &AudioBuffer) -> LoopSeam {
    let ch = buffer.channels.max(1) as usize;
    let frames = buffer.samples.len() / ch;
    let mut seam_step = 0.0f32;
    let mut largest_step = 0.0f32;
    if frames >= 2 {
        for c in 0..ch {
            let at = |frame: usize| buffer.samples[frame * ch + c];
            seam_step = seam_step.max((at(0) - at(frames - 1)).abs());
            for f in 1..frames {
                largest_step = largest_step.max((at(f) - at(f - 1)).abs());
            }
        }
    }
    LoopSeam {
        seam_step,
        largest_step,
        clean: seam_step <= largest_step,
    }
}

/// Render a recipe or a song and analyze it, with `seed` replacing the
/// document's own when given. `prefix` is put before argument names in
/// errors (`--` for the CLI, `arguments.` for the MCP server).
pub fn measure_document(
    doc: &Document,
    seed: Option<u64>,
    options: &Options,
    prefix: &str,
) -> Result<RecipeMeasurement, ToolError> {
    match doc {
        Document::Recipe(r) => {
            let extras = options.for_recipe(prefix)?;
            let mut recipe = r.loaded.recipe.clone();
            if let Some(s) = seed {
                recipe.seed = s;
            }
            extras.heard(recipe.layers.len(), prefix)?;
            measure_recipe(&recipe, &r.loaded.report.warnings, &extras)
        }
        Document::Song(s) => {
            let names: Vec<String> = s
                .loaded
                .song
                .tracks
                .iter()
                .map(|t| t.name.clone())
                .collect();
            let extras = options.for_song(&names, prefix)?;
            let mut song = s.loaded.song.clone();
            if let Some(seed) = seed {
                song.seed = seed;
            }
            measure_song(
                &song,
                &s.loaded.arrangement,
                &SongFacts::of(&s.loaded),
                &s.loaded.report.warnings,
                &extras,
            )
        }
    }
}
