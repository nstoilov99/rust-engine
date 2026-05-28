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

        let defaults = [
            Command::new(
                "file.save_scene",
                "File: Save Scene",
                EditorAction::SaveScene,
            ),
            Command::new("file.new_scene", "File: New Scene", EditorAction::NewScene),
            Command::new("edit.undo", "Edit: Undo", EditorAction::Undo),
            Command::new("edit.redo", "Edit: Redo", EditorAction::Redo),
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
                "engine.reload_shaders",
                "Engine: Reload All Shaders",
                EditorAction::ReloadAllShaders,
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
