//! Generic search popup with fuzzy matching.

use egui::Ui;

/// Trait for items searchable via `SearchPopup`.
pub trait SearchItem {
    /// Text to match against when searching.
    fn match_text(&self) -> &str;
    /// Render this item as a row in the popup.
    fn display(&self, ui: &mut Ui);
}

/// Generic fuzzy-search popup over a set of items.
pub struct SearchPopup<Item: SearchItem> {
    items: Vec<Item>,
    query: String,
    selected_index: usize,
}

impl<Item: SearchItem> SearchPopup<Item> {
    /// Create a new search popup with the given items.
    pub fn new(items: Vec<Item>) -> Self {
        Self {
            items,
            query: String::new(),
            selected_index: 0,
        }
    }

    /// Replace the item set (e.g., on data refresh).
    pub fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.selected_index = 0;
    }

    /// Get the current query.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query.
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.selected_index = 0;
    }

    /// Render the popup. Returns a reference to the picked item if Enter was pressed.
    pub fn show(&mut self, ui: &mut Ui) -> Option<&Item> {
        // Search input
        let text_response = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Type to search…")
                .desired_width(ui.available_width()),
        );

        // Auto-focus the search field
        if text_response.gained_focus() || ui.memory(|m| m.has_focus(text_response.id)) {
            // Already focused
        } else {
            text_response.request_focus();
        }

        // Filter + rank items
        let filtered = self.filtered_indices();

        // Clamp selected index
        if !filtered.is_empty() {
            self.selected_index = self.selected_index.min(filtered.len() - 1);
        } else {
            self.selected_index = 0;
        }

        // Keyboard navigation
        let mut picked = None;
        ui.input(|input| {
            if input.key_pressed(egui::Key::ArrowDown) && !filtered.is_empty() {
                self.selected_index = (self.selected_index + 1).min(filtered.len() - 1);
            }
            if input.key_pressed(egui::Key::ArrowUp) && self.selected_index > 0 {
                self.selected_index -= 1;
            }
            if input.key_pressed(egui::Key::Enter) && !filtered.is_empty() {
                picked = Some(filtered[self.selected_index]);
            }
        });

        // Render list
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (list_idx, &item_idx) in filtered.iter().enumerate() {
                    let selected = list_idx == self.selected_index;
                    let item = &self.items[item_idx];

                    let response = ui.selectable_label(selected, "");
                    let rect = response.rect;

                    // Render item content over the selectable
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                        item.display(ui);
                    });

                    if response.clicked() {
                        picked = Some(item_idx);
                    }
                }
            });

        picked.map(|idx| &self.items[idx])
    }

    /// Return indices of items matching the query, ranked by score.
    fn filtered_indices(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.items.len()).collect();
        }

        let query_lower = self.query.to_lowercase();
        let mut scored: Vec<(usize, i32)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let text = item.match_text().to_lowercase();
                fuzzy_score(&query_lower, &text).map(|score| (i, score))
            })
            .collect();

        // Higher score = better match
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().map(|(i, _)| i).collect()
    }
}

/// Simple subsequence fuzzy matcher with scoring bonuses.
///
/// Returns `Some(score)` if `query` is a subsequence of `text`, `None` otherwise.
/// Bonuses for:
/// - Exact prefix match
/// - Consecutive character matches
/// - Match at word boundary
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let query_chars: Vec<char> = query.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    let mut qi = 0;
    let mut score: i32 = 0;
    let mut prev_match_idx: Option<usize> = None;

    for (ti, &tc) in text_chars.iter().enumerate() {
        if qi < query_chars.len() && tc == query_chars[qi] {
            // Match found
            score += 1;

            // Bonus: consecutive match
            if let Some(prev) = prev_match_idx {
                if ti == prev + 1 {
                    score += 3;
                }
            }

            // Bonus: prefix match
            if qi == 0 && ti == 0 {
                score += 5;
            }

            // Bonus: word boundary (after space, underscore, or case change)
            if ti > 0 {
                let prev_char = text_chars[ti - 1];
                if prev_char == ' ' || prev_char == '_' || prev_char == '/' {
                    score += 2;
                }
            }

            prev_match_idx = Some(ti);
            qi += 1;
        }
    }

    if qi == query_chars.len() {
        Some(score)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_exact_prefix_scores_highest() {
        let exact = fuzzy_score("save", "save scene").unwrap();
        let partial = fuzzy_score("save", "a save").unwrap();
        assert!(exact > partial, "exact={exact}, partial={partial}");
    }

    #[test]
    fn fuzzy_subsequence_matches() {
        assert!(fuzzy_score("sv", "save").is_some());
        assert!(fuzzy_score("sz", "save").is_none());
    }

    #[test]
    fn fuzzy_empty_query_matches_all() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }
}
