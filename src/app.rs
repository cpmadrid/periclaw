//! Top-level app state, Message, update, view.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use iced::widget::{Canvas, canvas};
use iced::{Element, Length, Subscription, Task, time};

use crate::domain::{Agent, AgentId, AgentKind, AgentStatus, agent};
use crate::logs::{self, LogFilters, LogLine, LogSeverity};
use crate::net::events::{ActivityKind, GatewayUpdate};
use crate::net::rpc::{
    AgentInfo, ApprovalEventPayload, Channel, CronState, SessionInfo, SessionUsagePoint,
};
use crate::net::{WsEvent, events, mock, openclaw};
use crate::scene::{OfficeScene, ThoughtBubble, transition_text};
use crate::ui::chat_view::ChatMessage;
use crate::ui::{
    agent_card, agents_view, approvals, chat_input, chat_view, logs_view, sessions_view, sidebar,
    status_bar, theme,
};
use crate::ui_state::{self, UiState, WindowState};

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
    /// Operator clicked a session card in the Sessions tab. Updates
    /// `active_session_key` and kicks off `chat.history` for that
    /// session if the transcript isn't already cached for this
    /// connection. Persisted via `UiState` so the selection survives
    /// a relaunch.
    SessionSelected(String),
    /// Window was resized by the OS / compositor. Carries logical
    /// pixels. Debounced and persisted on the next `Tick` so a drag
    /// doesn't hammer the state file.
    WindowResized(f32, f32),
    /// Window was moved. Some compositors never emit this (Wayland
    /// hides window coordinates from the client), in which case we
    /// simply don't restore position on next launch.
    WindowMoved(f32, f32),
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
    /// Non-None when the gateway has filed a scope-upgrade
    /// pair-request for this device and is waiting on the operator
    /// to approve it (`openclaw devices approve <id>`). Surfaced in
    /// the approvals panel area so the fix is visible.
    pub scope_upgrade_pending: Option<String>,
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
    /// Cache for the Sessions tab's token-usage sparkline. Cleared
    /// whenever the active session's points change so the canvas
    /// re-renders only when there's something new to draw.
    pub sparkline_cache: canvas::Cache,
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
        let nav = state
            .tab
            .as_deref()
            .and_then(ui_state::nav_from_str)
            .unwrap_or(NavItem::Overview);
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
            scope_upgrade_pending: None,
            cron_details: HashMap::new(),
            cron_ids: HashMap::new(),
            channel_details: HashMap::new(),
            log_lines: VecDeque::with_capacity(2048),
            log_filters: LogFilters::default(),
            logs_auto_tail: true,
            transition_moments: HashMap::new(),
            scene_cache: canvas::Cache::default(),
            sparkline_cache: canvas::Cache::default(),
            chat_input: String::new(),
            chat_logs: HashMap::new(),
            chat_agents: Vec::new(),
            selected_chat_agent,
            history_fetched: HashSet::new(),
            chat_activities: HashMap::new(),
            pending_resets: HashMap::new(),
            session_transcripts: HashMap::new(),
            session_history_fetched: HashSet::new(),
            active_session_key: state.active_session_key.filter(|s| !s.is_empty()),
            session_usage: HashMap::new(),
            pending_window: state.window,
            pending_window_since: None,
        }
    }

    /// Snapshot the bits of the app we persist across launches.
    fn ui_state_snapshot(&self) -> UiState {
        UiState {
            tab: Some(ui_state::nav_to_str(self.nav).to_string()),
            selected_agent: Some(self.selected_chat_agent.as_str().to_string()),
            active_session_key: self.active_session_key.clone(),
            window: self.pending_window,
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::NavClicked(item) => {
                if self.nav != item {
                    self.nav = item;
                    ui_state::save(&self.ui_state_snapshot());
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
                // reconnect and send a fresh `ScopeUpgradePending` if
                // still unpaired, or `Connected` if the approval took
                // effect.
                self.scope_upgrade_pending = None;
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
                tracing::info!("WS connected");
            }
            WsEvent::Disconnected(reason) => {
                self.connected = false;
                self.last_disconnect = Some(reason.clone());
                // Reset the "hydrated this connection" set so the
                // next successful connect re-pulls chat.history as
                // the operator touches each agent. Logs themselves
                // are kept so the visible transcript survives a blip.
                self.history_fetched.clear();
                // Session drill-in transcripts follow the same
                // rule — keep them rendered across a blip, but
                // re-pull on reopen after reconnect.
                self.session_history_fetched.clear();
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
            WsEvent::ScopeUpgradePending(request_id) => {
                if request_id.is_some() {
                    tracing::info!(
                        request_id = ?request_id,
                        "scope-upgrade pair-request filed",
                    );
                }
                self.scope_upgrade_pending = request_id;
            }
            WsEvent::ApprovalResolved { id } => {
                self.last_poll = Some(Instant::now());
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
            }),
            NavItem::Chat => chat_view::view(
                &self.chat_agents,
                &self.selected_chat_agent,
                self.chat_logs.get(&self.selected_chat_agent),
                self.chat_activities.get(&self.selected_chat_agent),
                &self.chat_input,
                self.connected,
            ),
            NavItem::Sessions => sessions_view::view(sessions_view::SessionsViewSnapshot {
                sessions: &self.sessions,
                active_session_key: self.active_session_key.as_deref(),
                transcripts: &self.session_transcripts,
                hydrated: &self.session_history_fetched,
                usage: &self.session_usage,
                sparkline_cache: &self.sparkline_cache,
                connected: self.connected,
            }),
            NavItem::Logs => logs_view::view(
                self.log_lines.iter(),
                &self.log_filters,
                self.logs_auto_tail,
            ),
            NavItem::Settings => coming_soon("Settings"),
        };

        iced::widget::container(iced::widget::row![sidebar::view(self.nav), main].spacing(0))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| iced::widget::container::Style {
                background: Some((*theme::SURFACE_0).into()),
                ..Default::default()
            })
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

        // The approvals panel is a no-op row (empty iterator) when
        // nothing's pending, so we can always include it in the
        // layout without case-splitting on length.
        let approvals_panel = if self.pending_approvals.is_empty() {
            None
        } else {
            Some(approvals::view(self.pending_approvals.iter()))
        };
        let scope_notice = self
            .scope_upgrade_pending
            .as_deref()
            .map(approvals::scope_upgrade_notice);

        let mut col = iced::widget::column![
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
        .spacing(0);
        if let Some(notice) = scope_notice {
            col = col.push(notice);
        }
        if let Some(panel) = approvals_panel {
            col = col.push(panel);
        }
        col.push(status).into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        // `OPENCLAW_MOCK=1` routes to the scripted fixture stream for UI
        // work without a live gateway; otherwise we run the native WS.
        let ws = if mock::enabled() {
            Subscription::run(mock::connect).map(Message::Ws)
        } else {
            Subscription::run(openclaw::connect).map(Message::Ws)
        };

        // Window move/resize events route into the debounced save
        // path. `listen_with` wants a plain `fn` — the filter body
        // stays outside the closure so it can stay that shape.
        let window_events = iced::event::listen_with(window_event_filter);

        // Three-tier tick so the office feels alive without burning
        // CPU on a quiet scene:
        // - 33 ms while anything is actively moving (bob, flash,
        //   thought bubble fading) — fluid at ~30fps.
        // - 200 ms when sprites are just idle-cycling walk frames
        //   (Main + Cron alternate at 0.6–1 Hz). A 5fps refresh is
        //   plenty to show the frame change without a visible stutter.
        // - 500 ms when the scene is truly static (all disabled or
        //   all Channels, which don't cycle frames).
        let now = Instant::now();
        let tick_interval = if !self.bubbles.is_empty() || self.any_sprite_animating(now) {
            Duration::from_millis(33)
        } else if self.any_sprite_idle_cycling() {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(500)
        };
        let tick = time::every(tick_interval).map(|_| Message::Tick);

        Subscription::batch([ws, tick, window_events])
    }

    pub fn theme(&self) -> iced::Theme {
        theme::mission_control_theme()
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

/// Narrow the global Iced event stream to just the two window-level
/// changes we care about for state persistence. Must be a bare `fn`
/// (no captures) because `iced::event::listen_with` takes a function
/// pointer.
fn window_event_filter(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::Resized(size)) => {
            Some(Message::WindowResized(size.width, size.height))
        }
        iced::Event::Window(iced::window::Event::Moved(point)) => {
            Some(Message::WindowMoved(point.x, point.y))
        }
        _ => None,
    }
}

fn coming_soon(title: &'static str) -> Element<'static, Message> {
    iced::widget::center(
        iced::widget::column![
            iced::widget::text(title).size(24).color(*theme::FOREGROUND),
            iced::widget::text("coming soon")
                .size(13)
                .color(*theme::MUTED),
        ]
        .spacing(8)
        .align_x(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(iced::Padding::from(24))
    .into()
}

#[cfg(test)]
mod bubble_cleanup_tests {
    use super::clean_bubble_text;

    #[test]
    fn strips_code_fences_and_joins_lines() {
        let input = "```\nLinux ubu-3xdv 6.8.0-110-generic\nx86_64 GNU/Linux\n```";
        assert_eq!(
            clean_bubble_text(input, 80),
            "Linux ubu-3xdv 6.8.0-110-generic x86_64 GNU/Linux"
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
