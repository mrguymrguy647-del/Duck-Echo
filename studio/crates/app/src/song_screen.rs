//! The song timeline's screen: the bar along its top (back to Home, Play,
//! Loop, Stop), the lanes with their notes and clips under a ruler of
//! bars and the loop marker, the play head, and the summary line. What
//! the timeline knows is in [`crate::song_view`]; this file draws it and
//! passes clicks on.

use std::time::Instant;

use eframe::egui::{self, Color32, RichText, Sense, Stroke};

use crate::app::StudioApp;
use crate::song_view::{Block, SongView};
use crate::theme;

/// The width of the track names on the left.
const HEADER: f32 = 170.0;
/// The height of one lane.
const LANE: f32 = 66.0;
/// The height of the ruler (loop marker and bar numbers).
const RULER: f32 = 36.0;
/// The narrowest a beat is drawn.
const MIN_BEAT: f32 = 18.0;
/// A note the note check found a problem with.
const PROBLEM: Color32 = Color32::from_rgb(235, 70, 70);

/// The key of the song playing from the timeline, in
/// [`StudioApp::playing`].
fn key_of(rel: &str) -> String {
    format!("song:{rel}")
}

impl StudioApp {
    /// Open one of the person's songs in the timeline.
    pub fn open_song(&mut self, rel: &str) {
        let Some(root) = &self.root else {
            self.warn("Your sounds folder cannot be read.".into());
            return;
        };
        match SongView::open(root, rel) {
            Ok(view) => {
                self.stop();
                self.song_view = Some(view);
                self.note = None;
            }
            Err(why) => self.warn(why),
        }
    }

    /// Back to Home: stop the song.
    pub fn close_song(&mut self) {
        if self
            .playing
            .as_deref()
            .is_some_and(|k| k.starts_with("song:"))
        {
            self.stop();
        }
        self.song_view = None;
        self.rescan();
    }

    /// Play the open song: once as written (`loop` decides), or over and
    /// over.
    pub fn play_open_song(&mut self, looping: bool) {
        let Some(view) = &mut self.song_view else {
            return;
        };
        view.looping = looping;
        let Some((song, arrangement)) = view.loaded.clone() else {
            self.warn("The song has a mistake, so it cannot play.".into());
            return;
        };
        let key = key_of(&view.rel);
        let heard = view.heard();
        match self
            .speakers
            .play_song_with(song, arrangement, looping.then_some(true), heard)
        {
            Ok(()) => {
                self.playing = Some(key);
                self.note = None;
            }
            Err(why) => self.warn(why),
        }
    }

    /// The song changed, or which tracks are heard: if it plays, a new
    /// render takes over at the play head.
    fn song_heard_again(&mut self) {
        let Some(view) = &self.song_view else {
            return;
        };
        if self.playing.as_deref() != Some(key_of(&view.rel).as_str()) {
            return;
        }
        if let Some((song, arrangement)) = view.loaded.clone() {
            let heard = view.heard();
            self.speakers.song_changed(song, arrangement, heard);
        }
    }

    /// The play head, in beats from the start of the song.
    fn play_head(&self, view: &SongView) -> Option<f64> {
        if self.playing.as_deref() != Some(key_of(&view.rel).as_str()) {
            return None;
        }
        let (song, arrangement) = view.loaded.as_ref()?;
        let frame = self.speakers.song_frame()? as f64;
        let beats = frame / f64::from(song.sample_rate) / song.seconds_per_beat();
        let total = f64::from(arrangement.bars * song.beats_per_bar);
        Some(if song.loop_ || view.looping {
            beats % total.max(1.0)
        } else {
            beats
        })
    }

    /// The timeline, in the middle of the window.
    pub(crate) fn song_screen(&mut self, ui: &mut egui::Ui) {
        let Some(mut view) = self.song_view.take() else {
            return;
        };
        let changed = match &self.root {
            Some(root) => view.tick(root, Instant::now()),
            None => false,
        };
        let head = self.play_head(&view);
        let playing_here = self.playing.as_deref() == Some(key_of(&view.rel).as_str());
        let mut back = false;
        let mut play = None;
        let mut stop = false;
        let mut toggled = false;

        egui::Panel::top("song bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("< Home").clicked() {
                    back = true;
                }
                ui.label(RichText::new(&view.rel).strong().size(16.0));
                ui.add_space(10.0);
                if ui.add(theme::primary_button("Play")).clicked() {
                    play = Some(false);
                }
                if ui
                    .selectable_label(playing_here && view.looping, "Loop")
                    .on_hover_text("Play it over and over, whatever the song says.")
                    .clicked()
                {
                    play = Some(true);
                }
                if ui.button("Stop").clicked() {
                    stop = true;
                }
                if playing_here && self.speakers.song_getting_ready() {
                    ui.spinner();
                    ui.label(RichText::new("Getting the song ready...").weak());
                }
                if let Some((song, arrangement)) = &view.loaded {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!(
                            "{} BPM, {}/4, {} bars{}",
                            song.tempo,
                            song.beats_per_bar,
                            arrangement.bars,
                            if song.loop_ { ", loops" } else { "" }
                        ))
                        .weak(),
                    );
                }
            });
            if view.gone {
                ui.label(
                    RichText::new(format!(
                        "{} is gone: it was deleted or renamed. Press < Home.",
                        view.rel
                    ))
                    .color(theme::LOOK),
                );
            }
            if let Some(problem) = &view.problem {
                ui.label(RichText::new(problem).color(theme::LOOK));
            }
        });

        egui::Panel::bottom("song summary").show(ui, |ui| {
            ui.add_space(2.0);
            match (&view.checked, &view.check_problem) {
                (_, Some(why)) => {
                    ui.label(RichText::new(why).color(theme::LOOK));
                }
                (Some(checked), None) => {
                    ui.label(&checked.summary);
                    let count = checked.problems.len();
                    if count > 0 {
                        ui.label(
                            RichText::new(format!(
                                "{count} note{} need{} a look: outlined in red. Hover over one \
                                 to see why.",
                                if count == 1 { "" } else { "s" },
                                if count == 1 { "s" } else { "" },
                            ))
                            .color(theme::LOOK),
                        );
                    }
                }
                (None, None) => {}
            }
            if view.checking() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Listening to every note...").weak());
                });
            }
            ui.add_space(2.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both()
                .auto_shrink(false)
                .show(ui, |ui| toggled = lanes(ui, &mut view, head));
        });

        if view.checking() || playing_here {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        self.song_view = Some(view);
        if changed || toggled {
            self.song_heard_again();
        }
        if let Some(looping) = play {
            self.play_open_song(looping);
        }
        if stop {
            self.stop();
        }
        if back {
            self.close_song();
        }
    }
}

/// Draw the ruler and the lanes. Returns true when a Mute or Solo button
/// was pressed.
fn lanes(ui: &mut egui::Ui, view: &mut SongView, head: Option<f64>) -> bool {
    let Some((song, arrangement)) = view.loaded.clone() else {
        return false;
    };
    let per_bar = f64::from(song.beats_per_bar);
    let beats = f64::from(arrangement.bars) * per_bar;
    let width_for_beats = (ui.available_width() - HEADER - 12.0).max(200.0);
    let beat_px = (width_for_beats / beats as f32).max(MIN_BEAT);
    let size = egui::vec2(
        HEADER + beat_px * beats as f32 + 8.0,
        RULER + LANE * view.lanes.len() as f32 + 8.0,
    );
    let (area, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(area);
    let x_of = |beat: f64| area.left() + HEADER + beat as f32 * beat_px;
    let visuals = ui.visuals().clone();
    let weak = visuals.weak_text_color();
    let font = egui::TextStyle::Small.resolve(ui.style());

    // The ruler: the loop marker on top, the bar numbers below.
    if song.loop_ {
        let band = egui::Rect::from_min_max(
            egui::pos2(x_of(0.0), area.top() + 2.0),
            egui::pos2(x_of(beats), area.top() + 14.0),
        );
        painter.rect_filled(band, 3.0, theme::NIGHT_INDIGO.gamma_multiply(0.55));
        painter.text(
            band.center(),
            egui::Align2::CENTER_CENTER,
            format!("loop: bars 1 to {}", arrangement.bars),
            font.clone(),
            Color32::WHITE,
        );
        for x in [band.left(), band.right()] {
            painter.line_segment(
                [egui::pos2(x, band.top()), egui::pos2(x, area.bottom())],
                Stroke::new(2.0, theme::NIGHT_INDIGO_TEXT),
            );
        }
    }
    for bar in 0..arrangement.bars {
        let x = x_of(f64::from(bar) * per_bar);
        painter.text(
            egui::pos2(x + 3.0, area.top() + 18.0),
            egui::Align2::LEFT_TOP,
            (bar + 1).to_string(),
            font.clone(),
            visuals.text_color(),
        );
        painter.line_segment(
            [
                egui::pos2(x, area.top() + RULER - 4.0),
                egui::pos2(x, area.bottom()),
            ],
            Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        );
    }

    let mut toggled = false;
    for (t, lane) in view.lanes.clone().iter().enumerate() {
        let top = area.top() + RULER + LANE * t as f32;
        let rect = egui::Rect::from_min_size(
            egui::pos2(area.left(), top),
            egui::vec2(size.x - 8.0, LANE - 2.0),
        );
        let heard = view.is_heard(t);
        let shade = if t % 2 == 0 {
            visuals.faint_bg_color
        } else {
            visuals.extreme_bg_color
        };
        painter.rect_filled(rect, 2.0, shade);

        // The header: name, instrument, Mute and Solo.
        let header = egui::Rect::from_min_size(rect.min, egui::vec2(HEADER - 6.0, LANE - 2.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header.shrink(4.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.style_mut().interaction.selectable_labels = false;
        child.push_id(("lane", t), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&lane.name).strong());
                ui.label(RichText::new(format!("vol {}", lane.volume)).small().weak());
            });
            ui.label(RichText::new(&lane.instrument).small().weak());
            ui.horizontal(|ui| {
                let muted = view.muted.contains(&t);
                if ui
                    .selectable_label(muted, "Mute")
                    .on_hover_text(format!("Silence {}", lane.name))
                    .clicked()
                {
                    view.toggle_mute(t);
                    toggled = true;
                }
                let solo = view.solo.contains(&t);
                if ui
                    .selectable_label(solo, "Solo")
                    .on_hover_text(format!("Hear only {}", lane.name))
                    .clicked()
                {
                    view.toggle_solo(t);
                    toggled = true;
                }
            });
        });

        // The clips of placed patterns, under the notes.
        for clip in &lane.clips {
            let r = egui::Rect::from_min_max(
                egui::pos2(x_of(clip.beat) + 1.0, top + 2.0),
                egui::pos2(x_of(clip.beat + clip.length) - 1.0, top + LANE - 4.0),
            );
            painter.rect(
                r,
                4.0,
                theme::NIGHT_INDIGO.gamma_multiply(0.18),
                Stroke::new(1.0, theme::NIGHT_INDIGO_TEXT),
                egui::StrokeKind::Inside,
            );
            let name = if clip.transpose == 0 {
                clip.pattern.clone()
            } else {
                format!("{} {:+}", clip.pattern, clip.transpose)
            };
            painter.text(
                egui::pos2(r.left() + 3.0, r.top() + 1.0),
                egui::Align2::LEFT_TOP,
                name,
                font.clone(),
                theme::NIGHT_INDIGO_TEXT,
            );
        }

        // The notes.
        let note_top = top + 16.0;
        let note_span = LANE - 30.0;
        for (i, block) in lane.blocks.iter().enumerate() {
            let y = note_top + (1.0 - block.height) * note_span;
            let r = egui::Rect::from_min_max(
                egui::pos2(x_of(block.beat) + 1.0, y),
                egui::pos2(
                    (x_of(block.beat + block.length) - 1.0).max(x_of(block.beat) + 4.0),
                    y + 10.0,
                ),
            );
            let alpha = if heard {
                0.45 + 0.55 * block.velocity
            } else {
                0.2
            };
            painter.rect_filled(r, 2.0, theme::ECHO_CYAN.gamma_multiply(alpha));
            let problem = view.problem_of(t, block).cloned();
            if problem.is_some() {
                painter.rect_stroke(
                    r.expand(1.5),
                    2.0,
                    Stroke::new(2.0, PROBLEM),
                    egui::StrokeKind::Outside,
                );
            }
            if r.width() >= 24.0 {
                painter.text(
                    egui::pos2(r.left() + 2.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    &block.label,
                    font.clone(),
                    theme::INK_NAVY,
                );
            }
            let name = note_name(&lane.name, block);
            let response = ui
                .interact(r, ui.scope_id().with(("note", t, i)), Sense::hover())
                .on_hover_ui(|ui| {
                    ui.label(RichText::new(&name).strong());
                    ui.label(format!(
                        "held {} beat{}, played at {}",
                        block.length,
                        if block.length == 1.0 { "" } else { "s" },
                        block.velocity
                    ));
                    if block.path.starts_with("patterns.") {
                        ui.label(RichText::new(format!("from {}", block.path)).weak());
                    }
                    if let Some(p) = &problem {
                        ui.label(RichText::new(&p.why).color(PROBLEM));
                    }
                });
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::GenericContainer, true, name.clone())
            });
        }
    }

    // The play head.
    if let Some(beat) = head {
        let x = x_of(beat);
        painter.line_segment(
            [
                egui::pos2(x, area.top() + RULER - 6.0),
                egui::pos2(x, area.bottom()),
            ],
            Stroke::new(2.0, theme::DUCK_YELLOW),
        );
    }
    let _ = weak;
    toggled
}

/// What a note is called for people: `keys: E5, bar 3, beat 0`.
pub fn note_name(track: &str, block: &Block) -> String {
    format!(
        "{track}: {}, bar {}, beat {}",
        block.label, block.bar, block.beat_in_bar
    )
}
