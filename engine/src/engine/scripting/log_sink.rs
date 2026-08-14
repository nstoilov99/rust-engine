//! Where a graph's `Print` goes when nothing is listening to stdout.
//!
//! `Effect::Log` used to be a bare `println!`. In a shipped game that is the
//! right answer — there is a terminal, and the line is on it. In the editor
//! there is neither: `game_client` installs no logger and the Console panel
//! reads a [`ConsoleLog`](crate::engine::editor::ConsoleLog), not the
//! process's stdout. So the one node whose entire purpose is "tell me what
//! happened" told nobody, which reads as "my graph does nothing" — the single
//! most misleading diagnosis a runtime can offer (D6 asks for exactly the
//! opposite: "Print output goes to the console panel tagged with the graph
//! path").
//!
//! This is the smallest thing that fixes it: a bounded queue the runner pushes
//! into and the editor drains once a frame. Deliberately **not** a logger —
//! installing one would put every `log::` call in every dependency into the
//! author's Console, which is a different feature with a different design.
//!
//! ## Bounded, and who inserts it
//!
//! The sink is a plain resource with a hard cap ([`GRAPH_LOG_CAP`]); past it
//! the oldest entry is dropped and counted, so a graph printing every tick in
//! a paused editor cannot grow the process. **Only the editor inserts it**
//! (see the `graph_scripting` plugin): the plugin is compiled into both
//! binaries, but a shipped game has no console to drain into, and a queue
//! nobody reads is an allocation per print for nothing. With no resource the
//! runner's push is one failed `Resources` lookup and the `println!` is
//! untouched — standalone's stdout is exactly what it was.

use std::collections::VecDeque;

use node_graph_exec::LogLevel;

/// How many lines the sink holds before dropping the oldest. One frame of a
/// runaway graph is thousands of lines; the editor's own console keeps 2000,
/// so buffering more than this between drains would only delay a loss the
/// console is about to take anyway.
pub const GRAPH_LOG_CAP: usize = 1024;

/// One line a graph produced, still structured: the console decides how to
/// render it, and the level becomes a severity rather than text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLogEntry {
    pub level: LogLevel,
    /// Content-relative `.graph` path — which asset said this.
    pub graph: String,
    /// The entity's `Name`, or its debug id when it has none.
    pub entity: String,
    pub text: String,
}

impl GraphLogEntry {
    pub fn new(level: LogLevel, graph: &str, entity: &str, text: impl Into<String>) -> Self {
        Self {
            level,
            graph: graph.to_string(),
            entity: entity.to_string(),
            text: text.into(),
        }
    }

    /// The console line. The tag is the P7 spelling — `[graph X on Y]` — kept
    /// identical to the `println!` so a line read in a terminal and the same
    /// line read in the panel are recognizably the same line. The level is
    /// *not* in the text: it becomes the row's severity, which is what colours
    /// it and what the console's filter counts.
    pub fn line(&self) -> String {
        format!("[graph {} on {}] {}", self.graph, self.entity, self.text)
    }
}

/// The queue itself. A `Resource`, inserted by the editor's plugin build.
#[derive(Debug, Default)]
pub struct GraphLogSink {
    entries: VecDeque<GraphLogEntry>,
    /// Lines the cap threw away since the last drain, so the loss is reported
    /// rather than silently swallowed.
    dropped: u64,
}

impl GraphLogSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, entry: GraphLogEntry) {
        if self.entries.len() >= GRAPH_LOG_CAP {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(entry);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &GraphLogEntry> {
        self.entries.iter()
    }

    /// Take everything queued, leaving the sink empty.
    pub fn drain(&mut self) -> std::collections::vec_deque::Drain<'_, GraphLogEntry> {
        self.entries.drain(..)
    }

    /// How many lines the cap dropped, resetting the count. Reported once by
    /// the drain rather than accumulated forever.
    pub fn take_dropped(&mut self) -> u64 {
        std::mem::take(&mut self.dropped)
    }
}

/// Move everything the runner queued into the editor's console.
///
/// A free function taking the log rather than a method on either side: the
/// sink is engine-runtime state and the console is editor UI state, and
/// neither should own a reference to the other. `game_client` calls this once
/// a frame, right after the schedule runs, with
/// `&mut self.editor.console.messages`.
#[cfg(feature = "editor")]
pub fn drain_into_console(
    sink: &mut GraphLogSink,
    console: &mut crate::engine::editor::ConsoleLog,
) {
    use crate::engine::editor::LogMessage;

    // Ahead of the survivors: what was dropped is older than what is left.
    let dropped = sink.take_dropped();
    if dropped > 0 {
        console.push(LogMessage::warning(format!(
            "[graph] {dropped} log line(s) dropped — more than {GRAPH_LOG_CAP} printed between frames"
        )));
    }
    for entry in sink.drain() {
        let line = entry.line();
        console.push(match entry.level {
            LogLevel::Info => LogMessage::info(line),
            LogLevel::Warning => LogMessage::warning(line),
            LogLevel::Error => LogMessage::error(line),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_drops_the_oldest_and_counts_it() {
        let mut sink = GraphLogSink::new();
        for i in 0..(GRAPH_LOG_CAP + 5) {
            sink.push(GraphLogEntry::new(
                LogLevel::Info,
                "graphs/t.graph",
                "Duck",
                format!("{i}"),
            ));
        }
        assert_eq!(sink.len(), GRAPH_LOG_CAP, "bounded, whatever a graph does");
        assert_eq!(sink.take_dropped(), 5);
        assert_eq!(sink.take_dropped(), 0, "the count resets when it is taken");
        assert_eq!(
            sink.iter().next().unwrap().text,
            "5",
            "the *oldest* went, so the newest lines survive"
        );
    }

    #[test]
    fn the_tag_is_the_p7_spelling() {
        let e = GraphLogEntry::new(LogLevel::Warning, "graphs/t.graph", "Duck", "hello");
        assert_eq!(e.line(), "[graph graphs/t.graph on Duck] hello");
    }

    /// Levels map to console severities, and the drain empties the sink.
    #[cfg(feature = "editor")]
    #[test]
    fn draining_maps_levels_and_empties_the_sink() {
        use crate::engine::editor::{ConsoleLog, LogLevel as CLevel};

        let mut sink = GraphLogSink::new();
        for (level, text) in [
            (LogLevel::Info, "i"),
            (LogLevel::Warning, "w"),
            (LogLevel::Error, "e"),
        ] {
            sink.push(GraphLogEntry::new(level, "graphs/t.graph", "Duck", text));
        }

        let mut console = ConsoleLog::new();
        drain_into_console(&mut sink, &mut console);

        assert!(sink.is_empty(), "drained, not copied");
        let rows: Vec<(CLevel, String)> = console
            .iter()
            .map(|m| (m.level, m.text.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                (CLevel::Info, "[graph graphs/t.graph on Duck] i".to_string()),
                (CLevel::Warning, "[graph graphs/t.graph on Duck] w".to_string()),
                (CLevel::Error, "[graph graphs/t.graph on Duck] e".to_string()),
            ],
            "order preserved, level becomes severity, the tag survives"
        );

        // A second drain adds nothing.
        drain_into_console(&mut sink, &mut console);
        assert_eq!(console.iter().count(), 3);
    }

    /// An overflow announces itself in the console rather than vanishing.
    #[cfg(feature = "editor")]
    #[test]
    fn an_overflow_is_reported_to_the_console() {
        use crate::engine::editor::{ConsoleLog, LogLevel as CLevel};

        let mut sink = GraphLogSink::new();
        for i in 0..(GRAPH_LOG_CAP + 2) {
            sink.push(GraphLogEntry::new(
                LogLevel::Info,
                "graphs/t.graph",
                "Duck",
                format!("{i}"),
            ));
        }
        let mut console = ConsoleLog::new();
        drain_into_console(&mut sink, &mut console);
        let first = console.iter().next().unwrap();
        assert_eq!(first.level, CLevel::Warning);
        assert!(first.text.contains("2 log line(s) dropped"), "{}", first.text);
    }
}
