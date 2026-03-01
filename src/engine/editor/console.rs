//! Console log system with color-coded messages and filtering

use egui::{Color32, RichText};
use std::collections::VecDeque;

pub const MAX_CONSOLE_MESSAGES: usize = 2000;

/// Log level for console messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

impl LogLevel {
    /// Get the color for this log level
    pub fn color(&self) -> Color32 {
        match self {
            LogLevel::Info => Color32::from_gray(200),
            LogLevel::Warning => Color32::from_rgb(255, 200, 100),
            LogLevel::Error => Color32::from_rgb(255, 100, 100),
        }
    }

    /// Get a prefix string for this log level
    pub fn prefix(&self) -> &'static str {
        match self {
            LogLevel::Info => "[INFO]",
            LogLevel::Warning => "[WARN]",
            LogLevel::Error => "[ERROR]",
        }
    }
}

/// A console log message with level and text
#[derive(Debug, Clone)]
pub struct LogMessage {
    pub level: LogLevel,
    pub text: String,
}

impl LogMessage {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            text: text.into(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Warning,
            text: text.into(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Error,
            text: text.into(),
        }
    }

    /// Format the message with level prefix and color
    pub fn rich_text(&self) -> RichText {
        RichText::new(format!("{} {}", self.level.prefix(), self.text))
            .color(self.level.color())
    }
}

/// Capped console log with incrementally maintained per-level counts.
pub struct ConsoleLog {
    messages: VecDeque<LogMessage>,
    info_count: usize,
    warn_count: usize,
    error_count: usize,
}

impl ConsoleLog {
    pub fn new() -> Self {
        Self {
            messages: VecDeque::with_capacity(MAX_CONSOLE_MESSAGES),
            info_count: 0,
            warn_count: 0,
            error_count: 0,
        }
    }

    pub fn push(&mut self, msg: LogMessage) {
        self.increment(&msg);
        self.messages.push_back(msg);
        while self.messages.len() > MAX_CONSOLE_MESSAGES {
            if let Some(evicted) = self.messages.pop_front() {
                self.decrement(&evicted);
            }
        }
    }

    pub fn extend(&mut self, msgs: impl IntoIterator<Item = LogMessage>) {
        for msg in msgs {
            self.push(msg);
        }
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.info_count = 0;
        self.warn_count = 0;
        self.error_count = 0;
    }

    pub fn iter(&self) -> impl Iterator<Item = &LogMessage> {
        self.messages.iter()
    }

    /// Pre-computed counts — no per-frame scanning required.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.info_count, self.warn_count, self.error_count)
    }

    fn increment(&mut self, msg: &LogMessage) {
        match msg.level {
            LogLevel::Info => self.info_count += 1,
            LogLevel::Warning => self.warn_count += 1,
            LogLevel::Error => self.error_count += 1,
        }
    }

    fn decrement(&mut self, msg: &LogMessage) {
        match msg.level {
            LogLevel::Info => self.info_count = self.info_count.saturating_sub(1),
            LogLevel::Warning => self.warn_count = self.warn_count.saturating_sub(1),
            LogLevel::Error => self.error_count = self.error_count.saturating_sub(1),
        }
    }
}

impl Default for ConsoleLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Filter settings for console log display
#[derive(Debug, Clone)]
pub struct LogFilter {
    pub show_info: bool,
    pub show_warning: bool,
    pub show_error: bool,
}

impl Default for LogFilter {
    fn default() -> Self {
        Self {
            show_info: true,
            show_warning: true,
            show_error: true,
        }
    }
}

impl LogFilter {
    /// Check if a message should be shown based on filter settings
    pub fn should_show(&self, message: &LogMessage) -> bool {
        match message.level {
            LogLevel::Info => self.show_info,
            LogLevel::Warning => self.show_warning,
            LogLevel::Error => self.show_error,
        }
    }
}
