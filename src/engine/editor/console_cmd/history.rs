//! Command input history for recall with up/down arrows

/// Command input history for recall with up/down arrows
pub struct InputHistory {
    /// Past inputs (oldest first)
    entries: Vec<String>,
    /// Current position when navigating (-1 = not navigating)
    position: isize,
    /// Maximum entries to keep
    max_entries: usize,
    /// Temporary buffer for current incomplete input
    temp_input: String,
}

impl InputHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            position: -1,
            max_entries,
            temp_input: String::new(),
        }
    }

    /// Add an entry to history (called when command is executed)
    pub fn push(&mut self, input: String) {
        // Don't add duplicates of the last entry or empty strings
        if self.entries.last() != Some(&input) && !input.is_empty() {
            self.entries.push(input);
            if self.entries.len() > self.max_entries {
                self.entries.remove(0);
            }
        }
        self.reset_navigation();
    }

    /// Navigate to previous entry (up arrow)
    /// Returns the entry to display, or None if at the start
    pub fn previous(&mut self, current_input: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }

        if self.position == -1 {
            // Starting navigation - save current input
            self.temp_input = current_input.to_string();
            self.position = self.entries.len() as isize;
        }

        if self.position > 0 {
            self.position -= 1;
            return Some(&self.entries[self.position as usize]);
        }

        // Already at oldest
        self.entries.first().map(|s| s.as_str())
    }

    /// Navigate to next entry (down arrow)
    /// Returns the entry to display, or None if back at current input
    pub fn next(&mut self) -> Option<&str> {
        if self.position == -1 {
            return None;
        }

        self.position += 1;

        if self.position >= self.entries.len() as isize {
            // Back to current incomplete input
            self.position = -1;
            return Some(&self.temp_input);
        }

        Some(&self.entries[self.position as usize])
    }

    /// Reset navigation state
    pub fn reset_navigation(&mut self) {
        self.position = -1;
        self.temp_input.clear();
    }

    /// Check if currently navigating history
    pub fn is_navigating(&self) -> bool {
        self.position != -1
    }
}

impl Default for InputHistory {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history_navigation() {
        let mut history = InputHistory::new(10);
        history.push("first".to_string());
        history.push("second".to_string());
        history.push("third".to_string());

        // Navigate up through history
        assert_eq!(history.previous("current"), Some("third"));
        assert_eq!(history.previous("current"), Some("second"));
        assert_eq!(history.previous("current"), Some("first"));
        assert_eq!(history.previous("current"), Some("first")); // stays at oldest

        // Navigate back down
        assert_eq!(history.next(), Some("second"));
        assert_eq!(history.next(), Some("third"));
        assert_eq!(history.next(), Some("current")); // back to current
    }

    #[test]
    fn test_no_duplicate_last() {
        let mut history = InputHistory::new(10);
        history.push("same".to_string());
        history.push("same".to_string());
        assert_eq!(history.entries.len(), 1);
    }
}
