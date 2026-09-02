//! Anim event markers on a `.anim` asset, as a minimal editable list —
//! Task 41's authoring surface for notifies (a timeline editor is explicitly
//! out of scope). Opened by double-clicking a `.anim` in the asset browser;
//! Save rewrites the binary (markers are part of the clip asset, format v2)
//! and the host invalidates the clip caches so running machines pick the
//! edit up.

use std::path::PathBuf;

use crate::engine::assets::mesh_import::{load_anim_binary, write_anim_binary};
use crate::engine::assets::model_loader::{AnimEventMarker, RawAnimationClip};
use crusty_gui::context::Ui;
use crusty_gui::math::Vec2;
use crusty_gui::widgets::{Button, ComboBox, DragValue, Label, SelectableValue, TextEdit, Window};

/// The open dialog: one `.anim` container, all of its clips, markers editable
/// in place. Nothing touches the disk until Save.
pub struct AnimEventsDialog {
    /// Content-relative path — the display name, and what the host
    /// invalidates in the clip caches after a save.
    pub relative: String,
    abs_path: PathBuf,
    bone_names: Vec<String>,
    clips: Vec<RawAnimationClip>,
    selected: usize,
}

/// What the panel asked the host to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimEventsAction {
    None,
    Save,
    Cancel,
}

impl AnimEventsDialog {
    /// Load the container for editing.
    pub fn load(abs_path: PathBuf, relative: String) -> Result<Self, String> {
        let (bone_names, clips) =
            load_anim_binary(&abs_path).map_err(|e| format!("'{relative}': {e}"))?;
        if clips.is_empty() {
            return Err(format!("'{relative}' contains no clips"));
        }
        Ok(Self {
            relative,
            abs_path,
            bone_names,
            clips,
            selected: 0,
        })
    }

    /// Write the edited markers back (sorted by time — the order crossing
    /// detection and readers expect).
    pub fn save(&mut self) -> Result<(), String> {
        for clip in &mut self.clips {
            clip.events
                .sort_by(|a, b| a.time_seconds.total_cmp(&b.time_seconds));
        }
        write_anim_binary(&self.abs_path, &self.clips, &self.bone_names)
            .map_err(|e| format!("'{}': {e}", self.relative))
    }

    /// The markers of the selected clip — what the panel's list edits in
    /// place (and what ticket 04's authoring pickers will read).
    pub fn selected_events_mut(&mut self) -> &mut Vec<AnimEventMarker> {
        &mut self.clips[self.selected].events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::assets::model_loader::RawAnimationClip;

    /// Load → edit the list → save → reload: the marker edit is on disk, and
    /// the file stays a valid `.anim` container.
    #[test]
    fn edits_round_trip_through_the_asset() {
        let dir = std::env::temp_dir().join("rust_engine_anim_events_dialog");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("hero.anim");
        write_anim_binary(
            &path,
            &[RawAnimationClip {
                name: "Walk".into(),
                duration_seconds: 1.0,
                channels: vec![],
                events: vec![],
            }],
            &["root".to_string()],
        )
        .expect("fixture written");

        let mut dlg =
            AnimEventsDialog::load(path.clone(), "anims/hero.anim".into()).expect("loads");
        assert!(dlg.selected_events_mut().is_empty(), "viewable: starts empty");
        dlg.selected_events_mut().extend([
            AnimEventMarker {
                time_seconds: 0.7,
                name: "plant".into(),
            },
            AnimEventMarker {
                time_seconds: 0.2,
                name: "lift".into(),
            },
        ]);
        dlg.save().expect("saves");

        let (_, clips) = load_anim_binary(&path).expect("reloads");
        assert_eq!(
            clips[0]
                .events
                .iter()
                .map(|e| (e.time_seconds, e.name.as_str()))
                .collect::<Vec<_>>(),
            vec![(0.2, "lift"), (0.7, "plant")],
            "edits persist, sorted by time"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}

/// The modal list: a clip selector when the container holds several, then one
/// row per marker (time in seconds, name, remove), an add button, Save/Cancel.
pub fn anim_events_dialog_panel(ui: &mut Ui, dlg: &mut AnimEventsDialog) -> AnimEventsAction {
    let mut action = AnimEventsAction::None;
    let dim = ui.style().palette.text_secondary;

    Window::new(format!("Anim Events: {}", dlg.relative))
        .modal(true)
        .resizable(false)
        .collapsible(false)
        .anchor_center(true)
        .default_size(Vec2::new(420.0, 160.0))
        .auto_size(true)
        .show(ui, |ui| {
            if dlg.clips.len() > 1 {
                ui.horizontal(|ui| {
                    Label::new("Clip:").show(ui);
                    ComboBox::new("anim_events_clip")
                        .selected_text(&dlg.clips[dlg.selected].name)
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for i in 0..dlg.clips.len() {
                                SelectableValue::new(&mut dlg.selected, i, &dlg.clips[i].name)
                                    .show(ui);
                            }
                        });
                });
                ui.add_space(6.0);
            }

            let clip = &mut dlg.clips[dlg.selected];
            let duration = clip.duration_seconds;
            Label::new(format!(
                "{} — {duration:.2}s, {} marker(s)",
                clip.name,
                clip.events.len()
            ))
            .color(dim)
            .show(ui);
            ui.add_space(6.0);

            let mut remove: Option<usize> = None;
            for (i, ev) in clip.events.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    DragValue::new(&mut ev.time_seconds)
                        .speed(0.01)
                        .range(0.0..=duration)
                        .decimals(2)
                        .min_decimals(2)
                        .width(70.0)
                        .show(ui);
                    ui.add_space(6.0);
                    TextEdit::new(&mut ev.name).width(200.0).show_full(ui);
                    ui.add_space(6.0);
                    if Button::new("Remove").ghost().show(ui).clicked {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                clip.events.remove(i);
            }
            if clip.events.is_empty() {
                Label::new("No markers on this clip.").color(dim).show(ui);
            }

            ui.add_space(6.0);
            if Button::new("Add Marker").show(ui).clicked {
                clip.events.push(AnimEventMarker {
                    time_seconds: 0.0,
                    name: "event".to_string(),
                });
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let size = Vec2::new(80.0, 26.0);
                if Button::new("Save").primary().min_size(size).show(ui).clicked {
                    action = AnimEventsAction::Save;
                }
                ui.add_space(8.0);
                if Button::new("Cancel").ghost().min_size(size).show(ui).clicked {
                    action = AnimEventsAction::Cancel;
                }
            });
        });

    action
}
