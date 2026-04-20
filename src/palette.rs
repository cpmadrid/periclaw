//! Command palette — action catalog, fuzzy scorer, entry builder.
//!
//! The palette is the one-place keyboard-driven surface for the app:
//! switch tabs, run a cron, open a session, start a chat with a
//! specific agent. Opened with ⌘K (macOS) / Ctrl+K (elsewhere).
//!
//! Keeping the catalog, scoring, and entry shaping here lets the UI
//! module (`ui/palette.rs`) stay focused on rendering — this module
//! is pure data + functions, which means we can unit-test the hard
//! parts (fuzzy scoring, entry composition) without spinning up an
//! Iced `Canvas`.

use std::collections::HashMap;

use crate::app::NavItem;
use crate::domain::{AgentId, AgentKind};
use crate::net::rpc::{AgentInfo, CronState, SessionInfo};

/// A thing the operator can invoke via the palette. Each maps 1:1
/// to an existing `Message` variant the app already handles, so
/// dispatch is a single match arm in the Execute handler.
#[derive(Debug, Clone)]
pub enum PaletteAction {
    Nav(NavItem),
    RunCron(AgentId),
    ChatWithAgent(AgentId),
    OpenSession(String),
    ResetMainSession(AgentId),
}

/// Grouping for rendering. Entries within a group stay together in
/// the rendered list when the query is empty; once the operator
/// starts typing, the group ordering gives way to score ordering
/// (which reshuffles across groups).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaletteGroup {
    Navigation,
    Chat,
    Crons,
    Sessions,
    Actions,
}

impl PaletteGroup {
    pub fn label(self) -> &'static str {
        match self {
            PaletteGroup::Navigation => "Navigation",
            PaletteGroup::Chat => "Chat",
            PaletteGroup::Crons => "Crons",
            PaletteGroup::Sessions => "Sessions",
            PaletteGroup::Actions => "Actions",
        }
    }

    /// Display order when the query is empty — Navigation first
    /// (most common interaction), Actions last (destructive
    /// operations shouldn't be the eye's first stop).
    pub fn sort_key(self) -> u8 {
        match self {
            PaletteGroup::Navigation => 0,
            PaletteGroup::Chat => 1,
            PaletteGroup::Crons => 2,
            PaletteGroup::Sessions => 3,
            PaletteGroup::Actions => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaletteEntry {
    pub action: PaletteAction,
    /// Primary label the scorer matches against and the UI renders.
    pub label: String,
    /// Optional second line for disambiguation (agent id next to a
    /// display name, session key with its model, etc.).
    pub subtitle: Option<String>,
    pub group: PaletteGroup,
}

/// Build the full action catalog from current app state. Called on
/// every palette-open and every input change — callers rank the
/// returned vec against the current query.
pub fn build_entries(ctx: PaletteContext<'_>) -> Vec<PaletteEntry> {
    let mut out = Vec::new();

    // Navigation — always present so the palette is useful on first
    // open even before any agents are discovered.
    for item in [
        (NavItem::Overview, "Overview"),
        (NavItem::Chat, "Chat"),
        (NavItem::Agents, "Agents"),
        (NavItem::Sessions, "Sessions"),
        (NavItem::Logs, "Logs"),
        (NavItem::Settings, "Settings"),
    ] {
        out.push(PaletteEntry {
            action: PaletteAction::Nav(item.0),
            label: format!("Go to {}", item.1),
            subtitle: None,
            group: PaletteGroup::Navigation,
        });
    }

    // Chat targets — one row per known agent from `agents.list`.
    for info in ctx.chat_agents {
        let display = info.display_with_emoji();
        let subtitle = if display == info.id {
            None
        } else {
            Some(info.id.clone())
        };
        out.push(PaletteEntry {
            action: PaletteAction::ChatWithAgent(AgentId::new(info.id.clone())),
            label: format!("Chat with {display}"),
            subtitle,
            group: PaletteGroup::Chat,
        });
    }

    // Cron runs — only the ones we can actually dispatch (have a
    // known UUID; the app's RunCron handler rejects others).
    for (agent_id, state) in ctx.cron_details {
        if !ctx.cron_ids.contains_key(agent_id) {
            continue;
        }
        let subtitle = state
            .last_status
            .as_deref()
            .map(|s| format!("last: {s}"))
            .or_else(|| state.next_run_at_ms.map(|_| "scheduled".to_string()));
        out.push(PaletteEntry {
            action: PaletteAction::RunCron(agent_id.clone()),
            label: format!("Run cron {}", agent_id.as_str()),
            subtitle,
            group: PaletteGroup::Crons,
        });
    }

    // Sessions drill-in — one row per known session.
    for (key, info) in ctx.sessions {
        let short_key = key.strip_prefix("agent:").unwrap_or(key);
        let subtitle = info
            .model
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        out.push(PaletteEntry {
            action: PaletteAction::OpenSession(key.clone()),
            label: format!("Open session {short_key}"),
            subtitle,
            group: PaletteGroup::Sessions,
        });
    }

    // Destructive: reset session, per Main agent. Keeps the
    // two-click confirmation — the palette just arms the first
    // click, the operator runs the palette again to confirm.
    for info in ctx.chat_agents {
        // Main agents only — the reset-session semantic only
        // applies to the chat-capable agents from `agents.list`.
        let agent_id = AgentId::new(info.id.clone());
        let is_main = ctx
            .agent_kind
            .get(&agent_id)
            .is_some_and(|k| matches!(k, AgentKind::Main))
            || info.id == "main";
        if !is_main {
            continue;
        }
        out.push(PaletteEntry {
            action: PaletteAction::ResetMainSession(agent_id),
            label: format!("Reset session for {}", info.display_with_emoji()),
            subtitle: Some("destructive — two clicks to confirm".to_string()),
            group: PaletteGroup::Actions,
        });
    }

    out
}

/// Borrow-shaped context the entry builder reads. Keeps `build_entries`
/// free of any `App` knowledge so we can unit-test it with hand-rolled
/// fixtures instead of a full app instance.
pub struct PaletteContext<'a> {
    pub chat_agents: &'a [AgentInfo],
    pub cron_details: &'a HashMap<AgentId, CronState>,
    pub cron_ids: &'a HashMap<AgentId, String>,
    pub sessions: &'a HashMap<String, SessionInfo>,
    pub agent_kind: &'a HashMap<AgentId, AgentKind>,
}

/// Rank entries against a query. Returns `(entry_index, score)`
/// tuples sorted by descending score. Empty query returns every
/// entry in group-then-insertion order so the first-open view
/// shows the catalog predictably.
pub fn rank(entries: &[PaletteEntry], query: &str) -> Vec<(usize, u32)> {
    if query.trim().is_empty() {
        let mut out: Vec<(usize, u32)> = entries.iter().enumerate().map(|(i, _)| (i, 0)).collect();
        out.sort_by_key(|(i, _)| {
            let e = &entries[*i];
            (e.group.sort_key(), *i)
        });
        return out;
    }
    let q = query.trim();
    let mut scored: Vec<(usize, u32)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| fuzzy_score(q, &e.label).map(|s| (i, s)))
        .collect();
    // Higher score first; stable tiebreak on insertion order so
    // groups stay predictable when many entries tie.
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored
}

/// Score how well `query` matches `target`. Returns `None` when
/// there's no subsequence match at all. Higher score = better.
///
/// Layered strategy, best-to-worst:
/// 1. Exact equality (case-insensitive) — 10_000.
/// 2. Case-insensitive prefix — 5_000 + length proximity bonus.
/// 3. Substring match — 2_000 + word-boundary bonus.
/// 4. Subsequence match — 100 + bonuses for consecutive/boundary hits.
///
/// Matches are case-insensitive throughout. Uses ASCII lowercasing
/// for speed since our action labels are all ASCII — if we ever
/// surface operator-provided strings here we'd need `to_lowercase`.
pub fn fuzzy_score(query: &str, target: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let q_bytes = query.as_bytes();
    let t_bytes = target.as_bytes();
    // ASCII-casefold both into small stack buffers. Labels are
    // bounded (<200 bytes), so this is allocation-free.
    let q_lower: Vec<u8> = q_bytes.iter().map(u8::to_ascii_lowercase).collect();
    let t_lower: Vec<u8> = t_bytes.iter().map(u8::to_ascii_lowercase).collect();

    if q_lower == t_lower {
        return Some(10_000);
    }
    if t_lower.starts_with(q_lower.as_slice()) {
        // Short labels get a bigger relative boost — "Chat" exactly
        // matching "ch" should outrank "Chart settings" matching "ch".
        let len_bonus = (256u32).saturating_sub(t_lower.len() as u32);
        return Some(5_000 + len_bonus);
    }
    if let Some(pos) = find_subslice(&t_lower, &q_lower) {
        let boundary = is_word_boundary(&t_lower, pos);
        let bonus = if boundary { 500 } else { 0 };
        return Some(2_000 + bonus + (256u32).saturating_sub(pos as u32));
    }
    // Subsequence: walk query chars through target, counting
    // consecutive hits and word-boundary hits.
    let mut ti = 0;
    let mut score = 100u32;
    let mut last_match_idx: Option<usize> = None;
    for &qc in &q_lower {
        loop {
            if ti >= t_lower.len() {
                return None;
            }
            if t_lower[ti] == qc {
                // Consecutive chars matched → extra points.
                if last_match_idx == Some(ti.saturating_sub(1)) {
                    score = score.saturating_add(10);
                }
                if is_word_boundary(&t_lower, ti) {
                    score = score.saturating_add(20);
                }
                last_match_idx = Some(ti);
                ti += 1;
                break;
            }
            ti += 1;
        }
    }
    Some(score)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A position is at a "word boundary" if it's at index 0 or
/// preceded by a non-alphanumeric byte (space, punctuation, etc.).
/// Used to favor matches that start a word in the label.
fn is_word_boundary(bytes: &[u8], idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    let prev = bytes[idx - 1];
    !(prev.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_zero_not_none() {
        // Empty queries land in the "show everything" branch upstream,
        // but if the scorer is ever called with "" it should still
        // return Some — unmatchable queries get None, empty ones
        // just have no preference.
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn exact_equality_beats_prefix_beats_substring() {
        let exact = fuzzy_score("chat", "chat").unwrap();
        let prefix = fuzzy_score("chat", "chat with sebastian").unwrap();
        let substring = fuzzy_score("session", "go to sessions").unwrap();
        let subsequence = fuzzy_score("gto", "go to overview").unwrap();
        assert!(exact > prefix, "{exact} > {prefix}");
        assert!(prefix > substring, "{prefix} > {substring}");
        assert!(substring > subsequence, "{substring} > {subsequence}");
    }

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_score("CHAT", "chat with memoria").is_some());
        assert!(fuzzy_score("chat", "CHAT WITH MEMORIA").is_some());
    }

    #[test]
    fn subsequence_match_boundary_bonus() {
        // "goto" matches "Go to Overview" with two boundary hits
        // (g@0, t@3) and two consecutive hits (g→o, t→o). The same
        // query against "goat ovoid" only hits one boundary at the
        // tail (o@5 after space) and one consecutive pair (g→o) —
        // so the well-structured label should score higher.
        let good = fuzzy_score("goto", "Go to Overview").unwrap();
        let bad = fuzzy_score("goto", "goat ovoid").unwrap();
        assert!(good > bad, "good={good} bad={bad}");
    }

    #[test]
    fn non_matching_returns_none() {
        assert_eq!(fuzzy_score("xyz", "chat with memoria"), None);
    }

    #[test]
    fn rank_orders_by_score_then_insertion() {
        let entries = vec![
            entry("Go to Overview", PaletteGroup::Navigation),
            entry("Go to Chat", PaletteGroup::Navigation),
            entry("Chat with Sebastian", PaletteGroup::Chat),
        ];
        let ranked = rank(&entries, "chat");
        // "Chat with Sebastian" is a prefix match and outranks
        // "Go to Chat" (substring); both outrank "Go to Overview"
        // (no match, filtered out).
        let labels: Vec<&str> = ranked
            .iter()
            .map(|(i, _)| entries[*i].label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["Chat with Sebastian", "Go to Chat"],
            "actual order: {labels:?}",
        );
    }

    #[test]
    fn rank_empty_query_groups_navigation_first() {
        let entries = vec![
            entry("Reset session for main", PaletteGroup::Actions),
            entry("Go to Overview", PaletteGroup::Navigation),
            entry("Chat with main", PaletteGroup::Chat),
        ];
        let ranked = rank(&entries, "");
        let labels: Vec<&str> = ranked
            .iter()
            .map(|(i, _)| entries[*i].label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["Go to Overview", "Chat with main", "Reset session for main"],
        );
    }

    #[test]
    fn word_boundary_detection() {
        assert!(is_word_boundary(b"hello world", 0));
        assert!(is_word_boundary(b"hello world", 6)); // "world"
        assert!(!is_word_boundary(b"hello world", 3)); // inside "hello"
        assert!(is_word_boundary(b"reset-session", 6));
    }

    fn entry(label: &str, group: PaletteGroup) -> PaletteEntry {
        PaletteEntry {
            action: PaletteAction::Nav(NavItem::Overview),
            label: label.to_string(),
            subtitle: None,
            group,
        }
    }
}
