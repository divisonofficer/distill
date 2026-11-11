//! Program Status Protocol (OSC 7501) reporter.
//!
//! Spec: <https://www.superlogical.com/rex/docs/build/program-status>
//!
//! Terminal hosts that track coding agents (Orca, Rex, ...) otherwise have to guess Distill's
//! state from the tab title. The title only looks like an agent while the braille spinner is
//! showing, so between turns the pane degrades to "plain terminal". Reporting the state
//! explicitly keeps the pane recognized as an agent for the whole session: `idle` at startup,
//! `working`/`blocked` during a turn, `done`/`error`/`idle` when it ends, and `clear` on quit.
//!
//! Terminals MUST ignore unknown OSCs (per the spec), so the reports are emitted unconditionally.

use base64::Engine as _;

use super::title::{self, TitleState};
use super::tmux;
use crate::terminal::TerminalContext;

/// `app` id sent with every report. Orca maps `grok-build` to its built-in Grok agent
/// (icon, labels, status rows), which is the closest match for this Grok-derived harness.
pub const DEFAULT_PROGRAM_STATUS_APP: &str = "grok-build";

const OSC_PREFIX: &str = "\x1b]7501;";
const OSC_ST: &str = "\x1b\\";
/// Hosts shorten `msg` anyway (Orca keeps 160 chars of a live tool label), and a cap keeps
/// every report far below the spec's 2048-byte decoded `msg` limit.
const MAX_MSG_CHARS: usize = 160;
const MAX_APP_LEN: usize = 32;

/// How a turn ended, recorded by the turn-end handler and reported on the next at-rest update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// Finished normally: reported as `done` so the host can flag an unseen result.
    Completed,
    /// Failed: reported as `error`.
    Failed,
    /// Interrupted or cancelled by the user: the spec asks for `idle`.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Report {
    Idle,
    Working { msg: Option<String> },
    Blocked { msg: Option<String> },
    Done { msg: Option<String> },
    Error { msg: Option<String> },
    Clear,
}

impl Report {
    fn is_live(&self) -> bool {
        matches!(self, Report::Working { .. } | Report::Blocked { .. })
    }
}

pub struct ProgramStatusReporter {
    enabled: bool,
    app: String,
    /// The last report written, used to suppress identical re-sends (every tick asks).
    last: Option<Report>,
    /// Set when a turn ends; consumed by the next at-rest update.
    pending_outcome: Option<(TurnOutcome, Option<String>)>,
}

impl ProgramStatusReporter {
    pub fn new(enabled: bool, app: &str) -> Self {
        let app = if is_valid_app(app) {
            app.to_owned()
        } else {
            tracing::warn!(
                app,
                "invalid ui.notifications.program_status_app; using {DEFAULT_PROGRAM_STATUS_APP}"
            );
            DEFAULT_PROGRAM_STATUS_APP.to_owned()
        };
        Self {
            enabled,
            app,
            last: None,
            pending_outcome: None,
        }
    }

    /// Record how the current turn ended. The next update that finds the agent at rest reports it.
    pub fn note_turn_finished(&mut self, outcome: TurnOutcome, msg: Option<&str>) {
        if self.enabled {
            self.pending_outcome = Some((outcome, msg.map(str::to_owned)));
        }
    }

    /// Derive the report for this tick and append its escape to `buf` when it changed.
    pub fn update_into(&mut self, state: &TitleState<'_>, ctx: &TerminalContext, buf: &mut String) {
        if !self.enabled {
            return;
        }
        let next = self.desired_report(state);
        self.emit_into(next, ctx, buf);
    }

    /// Remove this program's record so the host stops treating the pane as an agent after quit.
    pub fn shutdown_into(&mut self, ctx: &TerminalContext, buf: &mut String) {
        if !self.enabled || matches!(self.last, None | Some(Report::Clear)) {
            return;
        }
        self.pending_outcome = None;
        self.emit_into(Report::Clear, ctx, buf);
    }

    fn desired_report(&mut self, state: &TitleState<'_>) -> Report {
        if state.has_pending_permissions {
            // A new turn (or the same one) is live again: any recorded outcome is stale.
            self.pending_outcome = None;
            return Report::Blocked {
                msg: Some("Approval required".to_owned()),
            };
        }
        if state.is_busy || state.activity.is_some() {
            self.pending_outcome = None;
            let msg = state.activity.map(|activity| {
                let mut label = String::new();
                title::write_activity(&mut label, activity);
                label
            });
            return Report::Working { msg };
        }
        if let Some((outcome, msg)) = self.pending_outcome.take() {
            return match outcome {
                TurnOutcome::Completed => Report::Done { msg },
                TurnOutcome::Failed => Report::Error { msg },
                TurnOutcome::Cancelled => Report::Idle,
            };
        }
        match &self.last {
            // The turn ended without a recorded outcome (e.g. the active agent changed): at rest.
            Some(last) if last.is_live() => Report::Idle,
            // Keep an unseen done/error (and idle) as is; the host decides when it was seen.
            Some(last) if *last != Report::Clear => last.clone(),
            _ => Report::Idle,
        }
    }

    fn emit_into(&mut self, next: Report, ctx: &TerminalContext, buf: &mut String) {
        if self.last.as_ref() == Some(&next) {
            return;
        }
        let sequence = build_report(&next, &self.app);
        if tmux::passthrough_available(ctx) {
            buf.push_str(&tmux::tmux_passthrough(&sequence));
        } else {
            buf.push_str(&sequence);
        }
        self.last = Some(next);
    }
}

/// `[A-Za-z0-9_.+-]{1,32}` per the spec; anything else would be dropped by the host.
fn is_valid_app(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= MAX_APP_LEN
        && app
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
}

fn build_report(report: &Report, app: &str) -> String {
    let (state, kind, msg) = match report {
        Report::Idle => ("idle", None, None),
        Report::Working { msg } => ("working", None, msg.as_deref()),
        Report::Blocked { msg } => ("blocked", Some("permission"), msg.as_deref()),
        Report::Done { msg } => ("done", None, msg.as_deref()),
        Report::Error { msg } => ("error", None, msg.as_deref()),
        Report::Clear => return format!("{OSC_PREFIX}state=clear{OSC_ST}"),
    };
    let mut out = format!("{OSC_PREFIX}state={state}");
    if let Some(kind) = kind {
        out.push_str(":kind=");
        out.push_str(kind);
    }
    out.push_str(":app=");
    out.push_str(app);
    if let Some(msg) = msg.and_then(sanitize_msg) {
        out.push_str(":msg=");
        out.push_str(&base64::engine::general_purpose::STANDARD.encode(msg));
    }
    out.push_str(OSC_ST);
    out
}

/// One line without control characters (a report carrying any is discarded whole by the host),
/// capped at [`MAX_MSG_CHARS`]. `None` when nothing printable is left.
fn sanitize_msg(msg: &str) -> Option<String> {
    let mut out = String::new();
    let mut pending_space = false;
    let mut count = 0;
    for ch in msg.chars() {
        if ch.is_control() || ch.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if count + usize::from(pending_space) >= MAX_MSG_CHARS {
            out.push('\u{2026}');
            break;
        }
        if pending_space {
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        out.push(ch);
        count += 1;
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::tracker::TurnActivity;
    use crate::terminal::MultiplexerKind;

    fn state<'a>(is_busy: bool, perms: bool) -> TitleState<'a> {
        TitleState {
            session_name: None,
            model: None,
            activity: None,
            has_pending_permissions: perms,
            cwd: None,
            turn_elapsed: None,
            is_busy,
            focused: true,
        }
    }

    fn tick(r: &mut ProgramStatusReporter, s: &TitleState<'_>) -> String {
        let mut buf = String::new();
        r.update_into(s, &TerminalContext::default(), &mut buf);
        buf
    }

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn first_tick_reports_idle_so_the_host_sees_an_agent_before_any_turn() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        assert_eq!(
            tick(&mut r, &state(false, false)),
            "\x1b]7501;state=idle:app=grok-build\x1b\\"
        );
        assert_eq!(
            tick(&mut r, &state(false, false)),
            "",
            "unchanged state is not re-sent"
        );
    }

    #[test]
    fn busy_reports_working_with_activity_label() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        let activity = TurnActivity::Thinking;
        let s = TitleState {
            activity: Some(&activity),
            ..state(true, false)
        };
        assert_eq!(
            tick(&mut r, &s),
            format!(
                "\x1b]7501;state=working:app=grok-build:msg={}\x1b\\",
                b64("Thinking")
            )
        );
        assert_eq!(tick(&mut r, &s), "");
    }

    #[test]
    fn busy_without_activity_reports_working_without_msg() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        assert_eq!(
            tick(&mut r, &state(true, false)),
            "\x1b]7501;state=working:app=grok-build\x1b\\"
        );
    }

    #[test]
    fn pending_permission_reports_blocked_permission() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        assert_eq!(
            tick(&mut r, &state(true, true)),
            format!(
                "\x1b]7501;state=blocked:kind=permission:app=grok-build:msg={}\x1b\\",
                b64("Approval required")
            )
        );
    }

    #[test]
    fn completed_turn_reports_done_and_stays_done_while_idle() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        tick(&mut r, &state(true, false));
        r.note_turn_finished(TurnOutcome::Completed, Some("Turn complete in 3s."));
        assert_eq!(
            tick(&mut r, &state(false, false)),
            format!(
                "\x1b]7501;state=done:app=grok-build:msg={}\x1b\\",
                b64("Turn complete in 3s.")
            )
        );
        assert_eq!(
            tick(&mut r, &state(false, false)),
            "",
            "done is kept, not downgraded"
        );
    }

    #[test]
    fn failed_turn_reports_error() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        tick(&mut r, &state(true, false));
        r.note_turn_finished(TurnOutcome::Failed, Some("Error: boom"));
        assert_eq!(
            tick(&mut r, &state(false, false)),
            format!(
                "\x1b]7501;state=error:app=grok-build:msg={}\x1b\\",
                b64("Error: boom")
            )
        );
    }

    #[test]
    fn cancelled_turn_and_unrecorded_turn_end_report_idle() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        tick(&mut r, &state(true, false));
        r.note_turn_finished(TurnOutcome::Cancelled, None);
        assert_eq!(
            tick(&mut r, &state(false, false)),
            "\x1b]7501;state=idle:app=grok-build\x1b\\"
        );

        tick(&mut r, &state(true, false));
        assert_eq!(
            tick(&mut r, &state(false, false)),
            "\x1b]7501;state=idle:app=grok-build\x1b\\"
        );
    }

    #[test]
    fn outcome_is_dropped_when_the_next_turn_is_already_running() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        tick(&mut r, &state(true, false));
        r.note_turn_finished(TurnOutcome::Completed, Some("first"));
        // A queued prompt started before the next tick: keep working, forget the stale outcome.
        assert_eq!(tick(&mut r, &state(true, false)), "");
        assert_eq!(
            tick(&mut r, &state(false, false)),
            "\x1b]7501;state=idle:app=grok-build\x1b\\"
        );
    }

    #[test]
    fn shutdown_clears_the_record_once() {
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        let ctx = TerminalContext::default();
        let mut buf = String::new();
        r.shutdown_into(&ctx, &mut buf);
        assert_eq!(buf, "", "nothing reported, nothing to clear");

        tick(&mut r, &state(false, false));
        r.shutdown_into(&ctx, &mut buf);
        assert_eq!(buf, "\x1b]7501;state=clear\x1b\\");
        buf.clear();
        r.shutdown_into(&ctx, &mut buf);
        assert_eq!(buf, "");
    }

    #[test]
    fn disabled_reporter_emits_nothing() {
        let mut r = ProgramStatusReporter::new(false, DEFAULT_PROGRAM_STATUS_APP);
        assert_eq!(tick(&mut r, &state(true, false)), "");
        r.note_turn_finished(TurnOutcome::Completed, Some("x"));
        assert_eq!(tick(&mut r, &state(false, false)), "");
        let mut buf = String::new();
        r.shutdown_into(&TerminalContext::default(), &mut buf);
        assert_eq!(buf, "");
    }

    #[test]
    fn custom_app_is_used_and_invalid_app_falls_back() {
        let mut r = ProgramStatusReporter::new(true, "distill");
        assert_eq!(
            tick(&mut r, &state(false, false)),
            "\x1b]7501;state=idle:app=distill\x1b\\"
        );
        for bad in ["", "has space", "colon:app", "a".repeat(33).as_str()] {
            let mut r = ProgramStatusReporter::new(true, bad);
            assert_eq!(
                tick(&mut r, &state(false, false)),
                "\x1b]7501;state=idle:app=grok-build\x1b\\",
                "{bad:?} must fall back"
            );
        }
    }

    #[test]
    fn msg_is_single_line_and_capped() {
        assert_eq!(sanitize_msg("a\nb\t\x1bc"), Some("a b c".to_owned()));
        assert_eq!(sanitize_msg("  \n "), None);
        let long = "x".repeat(500);
        let capped = sanitize_msg(&long).expect("non-empty");
        assert_eq!(capped.chars().count(), MAX_MSG_CHARS + 1);
        assert!(capped.ends_with('\u{2026}'));
    }

    #[test]
    fn reports_are_wrapped_for_tmux_passthrough() {
        let ctx = TerminalContext {
            multiplexer: MultiplexerKind::Tmux,
            tmux_version: Some("tmux 3.3".into()),
            ..Default::default()
        };
        let mut r = ProgramStatusReporter::new(true, DEFAULT_PROGRAM_STATUS_APP);
        let mut buf = String::new();
        r.update_into(&state(false, false), &ctx, &mut buf);
        assert_eq!(
            buf,
            "\x1bPtmux;\x1b\x1b]7501;state=idle:app=grok-build\x1b\x1b\\\x1b\\"
        );
    }
}
