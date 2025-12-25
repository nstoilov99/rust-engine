//! Console log system with color-coded messages and filtering

use egui::{Color32, RichText};

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

    /// Count messages by level
    pub fn count_by_level(messages: &[LogMessage]) -> (usize, usize, usize) {
        let mut info = 0;
        let mut warn = 0;
        let mut error = 0;
        for msg in messages {
            match msg.level {
                LogLevel::Info => info += 1,
                LogLevel::Warning => warn += 1,
                LogLevel::Error => error += 1,
            }
        }
        (info, warn, error)
    }
}
