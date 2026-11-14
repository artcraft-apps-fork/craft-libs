#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Reusable Vim / Evil-style modal keybinding framework (`Normal`, `Insert`, `Visual`) and
//! `.ideavimrc` / `.vimrc` configuration reader for Craft applications (`pdfcraft`, `wordcraft`,
//! `gridcraft`).

pub mod action;
pub mod defaults;
#[cfg(feature = "egui")]
pub mod egui;
pub mod parser;
pub mod rc;
pub mod token;
pub mod trie;

pub use action::{AlignPos, CommandCall, ModalAction, Mode, Motion, Operator, ScrollDir, ScrollUnit, Target, TextObj, VisualKind, ZoomOp};
pub use parser::{FeedOutcome, ModalEngine, ModalStatus};
pub use rc::{ModalOptions, RcEval, RcWarning};
pub use token::{KeyCode, KeyToken, Mods, NamedKey, format_tokens, parse_sequence};
pub use trie::{KeyTables, KeyTrie, MapDomain, Rhs, TrieLookup};
