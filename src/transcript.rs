//! Format chat / session transcripts for export to the clipboard.
//!
//! Output shape is markdown — lands cleanly in Slack, Discord,
//! GitHub PRs, and plain-text editors. Each turn gets a role
//! header followed by the body verbatim; we don't try to re-indent
//! assistant code blocks (they already arrive fence-wrapped from
//! the gateway).
//!
//! Pure data → data, so the hard bits (empty transcripts, role
//! ordering, whitespace) are unit-testable without instantiating
//! an Iced widget.

use std::time::SystemTime;

use crate::ui::chat_view::{ChatMessage, ChatRole};

/// Produce a markdown rendering of a transcript. The `title` is
/// rendered as an `# H1` at the top so the clipboard paste starts
/// with context ("what am I looking at?").
pub fn to_markdown(title: &str, messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    let trimmed_title = title.trim();
    if !trimmed_title.is_empty() {
        out.push_str("# ");
        out.push_str(trimmed_title);
        out.push_str("\n\n");
    }
    if messages.is_empty() {
        out.push_str("_(empty transcript)_\n");
        return out;
    }
    for (i, msg) in messages.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str("## ");
        out.push_str(role_label(msg.role));
        if let Some(ts) = format_timestamp(msg.at) {
            out.push_str(" — ");
            out.push_str(&ts);
        }
        out.push('\n');
        out.push('\n');
        // Message body verbatim — trim trailing whitespace to keep
        // paste clean; don't trim leading whitespace because
        // assistants sometimes open with intentional code indent.
        out.push_str(msg.text.trim_end());
    }
    out.push('\n');
    out
}

fn role_label(role: ChatRole) -> &'static str {
    match role {
        ChatRole::User => "you",
        ChatRole::Assistant => "agent",
        ChatRole::Other => "system",
    }
}

/// Render a SystemTime as a compact "HH:MM:SS" local-ish timestamp
/// using stdlib only (no chrono). Exact format:
/// `YYYY-MM-DDThh:mm:ssZ` (UTC). Good enough for paste context;
/// operators who need precision can export the raw timestamps from
/// the gateway.
fn format_timestamp(at: SystemTime) -> Option<String> {
    let dur = at.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    let secs = dur.as_secs();
    Some(format_unix_ts(secs))
}

/// Format a UNIX timestamp (seconds) as `YYYY-MM-DDThh:mm:ssZ`.
/// Correct Gregorian math, no time-zone smarts (always UTC).
fn format_unix_ts(secs: u64) -> String {
    let (year, month, day, hour, minute, second) = unix_to_ymdhms(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert a UNIX timestamp into (year, month, day, hour, min, sec)
/// in UTC. Uses the classic Howard Hinnant civil-from-days routine —
/// correct for all non-negative timestamps through year 9999.
fn unix_to_ymdhms(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let time_of_day = secs % 86_400;
    let hour = (time_of_day / 3600) as u32;
    let minute = ((time_of_day % 3600) / 60) as u32;
    let second = (time_of_day % 60) as u32;

    // Hinnant: civil_from_days — shift so the reference year is 0000-03-01.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y } as i32;
    (year, m as u32, d as u32, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mk(role: ChatRole, text: &str, epoch_secs: u64) -> ChatMessage {
        ChatMessage {
            role,
            text: text.to_string(),
            at: SystemTime::UNIX_EPOCH + Duration::from_secs(epoch_secs),
        }
    }

    #[test]
    fn empty_transcript_renders_placeholder() {
        let out = to_markdown("Sebastian 🦀", &[]);
        assert!(out.contains("# Sebastian 🦀"));
        assert!(out.contains("_(empty transcript)_"));
    }

    #[test]
    fn blank_title_omits_header() {
        let out = to_markdown("   ", &[mk(ChatRole::User, "hi", 0)]);
        // The transcript opens straight with the first role header
        // (`## you`) rather than a bare H1 title line. `starts_with("# ")`
        // is the specific "H1 with title" check — an H2 begins
        // with `##` (no space at position 1), so it passes cleanly.
        assert!(!out.starts_with("# "), "unexpected H1 header: {out:?}");
        assert!(out.starts_with("## you"));
    }

    #[test]
    fn role_labels_map_cleanly() {
        let msgs = vec![
            mk(ChatRole::User, "hi", 1_700_000_000),
            mk(ChatRole::Assistant, "hello", 1_700_000_005),
            mk(ChatRole::Other, "system note", 1_700_000_010),
        ];
        let out = to_markdown("Session", &msgs);
        assert!(out.contains("## you"));
        assert!(out.contains("## agent"));
        assert!(out.contains("## system"));
    }

    #[test]
    fn messages_separated_by_blank_lines() {
        let msgs = vec![
            mk(ChatRole::User, "one", 1_700_000_000),
            mk(ChatRole::Assistant, "two", 1_700_000_005),
        ];
        let out = to_markdown("", &msgs);
        // Two messages → exactly one `\n\n` between them before the
        // second role header.
        assert_eq!(out.matches("\n\n## ").count(), 1);
    }

    #[test]
    fn body_trailing_whitespace_trimmed_but_leading_preserved() {
        let msgs = vec![mk(
            ChatRole::Assistant,
            "    indented code line\n\n\n",
            1_700_000_000,
        )];
        let out = to_markdown("", &msgs);
        assert!(
            out.contains("    indented code line"),
            "leading indent should survive: {out:?}",
        );
        assert!(!out.ends_with("\n\n\n"), "trailing blanks should collapse");
    }

    #[test]
    fn timestamp_formats_to_iso_utc() {
        // 2024-01-02T03:04:05Z → 1704164645
        assert_eq!(format_unix_ts(1_704_164_645), "2024-01-02T03:04:05Z");
    }

    #[test]
    fn timestamp_handles_leap_year() {
        // 2024-02-29T12:00:00Z → 1709208000
        assert_eq!(format_unix_ts(1_709_208_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn timestamp_handles_epoch() {
        assert_eq!(format_unix_ts(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn timestamp_handles_century_boundary() {
        // 2000-03-01T00:00:00Z → 951868800
        assert_eq!(format_unix_ts(951_868_800), "2000-03-01T00:00:00Z");
    }

    #[test]
    fn markdown_output_shape() {
        let msgs = vec![
            mk(ChatRole::User, "run ls", 1_704_164_645),
            mk(
                ChatRole::Assistant,
                "Files:\n\n- a.txt\n- b.txt",
                1_704_164_650,
            ),
        ];
        let out = to_markdown("Sebastian 🦀", &msgs);
        let expected = "\
# Sebastian 🦀

## you — 2024-01-02T03:04:05Z

run ls

## agent — 2024-01-02T03:04:10Z

Files:

- a.txt
- b.txt
";
        assert_eq!(out, expected);
    }
}
