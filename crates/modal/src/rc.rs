//! `.vimrc` / `.ideavimrc` subset parser and configuration file loader.
//!
//! Supports `let mapleader`, `set` options, `*map` / `*noremap` / `*unmap` / `*mapclear`,
//! `<Action>(command.id)` and `:action command.id<CR>`, `if has('app') ... elseif ... else ... endif`,
//! and `source <file>`.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::action::{CommandCall, ModalAction, Mode};
use crate::token::{KeyToken, parse_sequence};
use crate::trie::{KeyTables, MapDomain, Rhs};

/// Runtime options configurable via `set ...` in `.vimrc` / `.ideavimrc`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalOptions {
    pub enabled: bool,
    pub start_mode: Mode,
    pub timeout_ms: u64,
    pub scrolloff: usize,
    pub ignore_case: bool,
    pub smart_case: bool,
    pub hl_search: bool,
    pub inc_search: bool,
    pub unnamed_clipboard: bool,
    pub show_mode_badge: bool,
}

impl Default for ModalOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            start_mode: Mode::Normal,
            timeout_ms: 350,
            scrolloff: 3,
            ignore_case: true,
            smart_case: true,
            hl_search: true,
            inc_search: true,
            unnamed_clipboard: true,
            show_mode_badge: true,
        }
    }
}

/// Non-fatal diagnostic produced while reading an RC file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RcWarning {
    pub source: String,
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Copy, Debug)]
struct CondFrame {
    parent_active: bool,
    branch_taken: bool,
    this_active: bool,
}

/// Mutable state passed through RC evaluation (including nested `source` files).
pub struct RcEval<'a> {
    pub app_id: &'a str,
    pub leader: Vec<KeyToken>,
    pub options: &'a mut ModalOptions,
    pub tables: &'a mut KeyTables,
    pub warnings: Vec<RcWarning>,
    #[cfg(not(target_arch = "wasm32"))]
    visited: Vec<PathBuf>,
}

impl<'a> RcEval<'a> {
    pub fn new(app_id: &'a str, options: &'a mut ModalOptions, tables: &'a mut KeyTables) -> Self {
        Self {
            app_id,
            leader: vec![KeyToken::plain_char('\\')],
            options,
            tables,
            warnings: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            visited: Vec::new(),
        }
    }

    /// Evaluate RC script text in memory.
    pub fn eval_str(&mut self, text: &str, source_name: &str, base_dir: Option<&str>) {
        let mut conds: Vec<CondFrame> = Vec::new();
        for (idx, raw_line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = strip_vim_comment(raw_line).trim();
            if trimmed.is_empty() {
                continue;
            }
            if handle_conditional(trimmed, self.app_id, &mut conds) {
                continue;
            }
            let active = conds.last().is_none_or(|c| c.parent_active && c.this_active);
            if !active {
                continue;
            }
            if let Err(msg) = self.eval_statement(trimmed, base_dir) {
                self.warnings.push(RcWarning { source: source_name.to_string(), line: line_no, message: msg });
            }
        }
    }

    fn eval_statement(&mut self, line: &str, base_dir: Option<&str>) -> Result<(), String> {
        let line = line.strip_prefix(':').map(str::trim_start).unwrap_or(line);
        let (cmd, rest) = split_first_word(line);
        match cmd {
            "set" | "se" => apply_set_line(rest, self.options),
            "let" => apply_let_line(rest, &mut self.leader),
            "source" | "so" => self.eval_source_cmd(rest, base_dir),
            "mapclear" => {
                self.tables.normal.clear();
                self.tables.visual.clear();
                self.tables.op_pending.clear();
                Ok(())
            }
            "nmapclear" => {
                self.tables.normal.clear();
                Ok(())
            }
            "vmapclear" | "xmapclear" => {
                self.tables.visual.clear();
                Ok(())
            }
            "imapclear" => {
                self.tables.insert.clear();
                Ok(())
            }
            _ => {
                if let Some((domains, remap)) = map_command_spec(cmd) {
                    return apply_map_cmd(rest, &domains, remap, &self.leader, self.tables);
                }
                if let Some(domains) = unmap_command_spec(cmd) {
                    return apply_unmap_cmd(rest, &domains, &self.leader, self.tables);
                }
                Err(format!("unsupported rc command `{cmd}`"))
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn eval_source_cmd(&mut self, raw_path: &str, base_dir: Option<&str>) -> Result<(), String> {
        if self.visited.len() >= 8 {
            return Err("maximum `source` nesting depth (8) exceeded".into());
        }
        let unquoted = unquote(raw_path.trim());
        let expanded = expand_tilde(unquoted, base_dir);
        self.eval_file(&expanded)
    }

    #[cfg(target_arch = "wasm32")]
    fn eval_source_cmd(&mut self, _raw_path: &str, _base_dir: Option<&str>) -> Result<(), String> {
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn eval_file(&mut self, path: &Path) -> Result<(), String> {
        if !path.is_file() {
            return Ok(());
        }
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if self.visited.contains(&canonical) {
            return Ok(());
        }
        let content = std::fs::read_to_string(&canonical).map_err(|e| format!("could not read `{}`: {e}", canonical.display()))?;
        self.visited.push(canonical.clone());
        let parent = canonical.parent().map(|p| p.to_string_lossy().into_owned());
        let label = canonical.display().to_string();
        self.eval_str(&content, &label, parent.as_deref());
        self.visited.pop();
        Ok(())
    }
}

/// Discover and load layered RC files for `app_id` (e.g., `"pdfcraft"`).
///
/// Honors `<APP>_CONFIG_DIR=""` test isolation (reads nothing when isolated).
/// Otherwise loads in ascending precedence:
/// 1. `~/.craftvimrc`
/// 2. `$XDG_CONFIG_HOME/artcraft/vimrc` (or `~/.config/artcraft/vimrc`)
/// 3. `~/.<app_id>vimrc`
/// 4. `$XDG_CONFIG_HOME/<app_id>/vimrc` (or `~/.config/<app_id>/vimrc`)
#[cfg(not(target_arch = "wasm32"))]
pub fn load_user_rc(app_id: &str, options: &mut ModalOptions, tables: &mut KeyTables) -> Vec<RcWarning> {
    let env_key = format!("{}_CONFIG_DIR", app_id.to_ascii_uppercase().replace('-', "_"));
    if let Some(val) = std::env::var_os(&env_key) {
        if val.is_empty() {
            return Vec::new();
        }
        let dir = PathBuf::from(val);
        let mut eval = RcEval::new(app_id, options, tables);
        let _ = eval.eval_file(&dir.join("vimrc"));
        return eval.warnings;
    }
    let candidates = candidate_rc_paths(app_id);
    let mut eval = RcEval::new(app_id, options, tables);
    for path in candidates {
        let _ = eval.eval_file(&path);
    }
    eval.warnings
}

#[cfg(target_arch = "wasm32")]
pub fn load_user_rc(_app_id: &str, _options: &mut ModalOptions, _tables: &mut KeyTables) -> Vec<RcWarning> {
    Vec::new()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn candidate_rc_paths(app_id: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = std::env::home_dir();
    let xdg = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    let config_root = xdg.or_else(|| home.as_ref().map(|h| h.join(".config")));

    if let Some(h) = &home {
        out.push(h.join(".craftvimrc"));
    }
    if let Some(cfg) = &config_root {
        out.push(cfg.join("artcraft").join("vimrc"));
    }
    if let Some(h) = &home {
        out.push(h.join(format!(".{app_id}vimrc")));
    }
    if let Some(cfg) = &config_root {
        out.push(cfg.join(app_id).join("vimrc"));
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn expand_tilde(raw: &str, base_dir: Option<&str>) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = std::env::home_dir()
    {
        return home.join(rest);
    }
    let path = PathBuf::from(raw);
    if path.is_relative()
        && let Some(base) = base_dir
    {
        return Path::new(base).join(path);
    }
    path
}

fn strip_vim_comment(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('"') {
        return "";
    }
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && (in_single || in_double) {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single {
            // In Vimscript, `"` outside single quotes starts a comment unless part of `let x = "..."`
            // or `"a` register in RHS. Only treat `"` as a trailing comment if preceded by whitespace
            // and no closing `"` follows on `let` lines.
            let rest = &line[idx + 1..];
            if !line.trim_start().starts_with("let ") && idx > 0 && line[..idx].ends_with(char::is_whitespace) && !rest.contains('"') {
                return &line[..idx];
            }
            in_double = !in_double;
        }
    }
    line
}

fn handle_conditional(line: &str, app_id: &str, stack: &mut Vec<CondFrame>) -> bool {
    let (kw, rest) = split_first_word(line);
    match kw {
        "if" => {
            let parent_active = stack.last().is_none_or(|c| c.parent_active && c.this_active);
            let cond = parent_active && eval_condition(rest, app_id);
            stack.push(CondFrame { parent_active, branch_taken: cond, this_active: cond });
            true
        }
        "elseif" => {
            if let Some(top) = stack.last_mut() {
                if top.parent_active && !top.branch_taken {
                    let cond = eval_condition(rest, app_id);
                    top.this_active = cond;
                    top.branch_taken = cond;
                } else {
                    top.this_active = false;
                }
            }
            true
        }
        "else" => {
            if let Some(top) = stack.last_mut() {
                top.this_active = top.parent_active && !top.branch_taken;
                top.branch_taken = true;
            }
            true
        }
        "endif" | "fi" => {
            stack.pop();
            true
        }
        _ => false,
    }
}

fn eval_condition(expr: &str, app_id: &str) -> bool {
    let e = expr.trim();
    if let Some(neg) = e.strip_prefix('!') {
        return !eval_condition(neg, app_id);
    }
    if let Some(inner) = e.strip_prefix("has(").and_then(|s| s.strip_suffix(')')) {
        let feat = unquote(inner.trim());
        return feat.eq_ignore_ascii_case(app_id)
            || feat.eq_ignore_ascii_case("craft")
            || feat.eq_ignore_ascii_case("artcraft")
            || feat.eq_ignore_ascii_case("ideavim")
            || feat.eq_ignore_ascii_case("modal");
    }
    e == "1" || e.eq_ignore_ascii_case("true")
}

fn apply_let_line(rest: &str, leader: &mut Vec<KeyToken>) -> Result<(), String> {
    let Some((lhs, rhs)) = rest.split_once('=') else {
        return Err("expected `let <var> = <value>`".into());
    };
    let var = lhs.trim();
    let var = var.strip_prefix("g:").unwrap_or(var);
    if var == "mapleader" {
        let raw = unquote_preserving_space(rhs.trim());
        let parsed = parse_sequence(&raw, leader)?;
        if !parsed.is_empty() {
            *leader = parsed;
        }
    }
    Ok(())
}

fn apply_set_line(rest: &str, opts: &mut ModalOptions) -> Result<(), String> {
    for token in rest.split_whitespace() {
        apply_single_option(token, opts)?;
    }
    Ok(())
}

fn apply_single_option(token: &str, opts: &mut ModalOptions) -> Result<(), String> {
    if let Some((k, v)) = token.split_once('=') {
        return match k {
            "timeoutlen" | "tm" => {
                opts.timeout_ms = v.parse::<u64>().map_err(|_| format!("invalid timeoutlen `{v}`"))?.clamp(20, 5000);
                Ok(())
            }
            "scrolloff" | "so" => {
                opts.scrolloff = v.parse::<usize>().map_err(|_| format!("invalid scrolloff `{v}`"))?.min(50);
                Ok(())
            }
            "clipboard" | "cb" => {
                opts.unnamed_clipboard = v.contains("unnamed");
                Ok(())
            }
            _ => Ok(()),
        };
    }
    let (neg, name) = match token.strip_prefix("no") {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, token),
    };
    let on = !neg;
    match name {
        "vim" | "modal" | "ideavim" => opts.enabled = on,
        "startinsert" => opts.start_mode = if on { Mode::Insert } else { Mode::Normal },
        "ignorecase" | "ic" => opts.ignore_case = on,
        "smartcase" | "scs" => opts.smart_case = on,
        "hlsearch" | "hls" => opts.hl_search = on,
        "incsearch" | "is" => opts.inc_search = on,
        "showmode" | "smd" => opts.show_mode_badge = on,
        _ => {}
    }
    Ok(())
}

fn map_command_spec(cmd: &str) -> Option<(Vec<MapDomain>, bool)> {
    match cmd {
        "map" => Some((vec![MapDomain::Normal, MapDomain::Visual, MapDomain::OperatorPending], true)),
        "noremap" => Some((vec![MapDomain::Normal, MapDomain::Visual, MapDomain::OperatorPending], false)),
        "nmap" => Some((vec![MapDomain::Normal], true)),
        "nnoremap" => Some((vec![MapDomain::Normal], false)),
        "vmap" | "xmap" => Some((vec![MapDomain::Visual], true)),
        "vnoremap" | "xnoremap" => Some((vec![MapDomain::Visual], false)),
        "omap" => Some((vec![MapDomain::OperatorPending], true)),
        "onoremap" => Some((vec![MapDomain::OperatorPending], false)),
        "imap" => Some((vec![MapDomain::Insert], true)),
        "inoremap" => Some((vec![MapDomain::Insert], false)),
        _ => None,
    }
}

fn unmap_command_spec(cmd: &str) -> Option<Vec<MapDomain>> {
    match cmd {
        "unmap" => Some(vec![MapDomain::Normal, MapDomain::Visual, MapDomain::OperatorPending]),
        "nunmap" => Some(vec![MapDomain::Normal]),
        "vunmap" | "xunmap" => Some(vec![MapDomain::Visual]),
        "ounmap" => Some(vec![MapDomain::OperatorPending]),
        "iunmap" => Some(vec![MapDomain::Insert]),
        _ => None,
    }
}

fn strip_map_arguments(mut rest: &str) -> &str {
    loop {
        let trimmed = rest.trim_start();
        let mut matched = None;
        for flag in ["<silent>", "<nowait>", "<buffer>", "<unique>", "<special>"] {
            if trimmed.len() >= flag.len() && trimmed[..flag.len()].eq_ignore_ascii_case(flag) {
                matched = Some(&trimmed[flag.len()..]);
                break;
            }
        }
        match matched {
            Some(after) => rest = after,
            None => return trimmed,
        }
    }
}

fn apply_map_cmd(rest: &str, domains: &[MapDomain], remap: bool, leader: &[KeyToken], tables: &mut KeyTables) -> Result<(), String> {
    let cleaned = strip_map_arguments(rest);
    let (lhs_str, rhs_raw) = split_first_word(cleaned);
    let rhs_str = rhs_raw.trim();
    if lhs_str.is_empty() || rhs_str.is_empty() {
        return Err("expected `<lhs> <rhs>` in mapping".into());
    }
    let lhs = parse_sequence(lhs_str, leader)?;
    if lhs.is_empty() {
        return Err("mapping LHS produced no keys".into());
    }
    let rhs = parse_rhs(rhs_str, remap, leader)?;
    for &domain in domains {
        tables.domain_mut(domain).insert(&lhs, rhs.clone());
    }
    Ok(())
}

fn apply_unmap_cmd(rest: &str, domains: &[MapDomain], leader: &[KeyToken], tables: &mut KeyTables) -> Result<(), String> {
    let cleaned = strip_map_arguments(rest);
    let lhs = parse_sequence(cleaned.trim(), leader)?;
    for &domain in domains {
        tables.domain_mut(domain).remove(&lhs);
    }
    Ok(())
}

fn parse_rhs(rhs_str: &str, remap: bool, leader: &[KeyToken]) -> Result<Rhs, String> {
    if rhs_str.eq_ignore_ascii_case("<nop>") {
        return Ok(Rhs::Nop);
    }
    if rhs_str.eq_ignore_ascii_case(":source<cr>") || rhs_str.eq_ignore_ascii_case("<action>(modal.reload)") {
        return Ok(Rhs::Action(ModalAction::ReloadConfig));
    }
    // `<Action>(command.id)` or `<Action>(command.id {"k":"v"})`
    if rhs_str.len() > 9
        && rhs_str[..9].eq_ignore_ascii_case("<action>(")
        && let Some(spec) = rhs_str[9..].strip_suffix(')')
    {
        let cmd = CommandCall::parse_spec(spec)?;
        return Ok(Rhs::Action(ModalAction::AppCommand { cmd, count: 1 }));
    }
    // IdeaVim `:action command.id<CR>`
    if let Some(after_colon) = rhs_str.strip_prefix(':') {
        let (kw, rest) = split_first_word(after_colon);
        if kw.eq_ignore_ascii_case("action") {
            let body = strip_trailing_cr(rest.trim());
            let cmd = CommandCall::parse_spec(body)?;
            return Ok(Rhs::Action(ModalAction::AppCommand { cmd, count: 1 }));
        }
    }
    let keys = parse_sequence(rhs_str, leader)?;
    Ok(Rhs::Keys { keys, remap })
}

fn strip_trailing_cr(s: &str) -> &str {
    for suffix in ["<CR>", "<cr>", "<Cr>", "<Enter>", "<enter>", "<Return>"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return stripped.trim();
        }
    }
    s
}

fn split_first_word(s: &str) -> (&str, &str) {
    let trimmed = s.trim_start();
    match trimmed.find(char::is_whitespace) {
        Some(idx) => (&trimmed[..idx], &trimmed[idx..]),
        None => (trimmed, ""),
    }
}

fn unquote(s: &str) -> &str {
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) { &s[1..s.len() - 1] } else { s }
}

fn unquote_preserving_space(s: &str) -> String {
    let inner = unquote(s);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && let Some(next) = chars.next()
        {
            out.push(next);
            continue;
        }
        out.push(ch);
    }
    out
}
