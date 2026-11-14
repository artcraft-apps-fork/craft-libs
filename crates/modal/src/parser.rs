//! Stateful modal sequence parser (`[register] [count1] [operator] [count2] motion/object`).

use crate::action::{ModalAction, Mode, Motion, Operator, Target};
use crate::defaults::default_tables;
use crate::rc::{ModalOptions, RcEval, RcWarning, load_user_rc};
use crate::token::{KeyCode, KeyToken, NamedKey, format_tokens};
use crate::trie::{KeyTables, MapDomain, Rhs, TrieLookup};

const MAX_REMAP_DEPTH: usize = 16;
const MAX_COUNT: usize = 99_999;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharWait {
    Register,
    FindChar { forward: bool, till: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingOp {
    op: Operator,
    double_tail: KeyToken,
}

/// Snapshot of modal engine state for UI status bars / badges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModalStatus {
    pub enabled: bool,
    pub show_badge: bool,
    pub mode: Mode,
    pub pending_keys: String,
}

/// Outcome of feeding a single key event to [`ModalEngine`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedOutcome {
    /// Key was NOT handled by the modal layer; let the host's native key handler process it.
    PassThrough,
    /// Key was consumed by the modal layer (e.g., prefix buffered or count accumulated).
    Consumed(Vec<ModalAction>),
}

/// Reusable modal keybinding engine (Normal / Insert / Visual + `.vimrc` / `.ideavimrc`).
#[derive(Clone, Debug)]
pub struct ModalEngine {
    app_id: String,
    pub options: ModalOptions,
    pub tables: KeyTables,
    mode: Mode,
    register: Option<char>,
    count1: Option<usize>,
    count2: Option<usize>,
    pending_op: Option<PendingOp>,
    char_wait: Option<CharWait>,
    seq_buf: Vec<KeyToken>,
    seq_started_at: Option<f64>,
    ambiguous_rhs: Option<Rhs>,
    last_find_char: Option<(char, bool, bool)>,
    warnings: Vec<RcWarning>,
}

impl ModalEngine {
    /// Initialize engine for `app_id` (e.g., `"pdfcraft"`, `"wordcraft"`, `"gridcraft"`),
    /// loading default Vim/Evil tables followed by user RC files.
    pub fn new(app_id: impl Into<String>) -> Self {
        let app_id = app_id.into();
        let mut options = ModalOptions::default();
        let mut tables = default_tables();
        let warnings = load_user_rc(&app_id, &mut options, &mut tables);
        let mode = options.start_mode;
        Self {
            app_id,
            options,
            tables,
            mode,
            register: None,
            count1: None,
            count2: None,
            pending_op: None,
            char_wait: None,
            seq_buf: Vec::new(),
            seq_started_at: None,
            ambiguous_rhs: None,
            last_find_char: None,
            warnings,
        }
    }

    /// Reload defaults and user RC files from disk.
    pub fn reload_rc(&mut self) -> &[RcWarning] {
        let mut options = ModalOptions::default();
        let mut tables = default_tables();
        self.warnings = load_user_rc(&self.app_id, &mut options, &mut tables);
        self.options = options;
        self.tables = tables;
        self.reset_pending();
        &self.warnings
    }

    /// Evaluate an in-memory RC snippet on top of the current configuration.
    pub fn apply_rc_str(&mut self, script: &str, source_name: &str) -> Vec<RcWarning> {
        let mut eval = RcEval::new(&self.app_id, &mut self.options, &mut self.tables);
        eval.eval_str(script, source_name, None);
        let out = eval.warnings.clone();
        self.warnings.extend(eval.warnings);
        self.reset_pending();
        out
    }

    pub fn warnings(&self) -> &[RcWarning] {
        &self.warnings
    }

    pub const fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            self.reset_pending();
        }
    }

    pub fn reset_pending(&mut self) {
        self.register = None;
        self.count1 = None;
        self.count2 = None;
        self.pending_op = None;
        self.char_wait = None;
        self.seq_buf.clear();
        self.seq_started_at = None;
        self.ambiguous_rhs = None;
    }

    pub fn has_pending_state(&self) -> bool {
        self.register.is_some()
            || self.count1.is_some()
            || self.count2.is_some()
            || self.pending_op.is_some()
            || self.char_wait.is_some()
            || !self.seq_buf.is_empty()
    }

    /// Deadline (in seconds on the same clock passed to `feed_key`) when a buffered sequence
    /// or ambiguous mapping should time out, if any.
    pub fn pending_deadline(&self) -> Option<f64> {
        let started = self.seq_started_at?;
        if self.seq_buf.is_empty() {
            return None;
        }
        Some(started + (self.options.timeout_ms as f64) / 1000.0)
    }

    /// Resolve expired ambiguous prefixes (`ExactAndPrefix`, or buffered `Insert` chords like `jk`).
    /// Returns `(actions_to_run, buffered_insert_tokens_to_replay)`.
    pub fn flush_timeout(&mut self, now: f64) -> (Vec<ModalAction>, Vec<KeyToken>) {
        let Some(deadline) = self.pending_deadline() else {
            return (Vec::new(), Vec::new());
        };
        if now < deadline {
            return (Vec::new(), Vec::new());
        }
        if let Some(rhs) = self.ambiguous_rhs.take() {
            self.seq_buf.clear();
            self.seq_started_at = None;
            let mut actions = Vec::new();
            self.execute_rhs(rhs, 0, now, &mut actions);
            return (actions, Vec::new());
        }
        if self.mode == Mode::Insert {
            let replay = std::mem::take(&mut self.seq_buf);
            self.seq_started_at = None;
            return (Vec::new(), replay);
        }
        self.reset_pending();
        (Vec::new(), Vec::new())
    }

    /// Status summary for rendering a mode / pending-keys indicator.
    pub fn status(&self) -> ModalStatus {
        let mut pending = String::new();
        if let Some(reg) = self.register {
            pending.push('"');
            pending.push(reg);
        }
        if let Some(c1) = self.count1 {
            pending.push_str(&c1.to_string());
        }
        if let Some(op) = &self.pending_op {
            pending.push_str(op_symbol(op.op));
        }
        if let Some(c2) = self.count2 {
            pending.push_str(&c2.to_string());
        }
        if !self.seq_buf.is_empty() {
            pending.push_str(&format_tokens(&self.seq_buf));
        }
        ModalStatus { enabled: self.options.enabled, show_badge: self.options.show_mode_badge, mode: self.mode, pending_keys: pending }
    }

    /// Feed one normalized [`KeyToken`] at timestamp `now` (seconds).
    pub fn feed_key(&mut self, token: KeyToken, now: f64) -> FeedOutcome {
        if !self.options.enabled {
            return FeedOutcome::PassThrough;
        }
        let tok = token.normalized();
        if self.mode == Mode::Insert {
            return self.feed_insert(tok, now);
        }
        self.feed_command_mode(tok, now)
    }

    fn feed_insert(&mut self, tok: KeyToken, now: f64) -> FeedOutcome {
        self.seq_buf.push(tok);
        if self.seq_started_at.is_none() {
            self.seq_started_at = Some(now);
        }
        match self.tables.insert.lookup(&self.seq_buf).cloned_rhs() {
            OwnedLookup::Exact(rhs) => {
                self.seq_buf.clear();
                self.seq_started_at = None;
                self.ambiguous_rhs = None;
                let mut actions = Vec::new();
                self.execute_rhs(rhs, 0, now, &mut actions);
                FeedOutcome::Consumed(actions)
            }
            OwnedLookup::Prefix => FeedOutcome::Consumed(Vec::new()),
            OwnedLookup::ExactAndPrefix(rhs) => {
                self.ambiguous_rhs = Some(rhs);
                FeedOutcome::Consumed(Vec::new())
            }
            OwnedLookup::None => {
                if self.seq_buf.len() == 1 {
                    self.seq_buf.clear();
                    self.seq_started_at = None;
                    FeedOutcome::PassThrough
                } else {
                    // Earlier buffered Insert prefix failed to complete; clear and let current pass through.
                    self.seq_buf.clear();
                    self.seq_started_at = None;
                    self.ambiguous_rhs = None;
                    FeedOutcome::PassThrough
                }
            }
        }
    }

    fn feed_command_mode(&mut self, tok: KeyToken, now: f64) -> FeedOutcome {
        // `<Esc>` while any sequence/operator/count is pending cancels the pending state first.
        if tok == KeyToken::named(NamedKey::Escape) && self.has_pending_state() {
            self.reset_pending();
            return FeedOutcome::Consumed(Vec::new());
        }

        let mut actions = Vec::new();
        let consumed = self.step_token(tok, true, 0, now, &mut actions);
        if consumed { FeedOutcome::Consumed(actions) } else { FeedOutcome::PassThrough }
    }

    fn step_token(&mut self, tok: KeyToken, allow_remap: bool, depth: usize, now: f64, actions: &mut Vec<ModalAction>) -> bool {
        if depth > MAX_REMAP_DEPTH {
            self.reset_pending();
            return true;
        }
        // 1. Character argument wait (`f{c}`, `t{c}`, `"{reg}`).
        if let Some(wait) = self.char_wait.take() {
            return self.resolve_char_wait(wait, tok, actions);
        }

        // 2. When no multi-key prefix is mid-flight, check register, counts, and doubled operators.
        if self.seq_buf.is_empty() {
            if tok == KeyToken::plain_char('"') && self.register.is_none() && self.pending_op.is_none() {
                self.char_wait = Some(CharWait::Register);
                return true;
            }
            if self.try_accumulate_digit(tok) {
                return true;
            }
            if let Some(pending) = &self.pending_op
                && tok == pending.double_tail
            {
                let op = pending.op;
                let count = self.effective_count();
                let register = self.register;
                self.reset_pending();
                actions.push(ModalAction::Operator { op, target: Target::Linewise { count }, register });
                return true;
            }
        }

        // 3. Look up in active trie (or fallback default table when `allow_remap` is false).
        self.seq_buf.push(tok);
        if self.seq_started_at.is_none() {
            self.seq_started_at = Some(now);
        }
        let lookup = self.lookup_active_seq(allow_remap);
        match lookup {
            OwnedLookup::Exact(rhs) => {
                self.seq_buf.clear();
                self.seq_started_at = None;
                self.ambiguous_rhs = None;
                self.execute_rhs(rhs, depth, now, actions);
                true
            }
            OwnedLookup::Prefix => true,
            OwnedLookup::ExactAndPrefix(rhs) => {
                self.ambiguous_rhs = Some(rhs);
                true
            }
            OwnedLookup::None => self.handle_unmatched_token(tok, depth, now, actions),
        }
    }

    fn handle_unmatched_token(&mut self, tok: KeyToken, depth: usize, now: f64, actions: &mut Vec<ModalAction>) -> bool {
        // If we had an ambiguous match on the previous prefix, run that first and re-feed `tok`.
        if let Some(prev_rhs) = self.ambiguous_rhs.take() {
            self.seq_buf.clear();
            self.seq_started_at = None;
            self.execute_rhs(prev_rhs, depth + 1, now, actions);
            return self.step_token(tok, true, depth + 1, now, actions);
        }
        let single_token = self.seq_buf.len() == 1 && !self.has_non_seq_pending();
        self.reset_pending();
        // Let GUI command shortcuts (`Cmd+S`, `Ctrl+O`, `Ctrl+K`, `F1..F12`, `Alt+...`) pass through
        // to the host application's shortcut registry when unbound in Normal/Visual mode!
        if single_token && (tok.mods.has_command_mod() || tok.is_function_key()) {
            return false;
        }
        // Plain unbound keys in Normal/Visual mode are swallowed so they never leak as text edits
        // or single-letter tool switches in the underlying application.
        true
    }

    fn has_non_seq_pending(&self) -> bool {
        self.register.is_some() || self.count1.is_some() || self.count2.is_some() || self.pending_op.is_some()
    }

    fn lookup_active_seq(&self, allow_remap: bool) -> OwnedLookup {
        let domain = if self.pending_op.is_some() {
            MapDomain::OperatorPending
        } else if self.mode.is_visual() {
            MapDomain::Visual
        } else {
            MapDomain::Normal
        };
        if allow_remap {
            self.tables.domain(domain).lookup(&self.seq_buf).cloned_rhs()
        } else {
            default_tables().domain(domain).lookup(&self.seq_buf).cloned_rhs()
        }
    }

    fn resolve_char_wait(&mut self, wait: CharWait, tok: KeyToken, actions: &mut Vec<ModalAction>) -> bool {
        let KeyCode::Char(ch) = tok.key else {
            self.reset_pending();
            return true;
        };
        if tok.mods.has_command_mod() {
            self.reset_pending();
            return true;
        }
        match wait {
            CharWait::Register => {
                self.register = Some(ch);
            }
            CharWait::FindChar { forward, till } => {
                self.last_find_char = Some((ch, forward, till));
                let motion = Motion::FindChar { ch, forward, till };
                self.emit_motion(motion, actions);
            }
        }
        true
    }

    fn try_accumulate_digit(&mut self, tok: KeyToken) -> bool {
        if !tok.mods.is_none() {
            return false;
        }
        let KeyCode::Char(ch) = tok.key else {
            return false;
        };
        let Some(digit) = ch.to_digit(10) else {
            return false;
        };
        let d = digit as usize;
        let slot = if self.pending_op.is_some() { &mut self.count2 } else { &mut self.count1 };
        match *slot {
            // Bare `0` with no count in progress is `Motion::LineStart`, not a count!
            None if d == 0 => false,
            None => {
                *slot = Some(d);
                true
            }
            Some(prev) => {
                *slot = Some(prev.saturating_mul(10).saturating_add(d).min(MAX_COUNT));
                true
            }
        }
    }

    fn effective_count(&self) -> usize {
        let c1 = self.count1.unwrap_or(1);
        let c2 = self.count2.unwrap_or(1);
        c1.saturating_mul(c2).clamp(1, MAX_COUNT)
    }

    fn has_explicit_count(&self) -> bool {
        self.count1.is_some() || self.count2.is_some()
    }

    fn execute_rhs(&mut self, rhs: Rhs, depth: usize, now: f64, actions: &mut Vec<ModalAction>) {
        match rhs {
            Rhs::Nop => self.reset_pending(),
            Rhs::CharMotion { forward, till } => {
                self.char_wait = Some(CharWait::FindChar { forward, till });
            }
            Rhs::Operator { op, double_tail } => {
                self.pending_op = Some(PendingOp { op, double_tail });
            }
            Rhs::Keys { keys, remap } => {
                for key in keys {
                    let _ = self.step_token(key, remap, depth + 1, now, actions);
                }
            }
            Rhs::Action(action) => self.dispatch_action(action, actions),
        }
    }

    fn dispatch_action(&mut self, action: ModalAction, actions: &mut Vec<ModalAction>) {
        let count = self.effective_count();
        let explicit_count = self.has_explicit_count();
        let register = self.register;
        match action {
            ModalAction::Motion { motion, .. } => {
                let resolved = self.resolve_repeat_find(motion);
                let Some(motion) = resolved else {
                    self.reset_pending();
                    return;
                };
                self.emit_motion_with(motion, count, explicit_count, actions);
            }
            ModalAction::SelectTextObject { obj, inner, .. } => {
                if let Some(pending) = self.pending_op.take() {
                    let op = pending.op;
                    self.reset_pending();
                    actions.push(ModalAction::Operator { op, target: Target::TextObject { obj, inner, count }, register });
                } else {
                    self.reset_pending();
                    actions.push(ModalAction::SelectTextObject { obj, inner, count });
                }
            }
            ModalAction::EnterMode { mode, append } => {
                let target_mode = if self.mode == mode && mode.is_visual() {
                    // Pressing `v` again in Visual Char mode exits back to Normal, just like Vim!
                    Mode::Normal
                } else {
                    mode
                };
                self.mode = target_mode;
                self.reset_pending();
                actions.push(ModalAction::EnterMode { mode: target_mode, append });
            }
            ModalAction::ReselectVisual => {
                self.reset_pending();
                actions.push(ModalAction::ReselectVisual);
            }
            ModalAction::Operator { op, target, .. } => {
                self.reset_pending();
                if self.mode.is_visual() && matches!(target, Target::Selection) {
                    self.mode = Mode::Normal;
                    if op == Operator::Change {
                        self.mode = Mode::Insert;
                    }
                }
                actions.push(ModalAction::Operator { op, target: scale_target(target, count), register });
            }
            ModalAction::ReloadConfig => {
                let _ = self.reload_rc();
                actions.push(ModalAction::ReloadConfig);
            }
            other => {
                self.reset_pending();
                actions.push(scale_action(other, count, register));
            }
        }
    }

    fn resolve_repeat_find(&self, motion: Motion) -> Option<Motion> {
        let Motion::RepeatFindChar { reverse } = motion else {
            return Some(motion);
        };
        let (ch, forward, till) = self.last_find_char?;
        Some(Motion::FindChar { ch, forward: if reverse { !forward } else { forward }, till })
    }

    fn emit_motion(&mut self, motion: Motion, actions: &mut Vec<ModalAction>) {
        let count = self.effective_count();
        let explicit_count = self.has_explicit_count();
        self.emit_motion_with(motion, count, explicit_count, actions);
    }

    fn emit_motion_with(&mut self, motion: Motion, count: usize, explicit_count: bool, actions: &mut Vec<ModalAction>) {
        let register = self.register;
        if let Some(pending) = self.pending_op.take() {
            let op = pending.op;
            self.reset_pending();
            if op == Operator::Change {
                self.mode = Mode::Insert;
            }
            actions.push(ModalAction::Operator { op, target: Target::Motion { motion, count, explicit_count }, register });
        } else {
            self.reset_pending();
            actions.push(ModalAction::Motion { motion, count, explicit_count });
        }
    }
}

#[derive(Clone, Debug)]
enum OwnedLookup {
    None,
    Prefix,
    Exact(Rhs),
    ExactAndPrefix(Rhs),
}

impl TrieLookup<'_> {
    fn cloned_rhs(self) -> OwnedLookup {
        match self {
            TrieLookup::None => OwnedLookup::None,
            TrieLookup::Prefix => OwnedLookup::Prefix,
            TrieLookup::Exact(rhs) => OwnedLookup::Exact(rhs.clone()),
            TrieLookup::ExactAndPrefix(rhs) => OwnedLookup::ExactAndPrefix(rhs.clone()),
        }
    }
}

fn scale_target(target: Target, factor: usize) -> Target {
    match target {
        Target::Selection => Target::Selection,
        Target::Linewise { count } => Target::Linewise { count: count.saturating_mul(factor).min(MAX_COUNT) },
        Target::Motion { motion, count, explicit_count } => {
            Target::Motion { motion, count: count.saturating_mul(factor).min(MAX_COUNT), explicit_count }
        }
        Target::TextObject { obj, inner, count } => Target::TextObject { obj, inner, count: count.saturating_mul(factor).min(MAX_COUNT) },
    }
}

fn scale_action(action: ModalAction, count: usize, register: Option<char>) -> ModalAction {
    match action {
        ModalAction::Scroll { unit, dir, count: base } => ModalAction::Scroll { unit, dir, count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::Paste { before, count: base, register: r } => {
            ModalAction::Paste { before, count: base.saturating_mul(count).min(MAX_COUNT), register: register.or(r) }
        }
        ModalAction::Undo { count: base } => ModalAction::Undo { count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::Redo { count: base } => ModalAction::Redo { count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::SearchStep { forward, count: base } => ModalAction::SearchStep { forward, count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::Zoom { op, count: base } => ModalAction::Zoom { op, count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::JumpHistory { forward, count: base } => ModalAction::JumpHistory { forward, count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::StepTab { forward, count: base } => ModalAction::StepTab { forward, count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::StepPage { forward, count: base } => ModalAction::StepPage { forward, count: base.saturating_mul(count).min(MAX_COUNT) },
        ModalAction::AppCommand { cmd, count: base } => ModalAction::AppCommand { cmd, count: base.saturating_mul(count).min(MAX_COUNT) },
        other => other,
    }
}

fn op_symbol(op: Operator) -> &'static str {
    match op {
        Operator::Yank => "y",
        Operator::Delete => "d",
        Operator::Change => "c",
        Operator::Highlight => "gh",
        Operator::Underline => "gu",
        Operator::StrikeOut => "gs",
        Operator::Squiggly => "gq",
        Operator::Note => "gn",
    }
}
