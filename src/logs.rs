//! Classified log buffer + severity parsing for the Logs tab.
//!
//! The gateway streams plain text lines in via `logs.tail`; we
//! classify each line's severity at ingest time so the view can
//! color-code and filter without re-parsing on every redraw.
//!
//! Severity parsing is a heuristic keyed to `tracing`-style output
//! (the format OpenClaw's gateway emits): the level token (`ERROR`,
//! `WARN`, `INFO`, `DEBUG`, `TRACE`) appears either flanked by
//! whitespace or at the very start of the line after a timestamp.
//! Gateway-side format changes can flip this into `Info` for
//! everything, at which point the chips become a no-op — recoverable,
//! not catastrophic.

use std::collections::VecDeque;

/// Maximum number of classified lines retained. Matches the previous
/// raw-string buffer cap so memory budget doesn't change.
pub const LOG_BUFFER_MAX: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogSeverity {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogSeverity {
    pub const ALL: [LogSeverity; 4] = [
        LogSeverity::Error,
        LogSeverity::Warn,
        LogSeverity::Info,
        LogSeverity::Debug,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LogSeverity::Error => "ERROR",
            LogSeverity::Warn => "WARN",
            LogSeverity::Info => "INFO",
            LogSeverity::Debug => "DEBUG",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub severity: LogSeverity,
    pub text: String,
}

impl LogLine {
    /// Build a classified line from a raw `logs.tail` string.
    pub fn classify(text: String) -> Self {
        let severity = parse_severity(&text);
        Self { severity, text }
    }
}

/// Classify a raw log line. Strategy: scan the first ~80 bytes for
/// any level-shaped token (flanked by non-alpha boundaries) and take
/// the **first** one found. `tracing` format always emits the level
/// right after the timestamp, so earliest-token wins reliably reads
/// the actual level — even if the message body later mentions another
/// level keyword. Info is the fallback when no token matches.
pub fn parse_severity(line: &str) -> LogSeverity {
    // Slice at a char boundary, NOT a raw byte index — an emoji or
    // any multi-byte char straddling byte 80 would otherwise panic
    // the whole app the first time a log line contained it.
    let head = head_bytes(line, 80);
    let candidates = [
        ("ERROR", LogSeverity::Error),
        ("WARN", LogSeverity::Warn),
        ("INFO", LogSeverity::Info),
        ("DEBUG", LogSeverity::Debug),
        ("TRACE", LogSeverity::Debug),
    ];
    let mut best: Option<(usize, LogSeverity)> = None;
    for (needle, sev) in candidates {
        if let Some(pos) = find_token(head, needle)
            && best.is_none_or(|(p, _)| pos < p)
        {
            best = Some((pos, sev));
        }
    }
    best.map(|(_, s)| s).unwrap_or(LogSeverity::Info)
}

/// Clamp a string slice to at most `max` bytes, snapping back to
/// the nearest UTF-8 char boundary when `max` falls inside a
/// multi-byte code point. Native Rust slice syntax (`&s[..max]`)
/// panics on non-boundary indices — this is the panic-free form.
fn head_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Find the first occurrence of `needle` in `hay` where the match is
/// flanked by non-alphabetic boundaries on both sides — so "info"
/// inside "information" is not a match, and " INFO " or "ERROR:" are.
/// Scans linearly past false positives (needle inside a larger word)
/// until a valid boundary is found.
fn find_token(hay: &str, needle: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(rel) = hay[start..].find(needle) {
        let idx = start + rel;
        let before = hay[..idx].chars().next_back();
        let after = hay[idx + needle.len()..].chars().next();
        let alpha = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphabetic());
        if !alpha(before) && !alpha(after) {
            return Some(idx);
        }
        start = idx + 1;
    }
    None
}

/// UI filter state for the Logs tab. `show_*` toggle visibility of a
/// severity; `search` is the (lower-cased) substring operator typed
/// into the search box (empty = no filter).
#[derive(Debug, Clone)]
pub struct LogFilters {
    pub show_error: bool,
    pub show_warn: bool,
    pub show_info: bool,
    pub show_debug: bool,
    /// Raw search text (as typed). Case-insensitive match is done at
    /// render time by lowercasing both sides — keeps the input
    /// widget's echo faithful to what the operator typed.
    pub search: String,
}

impl Default for LogFilters {
    fn default() -> Self {
        Self {
            show_error: true,
            show_warn: true,
            show_info: true,
            show_debug: true,
            search: String::new(),
        }
    }
}

impl LogFilters {
    pub fn shows(&self, severity: LogSeverity) -> bool {
        match severity {
            LogSeverity::Error => self.show_error,
            LogSeverity::Warn => self.show_warn,
            LogSeverity::Info => self.show_info,
            LogSeverity::Debug => self.show_debug,
        }
    }

    pub fn toggle(&mut self, severity: LogSeverity) {
        match severity {
            LogSeverity::Error => self.show_error = !self.show_error,
            LogSeverity::Warn => self.show_warn = !self.show_warn,
            LogSeverity::Info => self.show_info = !self.show_info,
            LogSeverity::Debug => self.show_debug = !self.show_debug,
        }
    }

    /// True when a line should be rendered under the current filters.
    /// Severity chip gates first, then search text.
    pub fn matches(&self, line: &LogLine) -> bool {
        if !self.shows(line.severity) {
            return false;
        }
        if self.search.is_empty() {
            return true;
        }
        let needle = self.search.to_lowercase();
        line.text.to_lowercase().contains(&needle)
    }
}

/// Append lines into a classified ring buffer, evicting oldest when
/// full. Centralizes the cap so the ingest path in `app.rs` doesn't
/// repeat the bound literal.
pub fn push_line(buf: &mut VecDeque<LogLine>, line: LogLine) {
    if buf.len() >= LOG_BUFFER_MAX {
        buf.pop_front();
    }
    buf.push_back(line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracing_style_levels() {
        assert_eq!(
            parse_severity("2026-04-19T12:00:00Z  INFO my::mod: hello"),
            LogSeverity::Info,
        );
        assert_eq!(
            parse_severity("2026-04-19T12:00:00Z  WARN my::mod: uh"),
            LogSeverity::Warn,
        );
        assert_eq!(
            parse_severity("2026-04-19T12:00:00Z ERROR my::mod: boom"),
            LogSeverity::Error,
        );
        assert_eq!(
            parse_severity("2026-04-19T12:00:00Z DEBUG my::mod: trace"),
            LogSeverity::Debug,
        );
    }

    #[test]
    fn substrings_do_not_match_tokens() {
        // "information" contains "INFO" but not as a standalone token.
        // Still matches because the next char is lowercase alpha — the
        // token boundary check rejects it. Line falls back to Info
        // (default), which is what we want.
        assert_eq!(parse_severity("xx information about"), LogSeverity::Info);
        // "WARNING" is not accepted — the gateway emits "WARN", not
        // "WARNING", so a "WARNING: ..." message is really user text.
        assert_eq!(
            parse_severity("12:00 WARNING: user wrote this"),
            LogSeverity::Info,
        );
    }

    #[test]
    fn earliest_token_wins_even_when_body_mentions_error() {
        // Level is INFO right after the timestamp; the body later
        // mentions ERROR. The "earliest token wins" rule keeps the
        // classification at Info — a plain info-line complaining
        // about a remote failure shouldn't flip the whole line red.
        let line = "2026-04-19T12:00:00Z  INFO periclaw::net: got ERROR from peer";
        assert_eq!(parse_severity(line), LogSeverity::Info);
    }

    #[test]
    fn handles_multibyte_chars_at_head_boundary() {
        // Regression: the naive `&line[..80]` slice would panic
        // when byte 80 lands in the middle of a multi-byte char.
        // "🚀" is 4 bytes; placing it so that byte 78-81 hosts the
        // emoji puts the cut inside it.
        let prefix = "x".repeat(78);
        let line = format!("{prefix}🚀 WARN tail");
        assert_eq!(parse_severity(&line), LogSeverity::Info);
    }

    #[test]
    fn head_bytes_snaps_to_char_boundary() {
        // "é" is 2 bytes in UTF-8. Asking for 3 bytes of "eéx"
        // should return "eé" (3 bytes), not panic at 2.
        assert_eq!(head_bytes("eéx", 3), "eé");
        // Asking for a byte count inside é snaps back to before it.
        assert_eq!(head_bytes("eéx", 2), "e");
        // All-ASCII passes through untouched at the boundary.
        assert_eq!(head_bytes("abcdef", 3), "abc");
    }

    #[test]
    fn long_prefix_beyond_head_window_is_ignored() {
        // Any level mention past the first 80 bytes is simply not
        // scanned — this is the safety net for really verbose
        // prefixes (e.g. deeply-nested module paths).
        let line = format!("{}  INFO periclaw::net: real content", "x".repeat(80),);
        assert_eq!(parse_severity(&line), LogSeverity::Info);
    }

    #[test]
    fn filters_default_show_everything() {
        let f = LogFilters::default();
        for sev in LogSeverity::ALL {
            assert!(f.shows(sev));
        }
    }

    #[test]
    fn filter_search_is_case_insensitive() {
        let f = LogFilters {
            search: "SCOPE".to_string(),
            ..LogFilters::default()
        };
        let line = LogLine::classify("waiting for scope-upgrade".to_string());
        assert!(f.matches(&line));
    }

    #[test]
    fn severity_toggle_round_trips() {
        let mut f = LogFilters::default();
        f.toggle(LogSeverity::Warn);
        assert!(!f.show_warn);
        f.toggle(LogSeverity::Warn);
        assert!(f.show_warn);
    }

    #[test]
    fn push_line_caps_at_max() {
        let mut buf = VecDeque::new();
        for i in 0..(LOG_BUFFER_MAX + 50) {
            push_line(&mut buf, LogLine::classify(format!("line {i}")));
        }
        assert_eq!(buf.len(), LOG_BUFFER_MAX);
        // Oldest lines evicted — front should be the 51st-inserted
        // line (original indices 0..49 were popped).
        assert_eq!(buf.front().unwrap().text, "line 50");
    }
}
