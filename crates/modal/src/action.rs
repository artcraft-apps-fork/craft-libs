//! Editing modes and abstract modal actions shared across Craft applications.

use serde::{Deserialize, Serialize};

/// Sub-flavors of Visual selection mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VisualKind {
    /// Character/glyph-wise selection (`v`).
    Char,
    /// Line-wise selection (`V`).
    Line,
    /// Rectangular/columnar block selection (`<C-v>`).
    Block,
}

/// Active modal editing mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual(VisualKind),
}

impl Mode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual(VisualKind::Char) => "VISUAL",
            Self::Visual(VisualKind::Line) => "V-LINE",
            Self::Visual(VisualKind::Block) => "V-BLOCK",
        }
    }

    pub const fn is_visual(self) -> bool {
        matches!(self, Self::Visual(_))
    }
}

/// Cursor / selection movement primitives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Motion {
    Left,
    Right,
    Down,
    Up,
    WordForward,
    WordBack,
    WordEnd,
    BigWordForward,
    BigWordBack,
    BigWordEnd,
    LineStart,
    FirstNonBlank,
    LineEnd,
    ParagraphPrev,
    ParagraphNext,
    DocStart,
    DocEnd,
    ScreenTop,
    ScreenMiddle,
    ScreenBottom,
    FindChar { ch: char, forward: bool, till: bool },
    RepeatFindChar { reverse: bool },
}

/// Viewport scrolling step sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScrollUnit {
    Line,
    HalfPage,
    FullPage,
}

/// Viewport scrolling directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScrollDir {
    Down,
    Up,
    Left,
    Right,
}

/// Vertical alignment target for `zz`, `zt`, `zb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AlignPos {
    Center,
    Top,
    Bottom,
}

/// Text objects (`iw`, `aw`, `il`, `al`, `ip`, `ap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TextObj {
    Word,
    BigWord,
    Line,
    Paragraph,
}

/// Composable operators that act on Visual selections, motions, or text objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operator {
    Yank,
    Delete,
    Change,
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Note,
}

/// Range an [`Operator`] acts upon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Target {
    /// Current Visual selection.
    Selection,
    /// Whole line(s) (`yy`, `dd`, `cc`, `ghh`).
    Linewise { count: usize },
    /// From caret across `motion` repeated `count` times.
    Motion { motion: Motion, count: usize, explicit_count: bool },
    /// Around/inside a text object (`yiw`, `ghip`, `daw`).
    TextObject { obj: TextObj, inner: bool, count: usize },
}

/// Zoom commands (`zi`, `zo`, `z0`, `zw`, `zp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZoomOp {
    In,
    Out,
    Reset,
    FitWidth,
    FitPage,
}

/// A native application command invocation (`<Action>(command.id)` or parameterized JSON).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandCall {
    pub id: String,
    pub params: serde_json::Value,
}

impl CommandCall {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), params: serde_json::Value::Null }
    }

    /// Parse `"command.id"` or `"command.id {\"k\":\"v\"}"`.
    pub fn parse_spec(spec: &str) -> Result<Self, String> {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return Err("empty <Action> command id".into());
        }
        if let Some((id, json_part)) = trimmed.split_once(char::is_whitespace) {
            let params: serde_json::Value = serde_json::from_str(json_part.trim()).map_err(|e| format!("invalid JSON params for `{id}`: {e}"))?;
            Ok(Self { id: id.to_string(), params })
        } else {
            Ok(Self::new(trimmed))
        }
    }
}

/// Concrete action emitted by [`crate::ModalEngine`] for the host application to execute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModalAction {
    /// Enter `Normal`, `Insert`, or `Visual(kind)`. `append` distinguishes `a` from `i`.
    EnterMode { mode: Mode, append: bool },
    /// Re-select the last Visual range (`gv`).
    ReselectVisual,
    /// Swap Visual selection anchor and cursor (`o` in Visual mode).
    SwapVisualAnchor,
    /// Move cursor / selection head (or scroll/jump when no text caret is active).
    Motion { motion: Motion, count: usize, explicit_count: bool },
    /// Scroll the viewport (`<C-d>`, `<C-u>`, `<C-f>`, `<C-b>`, `<C-e>`, `<C-y>`).
    Scroll { unit: ScrollUnit, dir: ScrollDir, count: usize },
    /// Reposition viewport around cursor (`zz`, `zt`, `zb`).
    AlignView(AlignPos),
    /// Execute an operator on a motion, text object, line, or Visual selection.
    Operator { op: Operator, target: Target, register: Option<char> },
    /// Expand or set Visual selection to a text object (`iw`, `aw`, `il`, `ip`).
    SelectTextObject { obj: TextObj, inner: bool, count: usize },
    /// Paste clipboard / register (`p`, `P`).
    Paste { before: bool, count: usize, register: Option<char> },
    /// Undo `count` steps (`u`).
    Undo { count: usize },
    /// Redo `count` steps (`<C-r>`).
    Redo { count: usize },
    /// Open search input (`/`, `?`).
    SearchStart { forward: bool },
    /// Search word under cursor (`*`, `#`).
    SearchWord { forward: bool },
    /// Jump to next/previous search match (`n`, `N`).
    SearchStep { forward: bool, count: usize },
    /// Dismiss active search or selection (`<Esc>` in Normal mode).
    DismissTransient,
    /// Adjust document zoom (`zi`, `zo`, `z0`, `zw`, `zp`).
    Zoom { op: ZoomOp, count: usize },
    /// Navigate view history (`<C-o>`, `<C-i>`).
    JumpHistory { forward: bool, count: usize },
    /// Switch document tabs (`gt`, `gT`).
    StepTab { forward: bool, count: usize },
    /// Step pages explicitly (`]p`, `[p`, `J`, `K`).
    StepPage { forward: bool, count: usize },
    /// Reload `.vimrc` / `.ideavimrc` from disk.
    ReloadConfig,
    /// Run a native application command by ID (`<Action>(...)`).
    AppCommand { cmd: CommandCall, count: usize },
}
