//! Command palette — fuzzy-search popup over editor commands.

pub mod action;

pub use action::{dispatch_action, EditorAction};

#[derive(Debug, Clone)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub keywords: Vec<String>,
    pub action: EditorAction,
}

impl Command {
    pub fn new(id: impl Into<String>, label: impl Into<String>, action: EditorAction) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            keywords: Vec::new(),
            action,
        }
    }

    pub fn with_keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let query = query.to_lowercase();
        self.label.to_lowercase().contains(&query)
            || self.id.to_lowercase().contains(&query)
            || self
                .keywords
                .iter()
                .any(|keyword| keyword.to_lowercase().contains(&query))
    }
}

#[derive(Debug, Clone)]
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        let mut registry = Self {
            commands: Vec::new(),
        };
        registry.register_defaults();
        registry
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, command: Command) {
        if let Some(existing) = self
            .commands
            .iter_mut()
            .find(|existing| existing.id == command.id)
        {
            *existing = command;
            return;
        }
        self.commands.push(command);
        self.commands
            .sort_by(|left, right| left.label.cmp(&right.label));
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn search(&self, query: &str) -> Vec<&Command> {
        self.commands
            .iter()
            .filter(|command| command.matches(query))
            .collect()
    }

    fn register_defaults(&mut self) {
        use crate::engine::editor::theme::Density;
        use crate::engine::rendering::rendering_3d::deferred::deferred_renderer::DebugView;

        let defaults = vec![
            Command::new("file.new_scene", "File: New Scene", EditorAction::NewScene)
                .with_keywords(["create", "empty"]),
            Command::new(
                "file.open_scene",
                "File: Open Scene",
                EditorAction::OpenScene,
            )
            .with_keywords(["load", "scene"]),
            Command::new(
                "file.save_scene",
                "File: Save Scene",
                EditorAction::SaveScene,
            ),
            Command::new(
                "file.save_scene_as",
                "File: Save Scene As",
                EditorAction::SaveSceneAs(None),
            ),
            Command::new("file.quit", "File: Quit", EditorAction::Quit),
            Command::new("edit.undo", "Edit: Undo", EditorAction::Undo).with_keywords(["history"]),
            Command::new("edit.redo", "Edit: Redo", EditorAction::Redo).with_keywords(["history"]),
            Command::new("edit.cut", "Edit: Cut", EditorAction::Cut),
            Command::new("edit.copy", "Edit: Copy", EditorAction::Copy),
            Command::new("edit.paste", "Edit: Paste", EditorAction::Paste),
            Command::new("edit.duplicate", "Edit: Duplicate", EditorAction::Duplicate),
            Command::new("edit.delete", "Edit: Delete", EditorAction::Delete),
            Command::new(
                "view.toggle_hierarchy",
                "View: Toggle Hierarchy",
                EditorAction::ToggleHierarchy,
            ),
            Command::new(
                "view.toggle_inspector",
                "View: Toggle Inspector",
                EditorAction::ToggleInspector,
            ),
            Command::new(
                "view.toggle_asset_browser",
                "View: Toggle Asset Browser",
                EditorAction::ToggleAssetBrowser,
            ),
            Command::new(
                "view.toggle_console",
                "View: Toggle Console",
                EditorAction::ToggleConsole,
            ),
            Command::new(
                "view.toggle_profiler",
                "View: Toggle Profiler",
                EditorAction::ToggleProfiler,
            ),
            Command::new(
                "view.reset_layout",
                "View: Reset Layout",
                EditorAction::ResetLayoutToDefault,
            ),
            Command::new(
                "view.density_compact",
                "View: Density Compact",
                EditorAction::SwitchDensity(Density::Compact),
            ),
            Command::new(
                "view.density_comfortable",
                "View: Density Comfortable",
                EditorAction::SwitchDensity(Density::Comfortable),
            ),
            Command::new(
                "view.widget_showcase",
                "View: Widget Showcase",
                EditorAction::ToggleDevShowcase,
            ),
            Command::new(
                "selection.select_all",
                "Selection: Select All",
                EditorAction::SelectAll,
            ),
            Command::new(
                "selection.deselect_all",
                "Selection: Deselect All",
                EditorAction::DeselectAll,
            ),
            Command::new(
                "selection.focus",
                "Selection: Focus Selection",
                EditorAction::FocusSelection,
            ),
            Command::new(
                "selection.find_entity",
                "Selection: Find Entity by Name",
                EditorAction::FindEntityByName,
            ),
            Command::new(
                "play.toggle",
                "Play: Toggle Play Mode",
                EditorAction::TogglePlayMode,
            ),
            Command::new(
                "play.step_frame",
                "Play: Step Frame",
                EditorAction::StepFrame,
            ),
            Command::new(
                "play.restart",
                "Play: Restart From Edit State",
                EditorAction::RestartFromEditState,
            ),
            Command::new(
                "render.debug_none",
                "Render: Debug View None",
                EditorAction::SwitchDebugView(DebugView::None),
            ),
            Command::new(
                "render.debug_position",
                "Render: Debug View Position",
                EditorAction::SwitchDebugView(DebugView::Position),
            ),
            Command::new(
                "render.debug_normal",
                "Render: Debug View Normal",
                EditorAction::SwitchDebugView(DebugView::Normal),
            ),
            Command::new(
                "render.debug_albedo",
                "Render: Debug View Albedo",
                EditorAction::SwitchDebugView(DebugView::Albedo),
            ),
            Command::new(
                "render.debug_material",
                "Render: Debug View Material",
                EditorAction::SwitchDebugView(DebugView::Material),
            ),
            Command::new(
                "render.debug_depth",
                "Render: Debug View Depth",
                EditorAction::SwitchDebugView(DebugView::Depth),
            ),
            Command::new(
                "render.toggle_wireframe",
                "Render: Toggle Wireframe",
                EditorAction::ToggleWireframe,
            ),
            Command::new(
                "render.toggle_grid",
                "Render: Toggle Grid",
                EditorAction::ToggleGrid,
            ),
            Command::new(
                "render.toggle_gizmos",
                "Render: Toggle Gizmos",
                EditorAction::ToggleGizmos,
            ),
            Command::new(
                "asset.reimport_selected",
                "Asset: Reimport Selected",
                EditorAction::ReimportSelected,
            ),
            Command::new(
                "asset.show_in_explorer",
                "Asset: Show in Explorer",
                EditorAction::ShowInExplorer,
            ),
            Command::new(
                "asset.reveal_in_browser",
                "Asset: Reveal in Asset Browser",
                EditorAction::RevealInAssetBrowser,
            ),
            Command::new(
                "engine.reload_shaders",
                "Engine: Reload All Shaders",
                EditorAction::ReloadAllShaders,
            ),
            Command::new(
                "engine.open_settings",
                "Engine: Open Settings",
                EditorAction::OpenSettings,
            ),
        ];

        for command in defaults {
            self.register(command);
        }
    }
}

#[derive(Debug, Default)]
pub struct CommandPalette {
    pub open: bool,
    query: String,
    selected: usize,
}

impl CommandPalette {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        registry: &CommandRegistry,
    ) -> Option<EditorAction> {
        if !self.open {
            return None;
        }

        let mut selected_action = None;
        let mut should_close = false;
        egui::Window::new("Command Palette")
            .id(egui::Id::new("command_palette"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search commands")
                        .desired_width(420.0),
                );
                response.request_focus();

                let matches = registry.search(&self.query);
                if self.selected >= matches.len() {
                    self.selected = matches.len().saturating_sub(1);
                }

                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    should_close = true;
                }
                if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
                    self.selected = (self.selected + 1).min(matches.len().saturating_sub(1));
                }
                if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
                    self.selected = self.selected.saturating_sub(1);
                }
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    if let Some(command) = matches.get(self.selected) {
                        selected_action = Some(command.action.clone());
                        should_close = true;
                    }
                }

                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for (index, command) in matches.iter().enumerate() {
                            let selected = index == self.selected;
                            if ui
                                .selectable_label(selected, command.label.as_str())
                                .clicked()
                            {
                                selected_action = Some(command.action.clone());
                                should_close = true;
                            }
                        }
                    });
            });

        if should_close {
            self.close();
        }
        selected_action
    }
}
