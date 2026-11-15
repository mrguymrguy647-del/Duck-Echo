//! The song timeline's screen: the bar along its top (back to Home, Play,
//! Loop, Stop), the lanes with their notes and clips under a ruler of
//! bars and the loop marker, the play head, and the summary line. What
//! the timeline knows is in [`crate::song_view`]; this file draws it and
//! passes clicks on.

use std::time::Instant;

use eframe::egui::{self, Color32, RichText, Sense, Stroke};

use crate::app::StudioApp;
use crate::song_edit::{self, Edit};
use crate::song_view::{Block, ClipDrag, Lane, NoteDrag, SongView, shown_beat};
use crate::theme;

/// The width of the track names on the left.
const HEADER: f32 = 200.0;
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

    /// After an edit, an Undo or a Redo: the timeline shows the file as it
    /// is now, and a playing song is heard with the change.
    pub fn song_after_edit(&mut self) {
        let changed = match (&mut self.song_view, &self.root) {
            (Some(view), Some(root)) => view.reload(root),
            _ => false,
        };
        if changed {
            self.song_heard_again();
        }
    }

    /// Let the person hear one note of track `t` (`what` is a note name
    /// or a kit piece), as it is added or moved. Skipped while the song
    /// plays: the change is heard in the song a moment later.
    fn hear_note(&mut self, t: usize, what: &str) {
        let (Some(view), Some(root)) = (&self.song_view, &self.root) else {
            return;
        };
        if self.playing.as_deref() == Some(key_of(&view.rel).as_str()) {
            return;
        }
        let Some((song, arrangement)) = view.one_note(root, t, what) else {
            return;
        };
        let (buffer, _) = duck_echo_synth::song::render_song(&song, &arrangement);
        // Hearing the note is a help, not the change itself: with no
        // speakers the change is still made and said, and Play says why
        // nothing can be heard.
        let _ = self
            .speakers
            .play(duck_echo_live::Sound::from_samples(buffer), false);
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
        let mut actions = Vec::new();

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
                ui.add_space(12.0);
                ui.label("Height");
                ui.add(egui::Slider::new(&mut view.zoom, 0.6..=3.0).show_value(false))
                    .on_hover_text("Make the lanes taller or shorter");
                ui.label("Snap");
                egui::ComboBox::from_id_salt("snap")
                    .selected_text(snap_name(view.snap))
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        for step in [1.0, 0.5, 0.25] {
                            ui.selectable_value(&mut view.snap, step, snap_name(step));
                        }
                    })
                    .response
                    .on_hover_text("New and moved notes land on this grid");
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
                .show(ui, |ui| actions = lanes(ui, &mut view, head));
        });

        if view.checking() || playing_here {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30));
        }
        // Delete (or Backspace) takes out the note picked by a click.
        let delete = ui
            .ctx()
            .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && !ui.ctx().egui_wants_keyboard_input();
        if delete
            && let Some((t, index)) = view.selected
            && let Some((song, _)) = &view.loaded
            && let Some(edit) = song_edit::remove_note(&view.rel, song, t, index)
        {
            actions.push(Action::Run(edit, "Deleted the note.".into()));
        }
        self.song_view = Some(view);
        if changed {
            self.song_heard_again();
        }
        for action in actions {
            match action {
                Action::Heard => self.song_heard_again(),
                Action::Hear(t, what) => self.hear_note(t, &what),
                Action::Run((name, args), said) => {
                    if self.run_command(name, args).is_some() {
                        self.say(said);
                    }
                    self.song_after_edit();
                }
            }
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

/// What the person did in the lanes.
pub enum Action {
    /// Mute or Solo changed: the song is heard again.
    Heard,
    /// Run this command, and say this when it worked.
    Run(Edit, String),
    /// Let the person hear this note of this track (a note name or a kit
    /// piece): a note was added, or moved to a new pitch.
    Hear(usize, String),
}

/// The height of a lane's name, instrument and buttons.
const HEADER_HEIGHT: f32 = 96.0;
/// Room at the top of a lane for a clip's name.
const CLIP_LABEL: f32 = 16.0;

/// The rows of a lane on screen: their height, and the lane's height.
fn rows_of(lane: &Lane, zoom: f32) -> (f32, f32) {
    let row = if lane.kit {
        (18.0 * zoom).max(14.0)
    } else {
        (7.0 * zoom).max(3.0)
    };
    let height = (CLIP_LABEL + row * lane.rows as f32 + 8.0).max(HEADER_HEIGHT);
    (row, height)
}

/// The height of a note block: a little taller than a row, so a name
/// fits, but never so tall that a chord's notes (a few rows apart) hide
/// each other.
fn block_height(lane: &Lane, row: f32) -> f32 {
    if lane.kit {
        row - 4.0
    } else {
        13.0_f32.min(row * 2.6).max(5.0)
    }
}

/// Draw the ruler and the lanes, and turn what the person does there into
/// [`Action`]s.
fn lanes(ui: &mut egui::Ui, view: &mut SongView, head: Option<f64>) -> Vec<Action> {
    let mut actions = Vec::new();
    let Some((song, arrangement)) = view.loaded.clone() else {
        return actions;
    };
    let file = view.rel.clone();
    let per_bar = f64::from(song.beats_per_bar);
    let beats = f64::from(arrangement.bars) * per_bar;
    let width_for_beats = (ui.available_width() - HEADER - 12.0).max(200.0);
    let beat_px = (width_for_beats / beats as f32).max(MIN_BEAT);
    let lanes = view.lanes.clone();
    let heights: Vec<(f32, f32)> = lanes.iter().map(|l| rows_of(l, view.zoom)).collect();
    let total: f32 = heights.iter().map(|(_, h)| h + 2.0).sum();
    let size = egui::vec2(HEADER + beat_px * beats as f32 + 8.0, RULER + total + 8.0);
    let (area, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(area);
    let x0 = area.left() + HEADER;
    let x_of = |beat: f64| x0 + beat as f32 * beat_px;
    let beat_at = |x: f32| f64::from((x - x0) / beat_px);
    let visuals = ui.visuals().clone();
    let font = egui::TextStyle::Small.resolve(ui.style());
    let pointer = ui.input(|i| i.pointer.interact_pos());
    // Where the mouse button went down: a drag is measured from there
    // (egui starts a drag only after the pointer has moved a little).
    let press_origin = ui.input(|i| i.pointer.press_origin());

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

    let mut top = area.top() + RULER;
    for (t, lane) in lanes.iter().enumerate() {
        let (row_px, lane_h) = heights[t];
        let block_h = block_height(lane, row_px);
        let rect = egui::Rect::from_min_size(
            egui::pos2(area.left(), top),
            egui::vec2(size.x - 8.0, lane_h),
        );
        let timeline = egui::Rect::from_min_max(egui::pos2(x0, top), rect.max);
        let rows = lane.rows as f32;
        // The middle of row `r` (0 is the bottom row).
        let y_of = |r: f32| top + CLIP_LABEL + (rows - 1.0 - r) * row_px + row_px / 2.0;
        let row_at = |y: f32| ((rows - 1.0) - ((y - top - CLIP_LABEL) / row_px).floor()) as i32;
        let heard = view.is_heard(t);
        let shade = if t % 2 == 0 {
            visuals.faint_bg_color
        } else {
            visuals.extreme_bg_color
        };
        painter.rect_filled(rect, 2.0, shade);

        // Row names in the header's right edge: every kit piece, and
        // each C on a pitched lane, with a faint line across.
        for r in 0..lane.rows as i32 {
            let name = lane.note_at(r);
            let show = lane.kit
                || name
                    .as_deref()
                    .is_some_and(|n| n.starts_with('C') && !n.starts_with("C#"));
            if let (true, Some(name)) = (show, name) {
                let y = y_of(r as f32);
                painter.text(
                    egui::pos2(x0 - 6.0, y),
                    egui::Align2::RIGHT_CENTER,
                    name,
                    font.clone(),
                    visuals.weak_text_color(),
                );
                painter.line_segment(
                    [egui::pos2(x0, y), egui::pos2(rect.right(), y)],
                    Stroke::new(
                        1.0,
                        visuals
                            .widgets
                            .noninteractive
                            .bg_stroke
                            .color
                            .gamma_multiply(0.5),
                    ),
                );
            }
        }

        // The header: name, instrument, Mute and Solo, volume.
        let header = egui::Rect::from_min_size(rect.min, egui::vec2(HEADER - 52.0, HEADER_HEIGHT));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(header.shrink(4.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.style_mut().interaction.selectable_labels = false;
        child.spacing_mut().item_spacing.y = 3.0;
        child.push_id(("lane", t), |ui| {
            ui.label(RichText::new(&lane.name).strong());
            let mut chosen = lane.instrument.clone();
            let instrument = egui::ComboBox::from_id_salt("instrument")
                .selected_text(&chosen)
                .width(110.0)
                .show_ui(ui, |ui| {
                    for name in song_edit::instruments(&song) {
                        ui.selectable_value(&mut chosen, name.clone(), &name);
                    }
                })
                .response
                .on_hover_text(format!("What plays {}", lane.name));
            let named = format!("{} instrument", lane.name);
            instrument.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::ComboBox, true, named.clone())
            });
            if chosen != lane.instrument {
                actions.push(Action::Run(
                    song_edit::set_instrument(&file, t, &chosen),
                    format!("{} is played by {chosen} now.", lane.name),
                ));
            }
            ui.horizontal(|ui| {
                let muted = view.muted.contains(&t);
                if ui
                    .selectable_label(muted, "Mute")
                    .on_hover_text(format!("Silence {}", lane.name))
                    .clicked()
                {
                    view.toggle_mute(t);
                    actions.push(Action::Heard);
                }
                let solo = view.solo.contains(&t);
                if ui
                    .selectable_label(solo, "Solo")
                    .on_hover_text(format!("Hear only {}", lane.name))
                    .clicked()
                {
                    view.toggle_solo(t);
                    actions.push(Action::Heard);
                }
            });
            let mut volume = match view.volume_drag {
                Some((track, v)) if track == t => v,
                _ => lane.volume,
            };
            let response = ui
                .add(
                    egui::DragValue::new(&mut volume)
                        .range(0.0..=2.0)
                        .speed(0.01)
                        .prefix("volume ")
                        .max_decimals(2),
                )
                .on_hover_text("Drag or type the track's volume");
            if response.dragged() {
                view.volume_drag = Some((t, volume));
            }
            let done = response.drag_stopped() || (response.changed() && !response.dragged());
            if done {
                view.volume_drag = None;
                if (volume - lane.volume).abs() > f32::EPSILON {
                    actions.push(Action::Run(
                        song_edit::set_volume(&file, t, volume),
                        format!("{} is at volume {:.2} now.", lane.name, volume),
                    ));
                }
            }
        });

        // The lane itself: double-click adds a note; right-click places a
        // pattern. Drawn first, so the clips and notes on it take their
        // own clicks.
        let background = ui.interact(
            timeline,
            ui.scope_id().with(("lane area", t)),
            Sense::click(),
        );
        background.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::GenericContainer,
                true,
                format!("{} lane", lane.name),
            )
        });
        if background.double_clicked()
            && let Some(at) = background.interact_pointer_pos()
        {
            let beat = song_edit::snap(beat_at(at.x), view.snap).min(beats - view.snap.max(0.25));
            let row = row_at(at.y).clamp(0, lane.rows as i32 - 1);
            if let Some(edit) = song_edit::add_note(&file, &song, lane, beat, row) {
                let what = lane.note_at(row).unwrap_or_default();
                let (bar, in_bar) = song_edit::bar_and_beat(&song, beat);
                actions.push(Action::Run(
                    edit,
                    format!(
                        "Added {what} to {} at bar {bar}, beat {}.",
                        lane.name,
                        shown_beat(in_bar)
                    ),
                ));
                actions.push(Action::Hear(t, what));
            }
        }
        if background.secondary_clicked()
            && let Some(at) = background.interact_pointer_pos()
        {
            view.menu_at = Some((t, beat_at(at.x)));
        }
        background.context_menu(|ui| {
            let Some((_, beat)) = view.menu_at.filter(|(track, _)| *track == t) else {
                ui.close();
                return;
            };
            let bar = (beat / per_bar).floor().max(0.0) as u32 + 1;
            if song.patterns.is_empty() {
                ui.label("This song has no patterns to place.");
            }
            ui.menu_button(format!("Place a pattern at bar {bar}"), |ui| {
                for name in song.patterns.keys() {
                    if ui.button(name).clicked() {
                        if let Some(edit) = song_edit::place_pattern(&file, &song, t, name, bar) {
                            actions.push(Action::Run(
                                edit,
                                format!("Placed {name} on {} at bar {bar}.", lane.name),
                            ));
                        }
                        ui.close();
                    }
                }
            });
        });

        // The clips of placed patterns, under the notes.
        for (c, clip) in lane.clips.iter().enumerate() {
            let shift = match &view.clip_drag {
                Some(d) if d.track == t && d.clip == *clip => {
                    f64::from(d.to_bar as i32 - clip.bar as i32) * per_bar
                }
                _ => 0.0,
            };
            let r = egui::Rect::from_min_max(
                egui::pos2(x_of(clip.beat + shift) + 1.0, top + 2.0),
                egui::pos2(
                    x_of(clip.beat + shift + clip.length) - 1.0,
                    top + lane_h - 3.0,
                ),
            );
            painter.rect(
                r,
                4.0,
                theme::NIGHT_INDIGO.gamma_multiply(if shift != 0.0 { 0.35 } else { 0.18 }),
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
                &name,
                font.clone(),
                theme::NIGHT_INDIGO_TEXT,
            );
            let response = ui
                .interact(
                    r,
                    ui.scope_id().with(("clip", t, c)),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text(format!(
                    "{name} at bar {}: drag it to another bar",
                    clip.bar
                ));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::GenericContainer,
                    true,
                    format!("{}: {} at bar {}", lane.name, clip.pattern, clip.bar),
                )
            });
            if response.drag_started()
                && let Some(at) = press_origin
            {
                view.clip_drag = Some(ClipDrag {
                    track: t,
                    clip: clip.clone(),
                    origin: at.x,
                    to_bar: clip.bar,
                });
            }
            if response.dragged()
                && let (Some(at), Some(d)) = (pointer, view.clip_drag.as_mut())
            {
                let bars_moved = ((at.x - d.origin) / (beat_px * per_bar as f32)).round() as i32;
                d.to_bar = (clip.bar as i32 + bars_moved).clamp(1, arrangement.bars as i32) as u32;
            }
            if response.drag_stopped()
                && let Some(d) = view.clip_drag.take()
                && let Some(edit) = song_edit::move_clip(&file, &song, t, &d.clip, d.to_bar)
            {
                actions.push(Action::Run(
                    edit,
                    format!(
                        "Moved {} on {} to bar {}.",
                        d.clip.pattern, lane.name, d.to_bar
                    ),
                ));
            }
            response.context_menu(|ui| {
                if ui.button("Take this clip off").clicked() {
                    if let Some(edit) = song_edit::remove_clip(&file, &song, t, clip) {
                        actions.push(Action::Run(
                            edit,
                            format!(
                                "Took {} at bar {} off {}.",
                                clip.pattern, clip.bar, lane.name
                            ),
                        ));
                    }
                    ui.close();
                }
            });
        }

        // The notes.
        for (i, block) in lane.blocks.iter().enumerate() {
            let dragged = view
                .note_drag
                .as_ref()
                .filter(|d| d.track == t && Some(d.index) == block.index);
            let (beat, length, row) = match dragged {
                Some(d) => (d.new_beat, d.new_length, block.row + d.rows as f32),
                None => (block.beat, block.length, block.row),
            };
            let y = y_of(row);
            let r = egui::Rect::from_min_max(
                egui::pos2(x_of(beat) + 1.0, y - block_h / 2.0),
                egui::pos2(
                    (x_of(beat + length) - 1.0).max(x_of(beat) + 5.0),
                    y + block_h / 2.0,
                ),
            );
            let alpha = if heard {
                0.45 + 0.55 * block.velocity
            } else {
                0.2
            };
            painter.rect_filled(r, 2.0, theme::ECHO_CYAN.gamma_multiply(alpha));
            let problem = view.problem_of(t, block).cloned();
            let selected =
                block.index.is_some() && view.selected == Some((t, block.index.unwrap_or(0)));
            if selected || dragged.is_some() {
                painter.rect_stroke(
                    r.expand(1.5),
                    2.0,
                    Stroke::new(2.0, Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            } else if problem.is_some() {
                painter.rect_stroke(
                    r.expand(1.5),
                    2.0,
                    Stroke::new(2.0, PROBLEM),
                    egui::StrokeKind::Outside,
                );
            }
            if r.width() >= 24.0 && block_h >= 9.0 {
                painter.text(
                    egui::pos2(r.left() + 2.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    &block.label,
                    font.clone(),
                    theme::INK_NAVY,
                );
            }
            let name = note_name(&lane.name, block);
            let editable = block.index.is_some();
            let sense = if editable {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            };
            let response = ui
                .interact(r, ui.scope_id().with(("note", t, i)), sense)
                .on_hover_ui(|ui| {
                    ui.label(RichText::new(&name).strong());
                    ui.label(format!(
                        "held {} beat{}, played at {}",
                        block.length,
                        if block.length == 1.0 { "" } else { "s" },
                        block.velocity
                    ));
                    if editable {
                        ui.label(
                            RichText::new(
                                "Drag it to move it, drag its right edge to make it longer, \
                                 right-click to delete it.",
                            )
                            .weak(),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "From the pattern in its clip ({}). Drag the clip to move it.",
                                block.path
                            ))
                            .weak(),
                        );
                    }
                    if let Some(p) = &problem {
                        ui.label(RichText::new(&p.why).color(PROBLEM));
                    }
                });
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::GenericContainer, true, name.clone())
            });
            let Some(index) = block.index else {
                continue;
            };
            let near_end = |x: f32| x > r.right() - 6.0;
            if response.hovered() && pointer.is_some_and(|p| near_end(p.x)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if response.clicked() {
                view.selected = Some((t, index));
            }
            if response.drag_started()
                && let Some(at) = press_origin
            {
                view.selected = Some((t, index));
                view.note_drag = Some(NoteDrag {
                    track: t,
                    index,
                    stretch: near_end(at.x),
                    origin: (at.x, at.y),
                    beat: block.beat,
                    length: block.length,
                    row: block.row,
                    new_beat: block.beat,
                    new_length: block.length,
                    rows: 0,
                });
            }
            if response.dragged()
                && let Some(at) = pointer
                && let Some(d) = view.note_drag.as_mut()
                && d.track == t
                && d.index == index
            {
                let moved = f64::from((at.x - d.origin.0) / beat_px);
                if d.stretch {
                    d.new_length =
                        song_edit::snap(d.length + moved, view.snap).max(view.snap.max(0.25));
                } else {
                    d.new_beat = song_edit::snap(d.beat + moved, view.snap).min(beats - 0.25);
                    let rows = ((d.origin.1 - at.y) / row_px).round() as i32;
                    let lowest = -(d.row.round() as i32);
                    let highest = lane.rows as i32 - 1 - d.row.round() as i32;
                    let rows = rows.clamp(lowest, highest);
                    if rows != d.rows {
                        d.rows = rows;
                        // Hear the new pitch as it moves.
                        let what = if lane.kit {
                            lane.note_at(d.row.round() as i32 + rows)
                        } else {
                            Some(block.label.clone()).and_then(|l| {
                                duck_echo_core::note::note_number(&l)
                                    .ok()
                                    .and_then(|n| duck_echo_core::note::note_name(n + rows))
                            })
                        };
                        if let Some(what) = what {
                            actions.push(Action::Hear(t, what));
                        }
                    }
                }
            }
            if response.drag_stopped()
                && let Some(d) = view.note_drag.take()
            {
                if d.stretch {
                    if d.new_length != d.length
                        && let Some(edit) =
                            song_edit::resize_note(&file, &song, t, index, d.new_length)
                    {
                        actions.push(Action::Run(
                            edit,
                            format!(
                                "{} on {} is {} beats long now.",
                                block.label, lane.name, d.new_length
                            ),
                        ));
                    }
                } else if d.new_beat != d.beat || d.rows != 0 {
                    let piece = lane
                        .kit
                        .then(|| lane.note_at(d.row.round() as i32 + d.rows))
                        .flatten();
                    let semitones = if lane.kit { 0 } else { d.rows };
                    if let Some(edit) =
                        song_edit::move_note(&file, &song, t, index, d.new_beat, semitones, piece)
                    {
                        let (bar, in_bar) = song_edit::bar_and_beat(&song, d.new_beat);
                        actions.push(Action::Run(
                            edit,
                            format!(
                                "Moved the note on {} to bar {bar}, beat {}.",
                                lane.name,
                                shown_beat(in_bar)
                            ),
                        ));
                    }
                }
            }
            response.context_menu(|ui| {
                if ui.button("Delete note").clicked() {
                    if let Some(edit) = song_edit::remove_note(&file, &song, t, index) {
                        actions.push(Action::Run(
                            edit,
                            format!("Deleted {} from {}.", block.label, lane.name),
                        ));
                    }
                    ui.close();
                }
            });
        }
        top += lane_h + 2.0;
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
    actions
}

/// The snap grid for people.
fn snap_name(step: f64) -> &'static str {
    match step {
        s if s >= 1.0 => "1 beat",
        s if s >= 0.5 => "1/2 beat",
        _ => "1/4 beat",
    }
}

/// What a note is called for people, with its beat counted from 1:
/// `keys: E5, bar 3, beat 1`.
pub fn note_name(track: &str, block: &Block) -> String {
    format!(
        "{track}: {}, bar {}, beat {}",
        block.label,
        block.bar,
        shown_beat(block.beat_in_bar)
    )
}
