//! Top-level app state, Message, update, view.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use iced::widget::{Canvas, canvas};
use iced::{Element, Length, Subscription, Task, time};

use crate::config;
use crate::domain::{Agent, AgentId, AgentKind, AgentStatus, agent};
use crate::logs::{self, LogFilters, LogLine, LogSeverity};
use crate::net::events::{ActivityKind, GatewayUpdate};
use crate::net::rpc::{
    AgentInfo, ApprovalEventPayload, Channel, CronState, SessionInfo, SessionUsagePoint,
};
use crate::net::{WsEvent, events, mock, openclaw};
use crate::notifications::Notifier;
use crate::palette::{self, PaletteAction, PaletteContext, PaletteEntry};
use crate::scene::{OfficeScene, ThoughtBubble, transition_text};
use crate::secret_store;
use crate::ui::chat_view::ChatMessage;
use crate::ui::{
    agent_card, agents_view, approvals, chat_input, chat_view, logs_view, palette as palette_view,
    sessions_view, settings_view, sidebar, status_bar, theme,
};
use crate::ui_state::{self, Settings, UiState, WindowState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItem {
    Overview,
    Chat,
    Agents,
    Sessions,
    Logs,
    Settings,
}

#[derive(Debug, Clone)]
pub enum Message {
    NavClicked(NavItem),
    Ws(WsEvent),
    Tick,
    /// Operator resolved a pending exec approval from the UI. The
    /// decision string matches OpenClaw's `ExecApprovalDecision`
    /// (`"allow-once" | "deny"` — `allow-always` intentionally not
    /// surfaced from the desktop to keep blast radius low).
    ResolveApproval {
        id: String,
        decision: &'static str,
    },
    /// Copy a string to the system clipboard — the UI uses this for
    /// one-click copying of shell commands the operator needs to run
    /// (e.g. the scope-upgrade approve line).
    CopyToClipboard(String),
    /// No-op sink for read-only `text_input` widgets (they require
    /// an `on_input` handler to stay interactive/selectable). The
    /// field's contents are discarded.
    InputDiscard(String),
    /// Fire a cron job immediately. Resolves the AgentId to its
    /// UUID via `cron_ids`, then dispatches `cron.run` to the WS.
    RunCron(AgentId),
    /// Operator-requested reconnect. Used by the "Retry now" button
    /// inside the scope-upgrade notice to short-circuit the long
    /// backoff after they've approved the pair-request on the
    /// gateway host.
    RequestReconnect,
    /// Chat input field contents changed (every keystroke).
    ChatInputChanged(String),
    /// Operator submitted the chat input (Enter or Send button).
    /// Dispatches `chat.send` with the current input text and clears
    /// the field. No-op when the input is empty or whitespace-only.
    SendChat,
    /// Operator picked a different agent from the Chat tab's left
    /// column. Switches the active conversation, clears the draft
    /// input, and (if not already hydrated this connection) kicks
    /// off `chat.history` for the new agent.
    SelectChatAgent(AgentId),
    /// Toggle the visibility chip for a given severity in the Logs
    /// tab. Applies only to the view filter — the underlying buffer
    /// is unchanged so toggling back is instant.
    LogsToggleSeverity(LogSeverity),
    /// Operator edited the Logs-tab search field (every keystroke).
    /// Empty string means no filter.
    LogsSearchChanged(String),
    /// Viewport updated in the Logs scrollable. Used to detect
    /// whether the operator has scrolled up (which pauses auto-tail
    /// and reveals the "Jump to latest" pill) or is pinned to the
    /// bottom (auto-tail resumes).
    LogsScrolled(iced::widget::scrollable::Viewport),
    /// "Jump to latest" pill clicked — scroll the Logs scrollable
    /// back to the bottom and resume auto-tail.
    LogsJumpToLatest,
    /// Operator clicked "Reset session" on a Main agent's Agents-tab
    /// row. First click arms confirmation (the button relabels and
    /// turns red); a second click within [`RESET_CONFIRM_WINDOW`]
    /// dispatches `sessions.reset`. Auto-disarms after the window
    /// elapses without a second click.
    ResetMainSession(AgentId),
    /// Click on an agent's error row — flip it between the
    /// truncated one-liner and the full expanded text. Target only
    /// exists when the agent has a `last_error` to show, so a
    /// stale dispatch is a no-op beyond set toggling.
    ToggleAgentError(AgentId),
    /// Operator clicked a session card in the Sessions tab. Updates
    /// `active_session_key` and kicks off `chat.history` for that
    /// session if the transcript isn't already cached for this
    /// connection. Persisted via `UiState` so the selection survives
    /// a relaunch.
    SessionSelected(String),
    /// ⌘K / Ctrl+K hit — open the palette if closed, close if open.
    /// On open, focuses the input via a `text_input::focus` task.
    PaletteToggle,
    /// Escape pressed or backdrop clicked — close the palette
    /// without executing anything. Also resets the query and
    /// selection so the next open starts fresh.
    PaletteClose,
    /// Operator typed in the palette input. Rebuilds the ranked
    /// filter list and clamps the selection.
    PaletteInputChanged(String),
    /// Up/Down arrow — move selection by delta (-1 up, +1 down).
    PaletteMove(i32),
    /// Enter pressed in the input field — execute the currently-
    /// selected entry.
    PaletteExecute,
    /// Operator clicked an entry row — execute it directly without
    /// requiring a separate selection step.
    PaletteSelectAndExecute(usize),
    /// Window was resized by the OS / compositor. Carries logical
    /// pixels. Debounced and persisted on the next `Tick` so a drag
    /// doesn't hammer the state file.
    WindowResized(f32, f32),
    /// Window was moved. Some compositors never emit this (Wayland
    /// hides window coordinates from the client), in which case we
    /// simply don't restore position on next launch.
    WindowMoved(f32, f32),
    /// Settings-tab gateway URL field changed (every keystroke).
    /// Updates the in-progress form; persistence happens on
    /// [`Message::SettingsSave`].
    SettingsGatewayUrlChanged(String),
    /// Settings-tab mode radio — `"auto" | "ws" | "mock"`.
    SettingsModeSelected(&'static str),
    /// Settings-tab token input (masked) changed. Write-only buffer:
    /// the field is never populated from storage, and it's cleared
    /// after a successful save. Keystrokes never leave the process
    /// until the operator clicks Save.
    SettingsTokenChanged(String),
    /// Settings-tab Save pressed. Flushes the form to persisted
    /// settings + writes any non-empty token through `secret_store`,
    /// then clears the token field so the UI reverts to the
    /// "token-present" state.
    SettingsSave,
    /// Settings-tab Clear Token pressed. Purges the token from the
    /// OS keychain AND the plaintext fallback file, regardless of
    /// build flavor — Clear means clear everywhere.
    SettingsClearToken,
}

pub struct App {
    pub nav: NavItem,
    pub roster: Vec<Agent>,
    pub statuses: HashMap<AgentId, AgentStatus>,
    pub bubbles: Vec<ThoughtBubble>,
    pub active_model: Option<String>,
    /// Timestamp of the most recent state update (push event OR bootstrap
    /// RPC). Despite the name, not a polling indicator — kept for the
    /// status-bar "last activity" readout.
    pub last_poll: Option<Instant>,
    pub connected: bool,
    pub last_disconnect: Option<String>,
    /// Pending exec approvals keyed by approval id. Populated by
    /// `exec.approval.requested`, cleared by `.resolved`. Scope-gated
    /// (empty unless the gateway granted `operator.read`+approvals).
    pub pending_approvals: HashMap<String, ApprovalEventPayload>,
    /// Per-session metadata keyed by session key. Populated from
    /// `sessions.list` at bootstrap and kept fresh via
    /// `session.message` events (the gateway spreads the same
    /// snapshot fields into those). Drives both the status bar's
    /// `ctx:` indicator and the Sessions nav tab.
    pub sessions: HashMap<String, SessionInfo>,
    /// Gateway-side update notification, when one is pending.
    pub gateway_update: Option<GatewayUpdate>,
    /// Non-None when the gateway has filed a pair-request (first
    /// pair OR scope upgrade) for this device and is waiting on
    /// the operator to approve it out-of-band (`openclaw devices
    /// approve <id>`). Surfaced in the approvals panel area so the
    /// fix is visible from any tab.
    pub pending_pair_request: Option<crate::net::events::PairRequest>,
    /// Full cron state per cron agent — keeps schedule-adjacent fields
    /// (`nextRunAtMs`, `lastRunAtMs`, `lastDurationMs`, `lastError`)
    /// that the Agents tab shows but the Overview sprite doesn't need.
    /// Populated from both the `cron.list` snapshot and the `cron`
    /// delta stream.
    pub cron_details: HashMap<AgentId, CronState>,
    /// AgentId → cron UUID. `cron.run` takes the UUID, not the name,
    /// so the Agents-tab "Run" button needs this to dispatch.
    /// Populated from the `cron.list` bootstrap snapshot only — push
    /// deltas carry the id too, but the snapshot is authoritative.
    pub cron_ids: HashMap<AgentId, String>,
    /// Full channel state per channel agent (connected, configured,
    /// last error). Refreshed by the 30s `channels.status` heartbeat.
    pub channel_details: HashMap<AgentId, Channel>,
    /// Rolling log-tail ring buffer, fed by the periodic `logs.tail`
    /// RPC. Each line is classified by severity at ingest so the
    /// Logs tab can filter / color without re-parsing on redraw.
    /// Bounded so memory stays flat on a long-running session.
    pub log_lines: VecDeque<LogLine>,
    /// Visible-severity chip toggles and the current search text.
    pub log_filters: LogFilters,
    /// `true` when the operator is scrolled to the bottom of the
    /// Logs scrollable — in which case new lines keep auto-scrolling
    /// into view. Flips to `false` the moment they scroll up,
    /// surfacing the "Jump to latest" pill; the pill sets it back.
    pub logs_auto_tail: bool,
    /// Instant at which each agent last changed status. Drives the
    /// ring-pulse animation in `OfficeScene`; entries older than
    /// [`TRANSITION_FLASH`] are pruned each tick.
    pub transition_moments: HashMap<AgentId, Instant>,
    pub scene_cache: canvas::Cache,
    /// Cache for the Sessions tab's detail-pane sparkline. Cleared
    /// whenever the active session's points change so the canvas
    /// re-renders only when there's something new to draw.
    pub sparkline_cache: canvas::Cache,
    /// Per-row mini-sparkline caches, keyed by session key. Lazily
    /// populated as `sessions.usage.timeseries` responses arrive;
    /// each entry's cache is cleared in place when its points
    /// update so only the affected row redraws.
    pub row_sparkline_caches: HashMap<String, canvas::Cache>,
    /// Contents of the chat input field (shared across Overview and
    /// Chat tabs). Cleared on agent switch so a half-typed message
    /// for Sebastian doesn't carry over into a reply to memoria.
    pub chat_input: String,
    /// Per-agent chat transcripts, keyed by agent id. Each log is
    /// bounded at `CHAT_LOG_MAX` turns; new agents get a fresh
    /// VecDeque on first reference via `chat_log_mut`.
    pub chat_logs: HashMap<AgentId, VecDeque<ChatMessage>>,
    /// Known chat-capable agents, delivered by the `agents.list`
    /// RPC. Drives the Chat picker rows and contributes sprites for
    /// any non-seeded Main agents to the Overview canvas.
    pub chat_agents: Vec<AgentInfo>,
    /// Currently-selected agent in the Chat picker. Also used by the
    /// Overview chat input (single "active conversation" concept).
    /// Seeded from `AgentsList.default_id` on first connect; stays
    /// `"main"` until then so the UI has something to render.
    pub selected_chat_agent: AgentId,
    /// Agents whose `chat.history` has been hydrated this connection.
    /// Cleared on disconnect so the next connect re-pulls history as
    /// the operator touches each agent — keeps the transcript
    /// authoritative against the server without chatty polling.
    pub history_fetched: HashSet<AgentId>,
    /// Per-agent "is this agent in the middle of processing my
    /// prompt?" indicator. Set on outbound `SendChat`, refined by
    /// `AgentActivity` and `session.tool`, cleared when the
    /// assistant `session.message` reply lands.
    pub chat_activities: HashMap<AgentId, ChatActivityState>,
    /// Main agents that have been armed for a session reset via the
    /// Agents tab's "Reset session" button. Instant records when the
    /// first click landed; a second click within
    /// [`RESET_CONFIRM_WINDOW`] actually fires the RPC. Stale entries
    /// are pruned each `Tick`.
    pub pending_resets: HashMap<AgentId, Instant>,
    /// Agent ids whose error row is currently expanded on the
    /// Agents tab. Transient view-state; not persisted across
    /// restarts because last-error text changes on the server's
    /// cadence and expanding yesterday's error tomorrow isn't
    /// useful.
    pub expanded_errors: HashSet<AgentId>,
    /// Per-session transcripts for the Sessions tab's drill-in. Keyed
    /// by the fully-qualified `agent:<id>:<sessionId>` key the gateway
    /// uses. Separate from `chat_logs` (which keys by agent id and
    /// holds the default-session transcript rendered in the Chat
    /// tab) so a drill-in doesn't clobber the Chat-tab view.
    pub session_transcripts: HashMap<String, VecDeque<ChatMessage>>,
    /// Session keys whose transcript has been hydrated this
    /// connection. Cleared on disconnect — the next reconnect
    /// re-pulls as the operator reopens each one.
    pub session_history_fetched: HashSet<String>,
    /// Currently-selected session in the Sessions tab's drill-in
    /// view. `None` means no session is selected and the detail
    /// pane shows its placeholder.
    pub active_session_key: Option<String>,
    /// Per-agent unread message count — incremented on every
    /// assistant reply that lands while the operator isn't actively
    /// watching that agent in the Chat tab. Cleared when the
    /// operator selects the agent (or opens the Chat tab on an
    /// already-selected agent). Drives the sidebar "Chat (N)"
    /// badge and per-row badges in the Chat picker.
    pub unread: HashMap<AgentId, usize>,
    /// Native-OS notification dispatcher. Stays in `App` state so
    /// its dedup sets (seen approvals, notified cron errors) persist
    /// across WsEvent arrivals, which otherwise would refire on
    /// every heartbeat. Cleared on disconnect so reconnect-time
    /// bootstrap can re-surface anything still unresolved.
    pub notifier: Notifier,
    /// Palette overlay visibility. When true, the main view is
    /// covered by a stack-layered palette widget that captures all
    /// keyboard nav.
    pub palette_open: bool,
    /// Text typed into the palette search input. Empty on first
    /// open; preserved across a close/reopen **within the same**
    /// session (actually no — we reset on close so the next open
    /// starts fresh, which matches operator muscle memory from
    /// VSCode / Slack / etc.).
    pub palette_input: String,
    /// Index into the ranked-entries list that the operator has
    /// arrow-keyed to. Clamped to the list length on each render,
    /// so growing/shrinking the filter set never leaves the
    /// selection dangling past the end.
    pub palette_selected: usize,
    /// Token-usage time series per session, keyed by full session
    /// key. Drives the sparkline in the detail pane. Absent ≠ empty:
    /// a missing entry means "not fetched yet"; an empty Vec means
    /// "fetched, no data points recorded."
    pub session_usage: HashMap<String, Vec<SessionUsagePoint>>,
    /// Pending window-state change awaiting a debounced flush. Reset
    /// each time the window moves or resizes; flushed to disk on the
    /// next `Tick` that lands at least [`WINDOW_SAVE_DEBOUNCE`] after
    /// the most recent change — prevents a drag-to-resize from
    /// writing the state file dozens of times per second.
    pending_window: Option<WindowState>,
    pending_window_since: Option<Instant>,
    /// Persisted connection settings (gateway URL + mode). Mirrors
    /// `UiState.settings` on disk. The Settings tab mutates this in
    /// place and triggers a write via `ui_state::save`; the ws
    /// subscription reads `gateway_url` here (with env-var override)
    /// to decide what to connect to.
    pub settings: Settings,
    /// In-progress Settings-tab form. Seeded from `settings` at
    /// startup and after each Save so navigating away and back
    /// preserves the displayed values; the token field is a
    /// write-only buffer that stays empty and isn't echoed back
    /// from storage.
    pub settings_form: SettingsForm,
    /// Cached "is a token currently saved?" flag. Populated at
    /// startup by `secret_store::has_token` and refreshed on Save /
    /// Clear so the view can toggle its status line without a live
    /// keychain call on every redraw.
    pub token_present: bool,
    /// Cached resolved token passed to the ws subscription. Populated
    /// from `config::try_load_token()` at startup + after Save /
    /// Clear; `subscription()` is called dozens of times per second
    /// by Iced, so resolving on every call (which hits disk + the
    /// keychain) is a non-starter. Stored as `Option<String>`
    /// because the gateway's Tailscale-auth path runs without a
    /// token at all.
    cached_token: Option<String>,
    /// Result of the most recent connect attempt, surfaced in the
    /// Settings tab so the operator gets feedback after Save
    /// without having to check the status bar. Drives the "status"
    /// line under the Gateway URL field.
    pub connection_status: ConnectionStatus,
    /// Bumps every time the operator clicks Save. Participates in
    /// `ConnectParams`' `Hash` so the ws subscription identity
    /// changes even when URL + token are unchanged — otherwise a
    /// Save with identical values would be a silent no-op and the
    /// operator would never see fresh "connecting / connected"
    /// feedback.
    save_nonce: u64,
}

/// Current connect attempt's outcome, shown in the Settings tab. A
/// reconnect loop on a bad URL cycles between `Connecting` and
/// `Failed` — the field reflects the latest event honestly rather
/// than freezing on the first failure.
#[derive(Debug, Clone, Default)]
pub enum ConnectionStatus {
    /// No Save this session, no attempt yet. View reads this as
    /// "(not tested)".
    #[default]
    Untested,
    /// WS subscription is alive and trying to establish a session.
    Connecting,
    /// Handshake succeeded and events are flowing.
    Ok,
    /// Latest attempt landed on `WsEvent::Disconnected(reason)`.
    /// Carries the reason string so the operator can see what
    /// went wrong (connection refused, TLS error, handshake
    /// rejection, etc.).
    Failed(String),
}

/// Ephemeral Settings-tab form state. Not persisted — it mirrors
/// `settings` plus a write-only `token` buffer that never reads from
/// storage. Stored on `App` so the contents survive navigating away
/// and back to the Settings tab within one session.
#[derive(Debug, Clone, Default)]
pub struct SettingsForm {
    pub gateway_url: String,
    /// `"auto"`, `"ws"`, or `"mock"`.
    pub mode: &'static str,
    /// Write-only token input. Cleared after Save; never populated
    /// from storage. Empty string means "operator hasn't typed
    /// anything this session".
    pub token: String,
}

impl SettingsForm {
    fn from_settings(settings: &Settings) -> Self {
        Self {
            gateway_url: settings.gateway_url.clone().unwrap_or_default(),
            mode: mode_as_static(settings.mode.as_deref()),
            token: String::new(),
        }
    }
}

/// Map a persisted mode string onto the small set of `&'static str`
/// values the radio widget uses. Unknown values normalize to
/// `"auto"` so old state files with typo'd modes don't get stuck.
fn mode_as_static(mode: Option<&str>) -> &'static str {
    match mode {
        Some("ws") => "ws",
        Some("mock") => "mock",
        _ => "auto",
    }
}

/// What the currently-selected agent is doing right now, as far as
/// the desktop knows. Rendered as a muted status row in the Chat tab
/// right above the input so the operator can see that their prompt
/// was received and the agent is working.
#[derive(Debug, Clone)]
pub struct ChatActivityState {
    pub kind: ChatActivity,
    pub since: Instant,
}

#[derive(Debug, Clone)]
pub enum ChatActivity {
    /// Prompt sent from the UI but not yet acknowledged by any
    /// server signal. Transitions to `Thinking` as soon as the first
    /// `agent` or `session.tool` event arrives.
    Sending,
    /// Agent is generating a response (streaming assistant deltas,
    /// planning, etc.).
    Thinking,
    /// Agent is running a tool. Name is empty when the event shape
    /// didn't include one.
    Tool(String),
}

/// How long a status-transition flash persists before the ring pulse
/// fades back to its resting stroke. Eye-noticeable without feeling
/// frantic.
pub const TRANSITION_FLASH: Duration = Duration::from_millis(600);

/// Auto-clear the chat-activity indicator after this long without a
/// corroborating event. Prevents a "thinking…" row from being stuck
/// when the server fails to close out a run cleanly (disconnect,
/// dropped event, etc.) — the operator's next send resets it anyway,
/// but a ghost indicator reads as broken.
pub const CHAT_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(45);

/// Chat log ring-buffer size. 500 turns is deep enough that the
/// operator never hits the edge during a working session, shallow
/// enough that full history renders as a single scrollable list
/// without paging.
const CHAT_LOG_MAX: usize = 500;

/// How long to wait after the last window move/resize before
/// persisting the new dimensions. 250 ms is long enough to collapse
/// an entire interactive drag into one write, short enough that a
/// quick resize + quit still lands on disk.
const WINDOW_SAVE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Two-click confirmation window for destructive ops like Reset
/// session. 4 seconds is enough that an operator whose mouse hovered
/// elsewhere has time to come back, short enough that a stale arm
/// doesn't quietly disarm nothing and then trigger on the next click.
const RESET_CONFIRM_WINDOW: Duration = Duration::from_secs(4);

impl Default for App {
    fn default() -> Self {
        Self::new(UiState::default())
    }
}

impl App {
    /// Build a fresh App, applying any persisted UI state where the
    /// value is still meaningful (tab / selected agent). The seed
    /// roster is unconditional — the persisted selected agent may
    /// reference a dynamic id that hasn't been re-announced yet, but
    /// when `agents.list` arrives the selection simply stays put and
    /// `chat.history` fires on first open as usual.
    pub fn new(state: UiState) -> Self {
        // First-run detection: if the operator has never configured a
        // gateway URL (neither persisted nor as an env var) and isn't
        // opting into mock mode, bounce them straight to the Settings
        // tab on launch — there's nothing meaningful to show on any
        // other tab until a URL is set.
        let first_run_incomplete = !mock::enabled()
            && std::env::var("OPENCLAW_GATEWAY_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .is_none()
            && state
                .settings
                .gateway_url
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .is_none();
        let nav = if first_run_incomplete {
            NavItem::Settings
        } else {
            state
                .tab
                .as_deref()
                .and_then(ui_state::nav_from_str)
                .unwrap_or(NavItem::Overview)
        };
        let selected_chat_agent = state
            .selected_agent
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(AgentId::new)
            .unwrap_or_else(|| AgentId::new("main"));
        Self {
            nav,
            roster: agent::seed_roster(),
            statuses: HashMap::new(),
            bubbles: Vec::new(),
            active_model: None,
            last_poll: None,
            connected: false,
            last_disconnect: None,
            pending_approvals: HashMap::new(),
            sessions: HashMap::new(),
            gateway_update: None,
            pending_pair_request: None,
            cron_details: HashMap::new(),
            cron_ids: HashMap::new(),
            channel_details: HashMap::new(),
            log_lines: VecDeque::with_capacity(2048),
            log_filters: LogFilters::default(),
            logs_auto_tail: true,
            transition_moments: HashMap::new(),
            scene_cache: canvas::Cache::default(),
            sparkline_cache: canvas::Cache::default(),
            row_sparkline_caches: HashMap::new(),
            chat_input: String::new(),
            chat_logs: HashMap::new(),
            chat_agents: Vec::new(),
            selected_chat_agent,
            history_fetched: HashSet::new(),
            chat_activities: HashMap::new(),
            pending_resets: HashMap::new(),
            expanded_errors: HashSet::new(),
            session_transcripts: HashMap::new(),
            session_history_fetched: HashSet::new(),
            active_session_key: state.active_session_key.filter(|s| !s.is_empty()),
            session_usage: HashMap::new(),
            unread: HashMap::new(),
            notifier: Notifier::new(),
            palette_open: false,
            palette_input: String::new(),
            palette_selected: 0,
            pending_window: state.window,
            pending_window_since: None,
            settings_form: SettingsForm::from_settings(&state.settings),
            // Status reflects what the ws subscription will actually
            // do in `fn subscription`: no URL → stays Untested; URL
            // configured → enters Connecting immediately so the
            // Settings view doesn't flash "(not tested)" for a split
            // second before the first WsEvent lands.
            connection_status: if first_run_incomplete || mock::enabled() {
                ConnectionStatus::Untested
            } else {
                ConnectionStatus::Connecting
            },
            settings: state.settings,
            token_present: secret_store::has_token(),
            cached_token: config::try_load_token(),
            save_nonce: 0,
        }
    }

    /// Build the palette's action catalog from current state.
    /// Called on every render and every input-changed message —
    /// cheap (a few HashMap iterations); no caching warranted.
    fn palette_entries(&self) -> Vec<PaletteEntry> {
        // The catalog's "reset-session" entries gate on agent kind,
        // so we need an `AgentId → AgentKind` map even though the
        // rest of the app keys its kind info through the `roster`
        // vec. Build one on the fly.
        let mut agent_kind = HashMap::new();
        for a in &self.roster {
            agent_kind.insert(a.id.clone(), a.kind);
        }
        palette::build_entries(PaletteContext {
            chat_agents: &self.chat_agents,
            cron_details: &self.cron_details,
            cron_ids: &self.cron_ids,
            sessions: &self.sessions,
            agent_kind: &agent_kind,
        })
    }

    /// Dispatch the action behind the currently-selected entry.
    /// Each `PaletteAction` variant maps to an existing top-level
    /// `Message`, so this is just a match + recursive update call —
    /// no new runtime behavior is introduced by the palette.
    fn palette_execute(&mut self, ranked_idx: usize) -> Task<Message> {
        let entries = self.palette_entries();
        let ranked = palette::rank(&entries, &self.palette_input);
        let Some((entry_idx, _)) = ranked.get(ranked_idx).copied() else {
            // No-op — selection pointing past the list (e.g. from
            // a race with filter narrowing). Close the palette so
            // the operator doesn't sit with a non-responsive state.
            self.palette_open = false;
            return Task::none();
        };
        let Some(entry) = entries.get(entry_idx) else {
            self.palette_open = false;
            return Task::none();
        };
        // Close the palette before dispatching so the downstream
        // handler (which may also mutate state) renders on top of
        // a clean layout.
        self.palette_open = false;
        self.palette_input.clear();
        self.palette_selected = 0;
        let message = match entry.action.clone() {
            PaletteAction::Nav(nav) => Message::NavClicked(nav),
            PaletteAction::RunCron(id) => Message::RunCron(id),
            PaletteAction::ChatWithAgent(id) => {
                // Switch to Chat tab first, then select the agent —
                // two chained updates so the operator lands on the
                // right surface for their intent.
                let sub = self.update(Message::NavClicked(NavItem::Chat));
                let sub2 = self.update(Message::SelectChatAgent(id));
                return Task::batch([sub, sub2]);
            }
            PaletteAction::OpenSession(key) => {
                let sub = self.update(Message::NavClicked(NavItem::Sessions));
                let sub2 = self.update(Message::SessionSelected(key));
                return Task::batch([sub, sub2]);
            }
            PaletteAction::ResetMainSession(id) => Message::ResetMainSession(id),
        };
        self.update(message)
    }

    /// `true` when no gateway URL is configured and mock mode isn't
    /// active — i.e. the app has nothing useful to connect to and
    /// the Settings tab should show a first-run banner asking the
    /// operator to configure one.
    pub fn first_run_incomplete(&self) -> bool {
        if mock::enabled() {
            return false;
        }
        if std::env::var("OPENCLAW_GATEWAY_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            return false;
        }
        self.settings
            .gateway_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .is_none()
    }

    /// Snapshot the bits of the app we persist across launches.
    fn ui_state_snapshot(&self) -> UiState {
        UiState {
            tab: Some(ui_state::nav_to_str(self.nav).to_string()),
            selected_agent: Some(self.selected_chat_agent.as_str().to_string()),
            active_session_key: self.active_session_key.clone(),
            window: self.pending_window,
            settings: self.settings.clone(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::NavClicked(item) => {
                if self.nav != item {
                    self.nav = item;
                    ui_state::save(&self.ui_state_snapshot());
                }
                // Navigating into Chat clears the currently-selected
                // agent's unread — we assume the operator is now
                // watching. Other-agent counts survive so a switch
                // away and back to Chat still shows what was missed.
                if item == NavItem::Chat {
                    self.unread.remove(&self.selected_chat_agent);
                }
                Task::none()
            }
            Message::Ws(event) => {
                self.apply_ws(event);
                // Invalidate canvas cache so sprites re-render at new positions.
                self.scene_cache.clear();
                Task::none()
            }
            Message::ResolveApproval { id, decision } => {
                tracing::info!(id = %id, decision, "UI: resolve approval");
                // Optimistically drop the entry so the panel collapses
                // immediately; the gateway's `exec.approval.resolved`
                // event will arrive shortly and confirm. If the RPC
                // fails, the operator can retry when/if the event
                // re-fires (real rare case).
                self.pending_approvals.remove(&id);
                if let Err(e) = crate::net::commands::sender().send(
                    crate::net::commands::GatewayCommand::ResolveApproval {
                        id,
                        decision: decision.to_string(),
                    },
                ) {
                    tracing::warn!(error = %e, "could not dispatch ResolveApproval command");
                }
                Task::none()
            }
            Message::CopyToClipboard(value) => {
                tracing::debug!(len = value.len(), "UI: copy to clipboard");
                iced::clipboard::write(value)
            }
            Message::InputDiscard(_) => Task::none(),
            Message::ChatInputChanged(value) => {
                self.chat_input = value;
                Task::none()
            }
            Message::SendChat => {
                let prompt = self.chat_input.trim().to_string();
                if prompt.is_empty() {
                    return Task::none();
                }
                let idem = uuid::Uuid::new_v4().to_string();
                let target = self.selected_chat_agent.clone();
                tracing::info!(
                    len = prompt.len(),
                    agent = %target.as_str(),
                    "UI: chat.send",
                );
                self.chat_input.clear();

                // Optimistic UI: outgoing bubble over the target
                // sprite + append to that agent's chat log.
                let snippet = clean_bubble_text(&prompt, 80);
                if !snippet.is_empty() {
                    self.bubbles
                        .push(ThoughtBubble::outgoing(target.clone(), snippet));
                }
                push_chat(
                    chat_log_mut(&mut self.chat_logs, &target),
                    ChatMessage::user(&prompt),
                );
                // Mark the target agent as actively sending. The
                // chat view renders this as a muted "sending…" row
                // above the input; refinement to Thinking or Tool
                // happens when the first server event arrives.
                self.chat_activities.insert(
                    target.clone(),
                    ChatActivityState {
                        kind: ChatActivity::Sending,
                        since: Instant::now(),
                    },
                );
                // Flip the target sprite into Running so the ring
                // pulses and "…working…" appears while the agent
                // processes. Reply stream settles the status back to
                // Ok once agent activity ends.
                self.apply_status_update(target.clone(), AgentStatus::Running);
                self.scene_cache.clear();

                if let Err(e) = crate::net::commands::sender().send(
                    crate::net::commands::GatewayCommand::SendChat {
                        agent_id: target.as_str().to_string(),
                        message: prompt,
                        idempotency_key: idem,
                    },
                ) {
                    tracing::warn!(error = %e, "could not dispatch SendChat command");
                }
                Task::none()
            }
            Message::SelectChatAgent(agent_id) => {
                if self.selected_chat_agent == agent_id {
                    return Task::none();
                }
                tracing::info!(
                    to = %agent_id.as_str(),
                    from = %self.selected_chat_agent.as_str(),
                    "UI: switch chat agent",
                );
                self.selected_chat_agent = agent_id.clone();
                // A draft written for the previous agent rarely makes
                // sense after a switch — clear so the new target's
                // conversation starts fresh.
                self.chat_input.clear();
                // Operator is now watching this agent; drop its
                // unread count so the sidebar/picker badges clear
                // immediately.
                self.unread.remove(&agent_id);
                // Lazy hydrate: only fire chat.history the first time
                // we open a given agent per connection.
                if !self.history_fetched.contains(&agent_id) {
                    self.history_fetched.insert(agent_id.clone());
                    if let Err(e) = crate::net::commands::sender().send(
                        crate::net::commands::GatewayCommand::FetchChatHistory {
                            agent_id: agent_id.as_str().to_string(),
                        },
                    ) {
                        tracing::warn!(error = %e, "could not dispatch FetchChatHistory");
                    }
                }
                ui_state::save(&self.ui_state_snapshot());
                Task::none()
            }
            Message::RequestReconnect => {
                tracing::info!("UI: operator requested reconnect");
                // Clear the notice optimistically — the WS will either
                // reconnect and send a fresh `PairRequestPending` if
                // still unpaired, or `Connected` if the approval took
                // effect.
                self.pending_pair_request = None;
                if let Err(e) = crate::net::commands::sender()
                    .send(crate::net::commands::GatewayCommand::Reconnect)
                {
                    tracing::warn!(error = %e, "could not dispatch Reconnect command");
                }
                Task::none()
            }
            Message::RunCron(agent_id) => {
                let Some(uuid) = self.cron_ids.get(&agent_id).cloned() else {
                    tracing::warn!(
                        id = %agent_id.as_str(),
                        "RunCron fired for agent without known UUID; ignoring",
                    );
                    return Task::none();
                };
                tracing::info!(
                    id = %agent_id.as_str(),
                    job_id = %uuid,
                    "UI: cron.run",
                );
                if let Err(e) = crate::net::commands::sender()
                    .send(crate::net::commands::GatewayCommand::RunCron { job_id: uuid })
                {
                    tracing::warn!(error = %e, "could not dispatch RunCron command");
                }
                Task::none()
            }
            Message::PaletteToggle => {
                if self.palette_open {
                    self.palette_open = false;
                    self.palette_input.clear();
                    self.palette_selected = 0;
                    Task::none()
                } else {
                    self.palette_open = true;
                    self.palette_selected = 0;
                    // Focus the text_input so typing starts
                    // immediately — Iced's input_id lookup lives in
                    // the palette UI module.
                    iced::widget::operation::focus(palette_view::input_id())
                }
            }
            Message::PaletteClose => {
                self.palette_open = false;
                self.palette_input.clear();
                self.palette_selected = 0;
                Task::none()
            }
            Message::PaletteInputChanged(value) => {
                self.palette_input = value;
                // Re-rank can shorten the list; clamp selection so
                // it stays inside bounds for the new filter set.
                let entries = self.palette_entries();
                let ranked = palette::rank(&entries, &self.palette_input);
                if ranked.is_empty() {
                    self.palette_selected = 0;
                } else if self.palette_selected >= ranked.len() {
                    self.palette_selected = ranked.len() - 1;
                }
                Task::none()
            }
            Message::PaletteMove(delta) => {
                if !self.palette_open {
                    return Task::none();
                }
                let entries = self.palette_entries();
                let ranked = palette::rank(&entries, &self.palette_input);
                if ranked.is_empty() {
                    return Task::none();
                }
                let len = ranked.len() as i32;
                let mut next = self.palette_selected as i32 + delta;
                // Wrap around — Spotlight-style; feels natural for a
                // short list and avoids "stuck at top/bottom" friction.
                while next < 0 {
                    next += len;
                }
                self.palette_selected = (next % len) as usize;
                Task::none()
            }
            Message::PaletteExecute => self.palette_execute(self.palette_selected),
            Message::PaletteSelectAndExecute(idx) => self.palette_execute(idx),
            Message::SessionSelected(session_key) => {
                let changed = self.active_session_key.as_deref() != Some(session_key.as_str());
                if changed {
                    tracing::info!(key = %session_key, "UI: session drill-in");
                    self.active_session_key = Some(session_key.clone());
                    // New active session → sparkline needs to redraw
                    // against whatever points are (or aren't) cached.
                    self.sparkline_cache.clear();
                    ui_state::save(&self.ui_state_snapshot());
                }
                // Lazy hydrate: only fetch this session's transcript
                // the first time it's opened per connection. The
                // timeseries fetch piggybacks off the same gate —
                // no independent "usage fetched" bookkeeping to
                // keep in sync.
                if !self.session_history_fetched.contains(&session_key) {
                    self.session_history_fetched.insert(session_key.clone());
                    let sender = crate::net::commands::sender();
                    if let Err(e) =
                        sender.send(crate::net::commands::GatewayCommand::FetchSessionHistory {
                            session_key: session_key.clone(),
                        })
                    {
                        tracing::warn!(error = %e, "could not dispatch FetchSessionHistory");
                    }
                    if let Err(e) =
                        sender.send(crate::net::commands::GatewayCommand::FetchSessionUsage {
                            session_key,
                        })
                    {
                        tracing::warn!(error = %e, "could not dispatch FetchSessionUsage");
                    }
                }
                Task::none()
            }
            Message::ToggleAgentError(agent_id) => {
                if !self.expanded_errors.remove(&agent_id) {
                    self.expanded_errors.insert(agent_id);
                }
                Task::none()
            }
            Message::ResetMainSession(agent_id) => {
                let now = Instant::now();
                let armed_recently = self
                    .pending_resets
                    .get(&agent_id)
                    .is_some_and(|t| now.saturating_duration_since(*t) < RESET_CONFIRM_WINDOW);
                if armed_recently {
                    self.pending_resets.remove(&agent_id);
                    let session_key = format!("agent:{}:main", agent_id.as_str());
                    tracing::info!(
                        id = %agent_id.as_str(),
                        key = %session_key,
                        "UI: sessions.reset (confirmed)",
                    );
                    if let Err(e) = crate::net::commands::sender()
                        .send(crate::net::commands::GatewayCommand::ResetSession { session_key })
                    {
                        tracing::warn!(error = %e, "could not dispatch ResetSession command");
                    }
                } else {
                    tracing::debug!(
                        id = %agent_id.as_str(),
                        "UI: reset armed — awaiting confirmation",
                    );
                    self.pending_resets.insert(agent_id, now);
                }
                Task::none()
            }
            Message::LogsToggleSeverity(sev) => {
                self.log_filters.toggle(sev);
                Task::none()
            }
            Message::LogsSearchChanged(value) => {
                self.log_filters.search = value;
                Task::none()
            }
            Message::LogsScrolled(viewport) => {
                // Pin = within 2% of the bottom. A hard == 1.0 check
                // is too strict — float rounding during resize or
                // content-growth can produce 0.9998 even when the
                // user hasn't scrolled away. 0.98 gives the view
                // headroom without making a small upward scroll go
                // unnoticed.
                let y = viewport.relative_offset().y;
                self.logs_auto_tail = !y.is_finite() || y >= 0.98;
                Task::none()
            }
            Message::LogsJumpToLatest => {
                self.logs_auto_tail = true;
                iced::widget::operation::snap_to_end(logs_view::scroll_id())
            }
            Message::WindowResized(width, height) => {
                let window = self.pending_window.unwrap_or(WindowState {
                    width,
                    height,
                    position: None,
                });
                self.pending_window = Some(WindowState {
                    width,
                    height,
                    position: window.position,
                });
                self.pending_window_since = Some(Instant::now());
                Task::none()
            }
            Message::WindowMoved(x, y) => {
                let window = self.pending_window.unwrap_or(WindowState {
                    // No size yet — use the launch defaults so we at
                    // least write something sensible. The first Resize
                    // event to follow will overwrite these.
                    width: 1280.0,
                    height: 800.0,
                    position: None,
                });
                self.pending_window = Some(WindowState {
                    width: window.width,
                    height: window.height,
                    position: Some((x, y)),
                });
                self.pending_window_since = Some(Instant::now());
                Task::none()
            }
            Message::SettingsGatewayUrlChanged(value) => {
                self.settings_form.gateway_url = value;
                Task::none()
            }
            Message::SettingsModeSelected(value) => {
                self.settings_form.mode = value;
                Task::none()
            }
            Message::SettingsTokenChanged(value) => {
                self.settings_form.token = value;
                Task::none()
            }
            Message::SettingsSave => {
                // Flush the form's non-secret fields into persisted
                // settings. Empty gateway URL is stored as `None` so
                // the ws subscription stays idle instead of trying
                // to connect to the empty string.
                let trimmed_url = self.settings_form.gateway_url.trim();
                self.settings.gateway_url =
                    (!trimmed_url.is_empty()).then(|| trimmed_url.to_string());
                // Mode `"auto"` doesn't need to be persisted — it's
                // the default. Only stash a non-default choice so old
                // state files that predate the setting don't suddenly
                // grow a `"mode": "auto"` entry.
                self.settings.mode = match self.settings_form.mode {
                    "ws" | "mock" => Some(self.settings_form.mode.to_string()),
                    _ => None,
                };
                ui_state::save(&self.ui_state_snapshot());

                // Token: only touch the secret store when the operator
                // actually typed something. An empty field on Connect
                // means "don't change the current token" — use Clear
                // to delete. We deliberately *don't* wipe the form
                // field after save — operators expect their typed
                // value to stay visible (masked) so they can tell
                // their entry persisted, rather than being left
                // staring at a blank input wondering if it took.
                let token = self.settings_form.token.trim().to_string();
                if !token.is_empty() {
                    if let Err(e) = secret_store::save_token(&token) {
                        tracing::warn!(error = %e, "saving token to secret store failed");
                    } else {
                        self.token_present = true;
                        // Refresh the cached token so the ws
                        // subscription's identity changes and Iced
                        // tears down + restarts the session with
                        // the new credential.
                        self.cached_token = config::try_load_token();
                    }
                }
                // Bump the nonce and mark status `Connecting` so the
                // Settings view shows fresh feedback even when the
                // operator re-saved the same URL/token (which
                // wouldn't otherwise change the subscription identity).
                self.save_nonce = self.save_nonce.wrapping_add(1);
                self.connection_status = if self.first_run_incomplete() {
                    ConnectionStatus::Untested
                } else {
                    ConnectionStatus::Connecting
                };
                Task::none()
            }
            Message::SettingsClearToken => {
                secret_store::clear_token();
                self.token_present = false;
                self.settings_form.token.clear();
                // Drop the in-memory token directly rather than
                // re-running the resolver. The resolver's last-
                // resort path reads from `~/.openclaw/openclaw.json`
                // and stashes whatever it finds back into the secret
                // store — which would silently undo the Clear for
                // OpenClaw-CLI users and is the opposite of what the
                // operator asked for.
                self.cached_token = None;
                Task::none()
            }
            Message::Tick => {
                let now = Instant::now();
                let before = self.bubbles.len();
                self.bubbles.retain(|b| !b.expired(now));
                self.transition_moments
                    .retain(|_, t| now.saturating_duration_since(*t) < TRANSITION_FLASH);
                // Debounced window-state flush. Only write once the
                // operator has stopped dragging / resizing for the
                // debounce interval — otherwise a resize produces
                // dozens of writes per second.
                if let Some(since) = self.pending_window_since
                    && now.saturating_duration_since(since) >= WINDOW_SAVE_DEBOUNCE
                {
                    self.pending_window_since = None;
                    ui_state::save(&self.ui_state_snapshot());
                }
                // Drop activity rows that have been stale for too
                // long — prevents a "thinking…" indicator from
                // getting stuck when the server fails to close out
                // the run cleanly.
                self.chat_activities.retain(|_, state| {
                    now.saturating_duration_since(state.since) < CHAT_ACTIVITY_TIMEOUT
                });
                // Disarm stale reset confirmations so the button
                // doesn't sit in red indefinitely after an operator
                // wandered off.
                self.pending_resets
                    .retain(|_, t| now.saturating_duration_since(*t) < RESET_CONFIRM_WINDOW);
                // Redraw every tick while anything is animating. The
                // office is considered "animating" if there's any
                // bubble or if a sprite is in a state that changes
                // between frames (running bob, transition flash, or
                // the idle walk-frame cycle on Main/Cron). Channels
                // also animate via the scrolling scanline when not
                // disabled. `Tick` itself throttles the rate, so
                // clearing the cache here is cheap.
                let has_active_channels = self.roster.iter().any(|a| {
                    matches!(a.kind, AgentKind::Channel)
                        && !matches!(
                            self.statuses
                                .get(&a.id)
                                .copied()
                                .unwrap_or(AgentStatus::Unknown),
                            AgentStatus::Disabled,
                        )
                });
                if self.bubbles.len() != before
                    || !self.bubbles.is_empty()
                    || self.any_sprite_animating(now)
                    || self.any_sprite_idle_cycling()
                    || has_active_channels
                {
                    self.scene_cache.clear();
                }
                Task::none()
            }
        }
    }

    fn apply_ws(&mut self, event: WsEvent) {
        match event {
            WsEvent::Connected => {
                self.connected = true;
                self.last_disconnect = None;
                self.connection_status = ConnectionStatus::Ok;
                tracing::info!("WS connected");
            }
            WsEvent::Disconnected(reason) => {
                self.connected = false;
                self.last_disconnect = Some(reason.clone());
                self.connection_status = ConnectionStatus::Failed(reason.clone());
                // Reset the "hydrated this connection" set so the
                // next successful connect re-pulls chat.history as
                // the operator touches each agent. Logs themselves
                // are kept so the visible transcript survives a blip.
                self.history_fetched.clear();
                // Session drill-in transcripts follow the same
                // rule — keep them rendered across a blip, but
                // re-pull on reopen after reconnect.
                self.session_history_fetched.clear();
                // Let unresolved approvals / cron errors re-surface
                // on the next connect so the operator isn't left
                // staring at a stale snapshot without a fresh ping.
                self.notifier.reset_on_disconnect();
                // Clear pending-response indicators — any run that
                // was in progress gets re-signaled on reconnect if
                // it's still going; a ghost "thinking…" across a
                // reconnect would misrepresent state.
                self.chat_activities.clear();
                tracing::warn!(%reason, "WS disconnected");
            }
            WsEvent::CronSnapshot(crons) => {
                self.last_poll = Some(Instant::now());
                for cron in &crons {
                    let id = events::cron_agent_id(cron);
                    self.ensure_agent(&id, AgentKind::Cron);
                    self.cron_details.insert(id.clone(), cron.state.clone());
                    if let Some(uuid) = cron.id.as_deref() {
                        self.cron_ids.insert(id.clone(), uuid.to_string());
                    }
                    // Notify if this cron is reporting an error —
                    // Notifier's dedup keeps repeated heartbeats
                    // quiet.
                    self.notifier.cron_state_changed(&id, &cron.state);
                    let status = events::cron_status(cron);
                    self.apply_status_update(id, status);
                }
            }
            WsEvent::CronDelta(cron) => {
                self.last_poll = Some(Instant::now());
                let id = events::cron_agent_id(&cron);
                self.ensure_agent(&id, AgentKind::Cron);
                // Merge rather than replace — push events only carry
                // the fields that changed, so a `finished` delta
                // shouldn't wipe the unrelated `nextRunAtMs` from the
                // previous snapshot.
                merge_cron_state(
                    self.cron_details.entry(id.clone()).or_default(),
                    &cron.state,
                );
                // Notify from the merged state (not the delta alone)
                // so a `finished` event with no `last_error` set
                // doesn't accidentally clear dedup when we still
                // have the error text from the snapshot.
                if let Some(merged) = self.cron_details.get(&id) {
                    self.notifier.cron_state_changed(&id, merged);
                }
                let status = events::cron_status(&cron);
                self.apply_status_update(id, status);
            }
            WsEvent::ChannelSnapshot(channels) => {
                self.last_poll = Some(Instant::now());
                for ch in &channels {
                    let id = events::channel_agent_id(ch);
                    self.ensure_agent(&id, AgentKind::Channel);
                    self.channel_details.insert(id.clone(), ch.clone());
                    let status = events::channel_status(ch);
                    self.apply_status_update(id, status);
                }
            }
            WsEvent::MainAgent(main) => {
                if let Some(model) = main.model.as_ref() {
                    self.active_model = Some(model.clone());
                }
                let id = AgentId::new(&main.id);
                let status = events::main_agent_status(&main);
                self.apply_status_update(id, status);
            }
            WsEvent::AgentsList { default_id, agents } => {
                // Store the raw list for the Chat picker.
                self.chat_agents = agents.clone();

                // Ensure every discovered agent has a sprite on the
                // Overview canvas; `main` is seeded, others need to
                // be added. Apply identity display on top of either.
                for info in &agents {
                    let persona = info.display_with_emoji();
                    if let Some(entry) = self.roster.iter_mut().find(|a| a.id.as_str() == info.id) {
                        if entry.display != persona {
                            tracing::info!(
                                id = %info.id,
                                persona = %persona,
                                "roster: identity rename",
                            );
                            entry.display = persona;
                            self.scene_cache.clear();
                        }
                    } else {
                        tracing::info!(
                            id = %info.id,
                            persona = %persona,
                            "roster: add chat agent",
                        );
                        self.roster
                            .push(Agent::main_with_display(&info.id, &persona));
                        self.scene_cache.clear();
                    }
                }

                // `agents.list` only carries identity fields from the
                // agent's own config entry. For the default agent and
                // anything configured via `ui.assistant` or the
                // workspace identity file, we need a per-agent
                // `agent.identity.get` call to get the real persona
                // ("Sebastian 🦀"). Fire one per agent now — cheap, and
                // the response merges in via the `AgentIdentity` arm
                // without overwriting data we already rendered.
                let sender = crate::net::commands::sender();
                for info in &agents {
                    if let Err(e) =
                        sender.send(crate::net::commands::GatewayCommand::FetchAgentIdentity {
                            agent_id: info.id.clone(),
                        })
                    {
                        tracing::warn!(
                            id = %info.id,
                            error = %e,
                            "could not dispatch FetchAgentIdentity",
                        );
                    }
                }

                // First selection: pick whatever the server says is
                // the default. Subsequent `AgentsList` events (e.g.
                // reconnect) don't override an operator choice.
                if !self.history_fetched.contains(&self.selected_chat_agent)
                    && self.selected_chat_agent.as_str() == "main"
                    && !default_id.is_empty()
                    && default_id != "main"
                {
                    self.selected_chat_agent = AgentId::new(&default_id);
                }

                // Kick off chat.history for whatever's currently
                // selected — the operator sees their active
                // conversation populate within a round-trip without
                // having to click the picker manually.
                if !self.history_fetched.contains(&self.selected_chat_agent) {
                    let target = self.selected_chat_agent.clone();
                    self.history_fetched.insert(target.clone());
                    if let Err(e) = crate::net::commands::sender().send(
                        crate::net::commands::GatewayCommand::FetchChatHistory {
                            agent_id: target.as_str().to_string(),
                        },
                    ) {
                        tracing::warn!(error = %e, "could not dispatch FetchChatHistory");
                    }
                }
            }
            WsEvent::AgentIdentity {
                agent_id,
                name,
                emoji,
            } => {
                // Merge the richer persona into `chat_agents` — the
                // Chat picker and the right-pane header pull from
                // that vec. `display_with_emoji` will prefer the
                // nested `identity.name` over the top-level name, so
                // we populate the nested shape.
                if let Some(entry) = self
                    .chat_agents
                    .iter_mut()
                    .find(|a| a.id == agent_id.as_str())
                {
                    let identity = entry
                        .identity
                        .get_or_insert_with(crate::net::rpc::AgentIdentity::default);
                    if let Some(n) = name.as_deref() {
                        identity.name = Some(n.to_string());
                    }
                    if let Some(e) = emoji.as_deref() {
                        identity.emoji = Some(e.to_string());
                    }
                    // Mirror into the sprite label on the Overview.
                    let persona = entry.display_with_emoji();
                    if let Some(roster_entry) = self
                        .roster
                        .iter_mut()
                        .find(|a| a.id.as_str() == agent_id.as_str())
                        && roster_entry.display != persona
                    {
                        tracing::info!(
                            id = %agent_id.as_str(),
                            persona = %persona,
                            "roster: identity refined",
                        );
                        roster_entry.display = persona;
                        self.scene_cache.clear();
                    }
                }
            }
            WsEvent::AgentMessage { agent_id, text } => {
                self.last_poll = Some(Instant::now());
                // Ensure a sprite exists in case this agent's
                // `session.message` arrives before `agents.list`
                // populates the roster (race on first connect).
                self.ensure_agent(&agent_id, AgentKind::Main);
                // Reply lands → activity indicator goes away.
                self.chat_activities.remove(&agent_id);
                // Bump unread count if the operator isn't actively
                // watching this conversation: either on a different
                // tab, or on Chat but looking at a different agent.
                let watching = self.nav == NavItem::Chat && self.selected_chat_agent == agent_id;
                if !watching {
                    *self.unread.entry(agent_id.clone()).or_insert(0) += 1;
                }
                // Chat log gets the full verbatim text — the transcript
                // view preserves line breaks and length. Bubble uses the
                // single-line snippet form because sprites can't host
                // multi-paragraph content.
                push_chat(
                    chat_log_mut(&mut self.chat_logs, &agent_id),
                    ChatMessage::assistant(&text),
                );
                let snippet = clean_bubble_text(&text, 80);
                if snippet.is_empty() {
                    tracing::debug!(
                        agent = %agent_id.as_str(),
                        "agent message empty after cleanup, skipping bubble",
                    );
                } else {
                    tracing::info!(
                        agent = %agent_id.as_str(),
                        preview = %snippet,
                        "agent message → bubble",
                    );
                    self.bubbles.push(ThoughtBubble::message(agent_id, snippet));
                }
            }
            WsEvent::AgentSilentTurn { agent_id } => {
                // Agent chose not to reply (`NO_REPLY` sentinel).
                // Nothing to render, but the run is done — clear the
                // "thinking…" row so the operator isn't left waiting
                // on a reply that's never coming.
                self.last_poll = Some(Instant::now());
                self.chat_activities.remove(&agent_id);
            }
            WsEvent::SessionUsageTimeseries {
                session_key,
                points,
            } => {
                tracing::info!(
                    key = %session_key,
                    count = points.len(),
                    "session usage timeseries applied",
                );
                let affects_active =
                    self.active_session_key.as_deref() == Some(session_key.as_str());
                // Ensure this session has a row-sparkline cache and
                // invalidate it so the mini-chart redraws with the
                // new points. `or_default` inserts an empty Cache
                // when this is the first time we've heard about
                // this session.
                self.row_sparkline_caches
                    .entry(session_key.clone())
                    .or_default()
                    .clear();
                self.session_usage.insert(session_key, points);
                if affects_active {
                    self.sparkline_cache.clear();
                }
            }
            WsEvent::SessionHistory {
                session_key,
                messages,
            } => {
                tracing::info!(
                    key = %session_key,
                    count = messages.len(),
                    "session history drill-in applied",
                );
                // Replace rather than append — the server is the
                // authority on session history.
                let log = self
                    .session_transcripts
                    .entry(session_key)
                    .or_insert_with(|| VecDeque::with_capacity(64));
                log.clear();
                for m in messages {
                    if log.len() >= CHAT_LOG_MAX {
                        log.pop_front();
                    }
                    log.push_back(m);
                }
            }
            WsEvent::ChatHistory { agent_id, messages } => {
                tracing::info!(
                    agent = %agent_id.as_str(),
                    count = messages.len(),
                    "chat history bootstrap applied",
                );
                // Replace rather than append — the server is the
                // authority on session history, and a reconnect
                // shouldn't stack duplicate history on top of what's
                // already rendered.
                let log = chat_log_mut(&mut self.chat_logs, &agent_id);
                log.clear();
                for m in messages {
                    if log.len() >= CHAT_LOG_MAX {
                        log.pop_front();
                    }
                    log.push_back(m);
                }
            }
            WsEvent::AgentToolInvoked { agent_id, text } => {
                self.last_poll = Some(Instant::now());
                let snippet = clean_bubble_text(&text, 80);
                // Capture the tool name for the activity-row display
                // — strip the leading "⚙ " prefix the WS layer added
                // so we render "using bash" rather than "using ⚙ bash".
                let tool_name = snippet.strip_prefix("⚙ ").unwrap_or(&snippet).to_string();
                if !snippet.is_empty() {
                    tracing::info!(
                        agent = %agent_id.as_str(),
                        preview = %snippet,
                        "tool invoke → bubble",
                    );
                    self.bubbles
                        .push(ThoughtBubble::tool(agent_id.clone(), snippet));
                }
                if !tool_name.is_empty() {
                    self.chat_activities.insert(
                        agent_id,
                        ChatActivityState {
                            kind: ChatActivity::Tool(tool_name),
                            since: Instant::now(),
                        },
                    );
                }
            }
            WsEvent::AgentActivity { agent_id, kind } => {
                self.last_poll = Some(Instant::now());
                let status = match kind {
                    ActivityKind::Thinking | ActivityKind::ToolCalling => AgentStatus::Running,
                    ActivityKind::Errored => AgentStatus::Error,
                };
                self.apply_status_update(agent_id.clone(), status);
                // Don't overwrite a Tool("bash") indicator with a
                // generic "thinking" — tool events are richer info.
                // Still upgrade Sending → Thinking / Errored because
                // those are the first signals we have that the agent
                // actually started running.
                let upgrade_to = match kind {
                    ActivityKind::Thinking | ActivityKind::ToolCalling => {
                        Some(ChatActivity::Thinking)
                    }
                    ActivityKind::Errored => {
                        // Error events clear any in-progress
                        // activity — the chat view renders the error
                        // as a bubble; a lingering "thinking…" row
                        // would misrepresent the state.
                        self.chat_activities.remove(&agent_id);
                        None
                    }
                };
                if let Some(next) = upgrade_to {
                    let keep_existing = matches!(
                        self.chat_activities.get(&agent_id),
                        Some(ChatActivityState {
                            kind: ChatActivity::Tool(_),
                            ..
                        })
                    );
                    if !keep_existing {
                        self.chat_activities.insert(
                            agent_id,
                            ChatActivityState {
                                kind: next,
                                since: Instant::now(),
                            },
                        );
                    }
                }
            }
            WsEvent::SessionsChanged => {
                self.last_poll = Some(Instant::now());
                tracing::trace!("sessions.changed");
            }
            WsEvent::SessionUsage(info) => {
                tracing::debug!(
                    session = %info.key,
                    total = ?info.total_tokens,
                    ctx = ?info.context_tokens,
                    "session usage",
                );
                self.sessions.insert(info.key.clone(), info);
            }
            WsEvent::ApprovalRequested(payload) => {
                self.last_poll = Some(Instant::now());
                self.notifier.approval_requested(&payload);
                let key = payload.id.clone().unwrap_or_else(|| {
                    // No id — best-effort key from tool+summary so
                    // resolved(null-id) still matches something.
                    format!(
                        "{}:{}",
                        payload.tool.as_deref().unwrap_or("?"),
                        payload.summary.as_deref().unwrap_or(""),
                    )
                });
                self.pending_approvals.insert(key, payload);
            }
            WsEvent::UpdateAvailable(update) => {
                if let Some(ref u) = update {
                    self.notifier.update_available(u);
                }
                self.gateway_update = update;
            }
            WsEvent::LogTail(tail) => {
                // Log rollover on the server side invalidates our
                // ring — drop the old buffer and start fresh so the
                // view doesn't mix two files.
                if tail.reset {
                    self.log_lines.clear();
                }
                for line in tail.lines {
                    logs::push_line(&mut self.log_lines, LogLine::classify(line));
                }
            }
            WsEvent::PairRequestPending(req) => {
                if let Some(pr) = req.as_ref() {
                    tracing::info!(
                        request_id = %pr.request_id,
                        kind = ?pr.kind,
                        "pair-request filed",
                    );
                }
                self.pending_pair_request = req;
            }
            WsEvent::ApprovalResolved { id } => {
                self.last_poll = Some(Instant::now());
                self.notifier.approval_resolved(id.as_deref());
                if let Some(id) = id.as_deref() {
                    self.pending_approvals.remove(id);
                } else {
                    // Unidentified resolve — safest to clear all since we
                    // can't tell which survived.
                    self.pending_approvals.clear();
                }
            }
        }
    }

    /// Ensure a sprite exists in the roster for `id`. First-time seen
    /// IDs (cron rename on the gateway, a new channel provider) get a
    /// fresh `Agent` with a deterministic color and the canvas cache is
    /// cleared so the sprite is painted this frame.
    fn ensure_agent(&mut self, id: &AgentId, kind: AgentKind) {
        if self.roster.iter().any(|a| a.id == *id) {
            return;
        }
        let agent = match kind {
            AgentKind::Cron => Agent::cron(id.as_str()),
            AgentKind::Channel => Agent::channel(id.as_str()),
            // A second Main agent showing up via a route other than
            // `agents.list` (e.g. an agent-scoped session.message
            // event arriving before the list RPC returned) — seed a
            // minimal sprite now; the subsequent AgentsList event
            // will upgrade `display` with the persona name/emoji.
            AgentKind::Main => Agent::main_with_display(id.as_str(), id.as_str()),
        };
        tracing::info!(id = %id.as_str(), kind = ?kind, "roster: new agent");
        self.roster.push(agent);
        self.scene_cache.clear();
    }

    /// True when any sprite is in a state that drives per-frame
    /// redraws — a Running agent bobs, and any just-transitioned
    /// agent pulses a ring flash for [`TRANSITION_FLASH`].
    fn any_sprite_animating(&self, now: Instant) -> bool {
        if self
            .statuses
            .values()
            .any(|s| matches!(s, AgentStatus::Running))
        {
            return true;
        }
        self.transition_moments
            .values()
            .any(|t| now.saturating_duration_since(*t) < TRANSITION_FLASH)
    }

    /// True when the scene has sprites that cycle frames even at
    /// idle — Main humanoids and Crons both alternate walk frames
    /// slowly. Used to pick a medium tick rate (not flat-out 33ms,
    /// but fast enough to show the cycle).
    fn any_sprite_idle_cycling(&self) -> bool {
        self.roster.iter().any(|a| {
            matches!(a.kind, AgentKind::Main | AgentKind::Cron)
                && !matches!(
                    self.statuses
                        .get(&a.id)
                        .copied()
                        .unwrap_or(AgentStatus::Unknown),
                    AgentStatus::Disabled,
                )
        })
    }

    /// Best display string for the currently-selected chat agent —
    /// operator persona ("Sebastian 🦀") when known, falling back to
    /// the raw agent id on first paint before `agents.list` returns.
    fn selected_chat_display(&self) -> String {
        self.chat_agents
            .iter()
            .find(|a| a.id == self.selected_chat_agent.as_str())
            .map(AgentInfo::display_with_emoji)
            .unwrap_or_else(|| self.selected_chat_agent.as_str().to_string())
    }

    fn apply_status_update(&mut self, id: AgentId, next: AgentStatus) {
        let prev = self.statuses.get(&id).copied();
        if prev == Some(next) {
            return;
        }
        self.statuses.insert(id.clone(), next);
        self.transition_moments.insert(id.clone(), Instant::now());
        if let Some(text) = transition_text(prev, next) {
            self.bubbles.push(ThoughtBubble::new(id, text));
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let main = match self.nav {
            NavItem::Overview => self.overview(),
            NavItem::Agents => agents_view::view(agents_view::AgentsViewSnapshot {
                roster: &self.roster,
                statuses: &self.statuses,
                cron_details: &self.cron_details,
                cron_ids: &self.cron_ids,
                channel_details: &self.channel_details,
                active_model: self.active_model.as_deref(),
                sessions: &self.sessions,
                pending_resets: &self.pending_resets,
                expanded_errors: &self.expanded_errors,
            }),
            NavItem::Chat => chat_view::view(
                &self.chat_agents,
                &self.selected_chat_agent,
                self.chat_logs.get(&self.selected_chat_agent),
                self.chat_activities.get(&self.selected_chat_agent),
                &self.chat_input,
                self.connected,
                &self.unread,
            ),
            NavItem::Sessions => sessions_view::view(sessions_view::SessionsViewSnapshot {
                sessions: &self.sessions,
                active_session_key: self.active_session_key.as_deref(),
                transcripts: &self.session_transcripts,
                hydrated: &self.session_history_fetched,
                usage: &self.session_usage,
                sparkline_cache: &self.sparkline_cache,
                row_sparkline_caches: &self.row_sparkline_caches,
                connected: self.connected,
            }),
            NavItem::Logs => logs_view::view(
                self.log_lines.iter(),
                &self.log_filters,
                self.logs_auto_tail,
            ),
            NavItem::Settings => settings_view::view(settings_view::Snapshot {
                settings: &self.settings,
                form: &self.settings_form,
                first_run_incomplete: self.first_run_incomplete(),
                token_present: self.token_present,
                storage_location: secret_store::storage_location_hint(),
                connection_status: &self.connection_status,
            }),
        };

        let total_unread: usize = self.unread.values().copied().sum();
        // Stack: [tab-specific main] on top, global bottom strip
        // (pair notice, approvals, status bar) below. The bottom
        // strip used to live inside `overview()`, which meant
        // switching to Settings or any other tab hid critical state
        // like the pair-request notice with the approve-command.
        let main_with_strip = iced::widget::column![main, self.bottom_strip()].spacing(0);
        let base = iced::widget::container(
            iced::widget::row![sidebar::view(self.nav, total_unread), main_with_strip].spacing(0),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| iced::widget::container::Style {
            background: Some((*theme::SURFACE_0).into()),
            ..Default::default()
        });

        if !self.palette_open {
            return base.into();
        }

        // Palette is open — layer the overlay on top of the base
        // view via `stack`. Ranking happens here (once per render)
        // so the view stays a pure function of App state.
        let entries = self.palette_entries();
        let ranked = palette::rank(&entries, &self.palette_input);
        let selected = self.palette_selected.min(ranked.len().saturating_sub(1));
        let overlay = palette_view::view(&self.palette_input, entries, ranked, selected);

        iced::widget::stack![base, overlay]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn overview(&self) -> Element<'_, Message> {
        let scene = OfficeScene {
            roster: &self.roster,
            statuses: &self.statuses,
            bubbles: &self.bubbles,
            transition_moments: &self.transition_moments,
            cache: &self.scene_cache,
        };

        let canvas = Canvas::new(scene).width(Length::Fill).height(Length::Fill);
        let cards = agent_card::row_view(&self.roster, &self.statuses);

        iced::widget::column![
            iced::widget::container(canvas)
                .width(Length::Fill)
                .height(Length::FillPortion(3))
                .padding(iced::Padding::from(16)),
            cards,
            chat_input::view(
                &self.chat_input,
                self.connected,
                &self.selected_chat_display(),
            ),
        ]
        .spacing(0)
        .into()
    }

    /// Build the bottom strip — pair-request notice (when a pair is
    /// pending), pending-approvals panel, and the always-on status
    /// bar. These are global concerns, not Overview-specific, so the
    /// top-level `view` renders them under every tab's main content.
    fn bottom_strip(&self) -> Element<'_, Message> {
        let main_usage = self
            .sessions
            .get("agent:main:main")
            .and_then(|i| i.total_tokens.zip(i.context_tokens));
        let status = status_bar::view(status_bar::Snapshot {
            connected: self.connected,
            agents_tracked: self.statuses.len(),
            last_poll: self.last_poll,
            active_model: self.active_model.as_deref(),
            last_disconnect: self.last_disconnect.as_deref(),
            main_usage,
            pending_approvals: self.pending_approvals.len(),
            update: self
                .gateway_update
                .as_ref()
                .map(|u| (u.current.as_str(), u.latest.as_str())),
        });

        let mut col = iced::widget::column![].spacing(0);
        if let Some(req) = self.pending_pair_request.as_ref() {
            col = col.push(approvals::pair_request_notice(req));
        }
        if !self.pending_approvals.is_empty() {
            col = col.push(approvals::view(self.pending_approvals.iter()));
        }
        col.push(status).into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        // `OPENCLAW_MOCK=1` routes to the scripted fixture stream for UI
        // work without a live gateway; otherwise we resolve the gateway
        // URL (env > persisted settings) and attach a native WS
        // subscription keyed on the URL so URL changes auto-restart it.
        // When no URL is configured, the ws subscription stays idle
        // until the operator saves one via the Settings tab.
        let ws = if mock::enabled() {
            Subscription::run(mock::connect).map(Message::Ws)
        } else if let Some(gateway_url) = config::gateway_url(self.settings.gateway_url.as_deref())
        {
            let params = openclaw::ConnectParams {
                gateway_url,
                token: self.cached_token.clone(),
                save_nonce: self.save_nonce,
            };
            // Explicit fn-pointer cast — `openclaw::connect` returns
            // `impl Stream`, which is a fn-item not a fn-pointer, and
            // `Subscription::run_with` needs the latter. Coercing via
            // `as fn(…) -> _` lets the compiler pin down the opaque
            // return type to the one monomorphization we use here.
            let builder: fn(&openclaw::ConnectParams) -> _ = openclaw::connect;
            Subscription::run_with(params, builder).map(Message::Ws)
        } else {
            Subscription::none()
        };

        // Window + keyboard events both route through a single
        // `listen_with`. The filter must be a plain `fn` (no
        // captures), so palette-specific messages emitted here
        // are gated on `palette_open` inside their handlers
        // rather than via closure state.
        let app_events = iced::event::listen_with(global_event_filter);

        // Three-tier tick so the office feels alive without burning
        // CPU on a quiet scene:
        // - 33 ms while anything is actively moving (bob, flash,
        //   thought bubble fading) — fluid at ~30fps.
        // - 50 ms while sprites are idle-cycling frames (the lobster
        //   IDLE loop at 3 Hz × 7 frames). An earlier 200 ms interval
        //   produced a visible beat pattern: phase advances 0.6
        //   frames per tick, so some frames rendered twice while
        //   neighbours rendered once, reading as stutter rather
        //   than smooth animation. 50 ms × 3 Hz = 0.15 frames/tick,
        //   which samples the cycle finely enough to read smooth.
        // - 500 ms when the scene is truly static (all disabled or
        //   all Channels, which don't cycle frames).
        let now = Instant::now();
        let tick_interval = if !self.bubbles.is_empty() || self.any_sprite_animating(now) {
            Duration::from_millis(33)
        } else if self.any_sprite_idle_cycling() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(500)
        };
        let tick = time::every(tick_interval).map(|_| Message::Tick);

        Subscription::batch([ws, tick, app_events])
    }

    pub fn theme(&self) -> iced::Theme {
        theme::periclaw_theme()
    }
}

/// Turn raw assistant text into something that reads cleanly on a
/// single-line bubble. Strips markdown code fences, collapses
/// whitespace, drops common emphasis markers, then clips to `max`
/// Unicode scalars with a trailing `…` when truncated.
/// Append a message to the chat log, evicting the oldest entry when
/// the ring is full. Centralized so both the "user sent" and "agent
/// replied" paths share the same cap without copy-pasting.
fn push_chat(log: &mut VecDeque<ChatMessage>, msg: ChatMessage) {
    if log.len() >= CHAT_LOG_MAX {
        log.pop_front();
    }
    log.push_back(msg);
}

/// Get (or lazily create) the chat log for an agent. Centralizes the
/// "seen this agent for the first time" path so push / replace /
/// history bootstrap all allocate the same way.
fn chat_log_mut<'a>(
    logs: &'a mut HashMap<AgentId, VecDeque<ChatMessage>>,
    agent_id: &AgentId,
) -> &'a mut VecDeque<ChatMessage> {
    logs.entry(agent_id.clone())
        .or_insert_with(|| VecDeque::with_capacity(64))
}

/// Merge a cron-state delta onto the stored value — push events only
/// carry fields that changed, so a bare `finished` delta must not
/// wipe `nextRunAtMs` from the previous snapshot. `running` is the
/// one field we always copy since the lifecycle transition is the
/// whole point of the event.
fn merge_cron_state(dst: &mut CronState, src: &CronState) {
    dst.running = src.running;
    if src.next_run_at_ms.is_some() {
        dst.next_run_at_ms = src.next_run_at_ms;
    }
    if src.last_run_at_ms.is_some() {
        dst.last_run_at_ms = src.last_run_at_ms;
    }
    if src.last_status.is_some() {
        dst.last_status = src.last_status.clone();
    }
    if src.last_duration_ms.is_some() {
        dst.last_duration_ms = src.last_duration_ms;
    }
    if src.last_error.is_some() {
        dst.last_error = src.last_error.clone();
    }
}

fn clean_bubble_text(raw: &str, max: usize) -> String {
    let mut body: Vec<&str> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            // Fence open or close — drop entirely; the language tag
            // after ``` isn't interesting on a bubble either.
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        body.push(trimmed);
    }
    // Collapse runs of whitespace across the joined text and drop
    // markdown emphasis asterisks/underscores that would read as
    // literal characters in the bubble font.
    let joined = body.join(" ");
    let mut compact = String::with_capacity(joined.len());
    let mut prev_space = false;
    for ch in joined.chars() {
        match ch {
            // Drop markdown emphasis (`*bold*`) and inline-code
            // backticks. Leave `_` alone — legitimate identifiers
            // like `x86_64` or `session_key` contain it.
            '*' | '`' => continue,
            c if c.is_whitespace() => {
                if !prev_space && !compact.is_empty() {
                    compact.push(' ');
                }
                prev_space = true;
            }
            c => {
                compact.push(c);
                prev_space = false;
            }
        }
    }
    let trimmed = compact.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Global event filter covering both window-level changes (resize,
/// move) for state persistence AND keyboard shortcuts for the
/// command palette. Must be a bare `fn` (no captures) because
/// `iced::event::listen_with` takes a function pointer — palette
/// messages are emitted unconditionally and gated in their
/// handlers on `palette_open`.
fn global_event_filter(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    use iced::keyboard::key::Named;
    match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.width, size.height))
        }
        iced::Event::Window(iced::window::Event::Moved(point)) => {
            Some(Message::WindowMoved(point.x, point.y))
        }
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            // ⌘K / Ctrl+K opens-or-closes the palette from
            // anywhere in the app. Test `command()` rather than
            // splitting cases per-platform — Iced normalizes
            // this for us.
            if modifiers.command()
                && matches!(&key, iced::keyboard::Key::Character(s) if s.as_ref() == "k")
            {
                return Some(Message::PaletteToggle);
            }
            // Navigation keys are only meaningful inside the
            // palette — the handlers no-op when it's closed so
            // emitting unconditionally is safe.
            match key {
                iced::keyboard::Key::Named(Named::Escape) => Some(Message::PaletteClose),
                iced::keyboard::Key::Named(Named::ArrowUp) => Some(Message::PaletteMove(-1)),
                iced::keyboard::Key::Named(Named::ArrowDown) => Some(Message::PaletteMove(1)),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod bubble_cleanup_tests {
    use super::clean_bubble_text;

    #[test]
    fn strips_code_fences_and_joins_lines() {
        let input = "```\nLinux host 6.8.0-110-generic\nx86_64 GNU/Linux\n```";
        assert_eq!(
            clean_bubble_text(input, 80),
            "Linux host 6.8.0-110-generic x86_64 GNU/Linux"
        );
    }

    #[test]
    fn drops_markdown_emphasis_and_collapses_whitespace() {
        let input = "Done with **step 1** and    *step 2*.";
        assert_eq!(clean_bubble_text(input, 80), "Done with step 1 and step 2.");
    }

    #[test]
    fn truncates_with_ellipsis() {
        let input = "a".repeat(200);
        let out = clean_bubble_text(&input, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn empty_after_cleanup_returns_empty() {
        assert_eq!(clean_bubble_text("```\n```", 80), "");
        assert_eq!(clean_bubble_text("   ", 80), "");
    }
}
