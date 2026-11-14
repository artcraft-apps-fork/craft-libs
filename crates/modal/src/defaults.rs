//! Default Vim / Evil-style Normal, Visual, Operator-Pending, and Insert bindings.

use crate::action::{AlignPos, ModalAction, Mode, Motion, Operator, ScrollDir, ScrollUnit, Target, TextObj, VisualKind, ZoomOp};
use crate::token::{KeyToken, NamedKey, parse_sequence};
use crate::trie::{KeyTables, KeyTrie, Rhs};

fn bind(trie: &mut KeyTrie, lhs: &str, rhs: Rhs) {
    if let Ok(seq) = parse_sequence(lhs, &[KeyToken::plain_char('\\')]) {
        trie.insert(&seq, rhs);
    }
}

fn bind_motion(tables: &mut KeyTables, lhs: &str, motion: Motion) {
    let rhs = Rhs::Action(ModalAction::Motion { motion, count: 1, explicit_count: false });
    bind(&mut tables.normal, lhs, rhs.clone());
    bind(&mut tables.visual, lhs, rhs.clone());
    bind(&mut tables.op_pending, lhs, rhs);
}

fn bind_text_obj(tables: &mut KeyTables, lhs: &str, obj: TextObj, inner: bool) {
    let rhs = Rhs::Action(ModalAction::SelectTextObject { obj, inner, count: 1 });
    bind(&mut tables.visual, lhs, rhs.clone());
    bind(&mut tables.op_pending, lhs, rhs);
}

fn bind_operator(tables: &mut KeyTables, lhs: &str, double_tail: char, op: Operator) {
    bind(&mut tables.normal, lhs, Rhs::Operator { op, double_tail: KeyToken::plain_char(double_tail) });
    bind(&mut tables.visual, lhs, Rhs::Action(ModalAction::Operator { op, target: Target::Selection, register: None }));
}

/// Construct a fresh [`KeyTables`] populated with standard Vim/Evil bindings.
pub fn default_tables() -> KeyTables {
    let mut t = KeyTables::default();
    install_motions(&mut t);
    install_text_objects(&mut t);
    install_operators(&mut t);
    install_scrolling_and_view(&mut t);
    install_mode_and_edit(&mut t);
    t
}

fn install_motions(t: &mut KeyTables) {
    bind_motion(t, "h", Motion::Left);
    bind_motion(t, "<Left>", Motion::Left);
    bind_motion(t, "l", Motion::Right);
    bind_motion(t, "<Right>", Motion::Right);
    bind_motion(t, "j", Motion::Down);
    bind_motion(t, "<Down>", Motion::Down);
    bind_motion(t, "k", Motion::Up);
    bind_motion(t, "<Up>", Motion::Up);

    bind_motion(t, "w", Motion::WordForward);
    bind_motion(t, "W", Motion::BigWordForward);
    bind_motion(t, "b", Motion::WordBack);
    bind_motion(t, "B", Motion::BigWordBack);
    bind_motion(t, "e", Motion::WordEnd);
    bind_motion(t, "E", Motion::BigWordEnd);

    bind_motion(t, "0", Motion::LineStart);
    bind_motion(t, "<Home>", Motion::LineStart);
    bind_motion(t, "^", Motion::FirstNonBlank);
    bind_motion(t, "_", Motion::FirstNonBlank);
    bind_motion(t, "$", Motion::LineEnd);
    bind_motion(t, "<End>", Motion::LineEnd);

    bind_motion(t, "{", Motion::ParagraphPrev);
    bind_motion(t, "}", Motion::ParagraphNext);
    bind_motion(t, "gg", Motion::DocStart);
    bind_motion(t, "G", Motion::DocEnd);
    bind_motion(t, "H", Motion::ScreenTop);
    bind_motion(t, "M", Motion::ScreenMiddle);
    bind_motion(t, "L", Motion::ScreenBottom);

    bind_motion(t, ";", Motion::RepeatFindChar { reverse: false });
    bind_motion(t, ",", Motion::RepeatFindChar { reverse: true });

    for (lhs, forward, till) in [("f", true, false), ("F", false, false), ("t", true, true), ("T", false, true)] {
        let rhs = Rhs::CharMotion { forward, till };
        bind(&mut t.normal, lhs, rhs.clone());
        bind(&mut t.visual, lhs, rhs.clone());
        bind(&mut t.op_pending, lhs, rhs);
    }
}

fn install_text_objects(t: &mut KeyTables) {
    bind_text_obj(t, "iw", TextObj::Word, true);
    bind_text_obj(t, "aw", TextObj::Word, false);
    bind_text_obj(t, "iW", TextObj::BigWord, true);
    bind_text_obj(t, "aW", TextObj::BigWord, false);
    bind_text_obj(t, "il", TextObj::Line, true);
    bind_text_obj(t, "al", TextObj::Line, false);
    bind_text_obj(t, "ip", TextObj::Paragraph, true);
    bind_text_obj(t, "ap", TextObj::Paragraph, false);
}

fn install_operators(t: &mut KeyTables) {
    bind_operator(t, "y", 'y', Operator::Yank);
    bind_operator(t, "d", 'd', Operator::Delete);
    bind_operator(t, "c", 'c', Operator::Change);
    // Annotation / formatting operators shared across PDF, Word, and Grid:
    // `gh` highlight, `gu` underline, `gs` strikeout, `gq` squiggly, `gn` sticky/text note.
    bind_operator(t, "gh", 'h', Operator::Highlight);
    bind_operator(t, "gu", 'u', Operator::Underline);
    bind_operator(t, "gs", 's', Operator::StrikeOut);
    bind_operator(t, "gq", 'q', Operator::Squiggly);
    bind_operator(t, "gn", 'n', Operator::Note);

    // `Y` yanks whole line, `x` deletes character under cursor.
    bind(&mut t.normal, "Y", Rhs::Action(ModalAction::Operator { op: Operator::Yank, target: Target::Linewise { count: 1 }, register: None }));
    bind(
        &mut t.normal,
        "x",
        Rhs::Action(ModalAction::Operator {
            op: Operator::Delete,
            target: Target::Motion { motion: Motion::Right, count: 1, explicit_count: false },
            register: None,
        }),
    );
    bind(&mut t.visual, "x", Rhs::Action(ModalAction::Operator { op: Operator::Delete, target: Target::Selection, register: None }));
}

fn install_scrolling_and_view(t: &mut KeyTables) {
    for (lhs, unit, dir) in [
        ("<C-d>", ScrollUnit::HalfPage, ScrollDir::Down),
        ("<C-u>", ScrollUnit::HalfPage, ScrollDir::Up),
        ("<C-f>", ScrollUnit::FullPage, ScrollDir::Down),
        ("<PageDown>", ScrollUnit::FullPage, ScrollDir::Down),
        ("<C-b>", ScrollUnit::FullPage, ScrollDir::Up),
        ("<PageUp>", ScrollUnit::FullPage, ScrollDir::Up),
        ("<C-e>", ScrollUnit::Line, ScrollDir::Down),
        ("<C-y>", ScrollUnit::Line, ScrollDir::Up),
    ] {
        let rhs = Rhs::Action(ModalAction::Scroll { unit, dir, count: 1 });
        bind(&mut t.normal, lhs, rhs.clone());
        bind(&mut t.visual, lhs, rhs);
    }

    for (lhs, pos) in [("zz", AlignPos::Center), ("zt", AlignPos::Top), ("zb", AlignPos::Bottom)] {
        let rhs = Rhs::Action(ModalAction::AlignView(pos));
        bind(&mut t.normal, lhs, rhs.clone());
        bind(&mut t.visual, lhs, rhs);
    }

    for (lhs, op) in [
        ("zi", ZoomOp::In),
        ("+", ZoomOp::In),
        ("=", ZoomOp::In),
        ("zo", ZoomOp::Out),
        ("-", ZoomOp::Out),
        ("z0", ZoomOp::Reset),
        ("zw", ZoomOp::FitWidth),
        ("zp", ZoomOp::FitPage),
    ] {
        bind(&mut t.normal, lhs, Rhs::Action(ModalAction::Zoom { op, count: 1 }));
    }

    bind(&mut t.normal, "<C-o>", Rhs::Action(ModalAction::JumpHistory { forward: false, count: 1 }));
    bind(&mut t.normal, "<C-i>", Rhs::Action(ModalAction::JumpHistory { forward: true, count: 1 }));
    bind(&mut t.normal, "gt", Rhs::Action(ModalAction::StepTab { forward: true, count: 1 }));
    bind(&mut t.normal, "gT", Rhs::Action(ModalAction::StepTab { forward: false, count: 1 }));
    bind(&mut t.normal, "]p", Rhs::Action(ModalAction::StepPage { forward: true, count: 1 }));
    bind(&mut t.normal, "[p", Rhs::Action(ModalAction::StepPage { forward: false, count: 1 }));
    bind(&mut t.normal, "J", Rhs::Action(ModalAction::StepPage { forward: true, count: 1 }));
    bind(&mut t.normal, "K", Rhs::Action(ModalAction::StepPage { forward: false, count: 1 }));
}

fn install_mode_and_edit(t: &mut KeyTables) {
    bind(&mut t.normal, "i", Rhs::Action(ModalAction::EnterMode { mode: Mode::Insert, append: false }));
    bind(&mut t.normal, "a", Rhs::Action(ModalAction::EnterMode { mode: Mode::Insert, append: true }));
    bind(&mut t.normal, "v", Rhs::Action(ModalAction::EnterMode { mode: Mode::Visual(VisualKind::Char), append: false }));
    bind(&mut t.normal, "V", Rhs::Action(ModalAction::EnterMode { mode: Mode::Visual(VisualKind::Line), append: false }));
    bind(&mut t.normal, "<C-v>", Rhs::Action(ModalAction::EnterMode { mode: Mode::Visual(VisualKind::Block), append: false }));
    bind(&mut t.normal, "gv", Rhs::Action(ModalAction::ReselectVisual));
    bind(&mut t.normal, "<Esc>", Rhs::Action(ModalAction::DismissTransient));

    bind(&mut t.visual, "v", Rhs::Action(ModalAction::EnterMode { mode: Mode::Visual(VisualKind::Char), append: false }));
    bind(&mut t.visual, "V", Rhs::Action(ModalAction::EnterMode { mode: Mode::Visual(VisualKind::Line), append: false }));
    bind(&mut t.visual, "<C-v>", Rhs::Action(ModalAction::EnterMode { mode: Mode::Visual(VisualKind::Block), append: false }));
    bind(&mut t.visual, "o", Rhs::Action(ModalAction::SwapVisualAnchor));
    bind(&mut t.visual, "O", Rhs::Action(ModalAction::SwapVisualAnchor));
    bind(&mut t.visual, "<Esc>", Rhs::Action(ModalAction::EnterMode { mode: Mode::Normal, append: false }));

    bind(&mut t.insert, "<Esc>", Rhs::Action(ModalAction::EnterMode { mode: Mode::Normal, append: false }));
    let esc_tok = KeyToken::named(NamedKey::Escape);
    t.insert.insert(&[esc_tok], Rhs::Action(ModalAction::EnterMode { mode: Mode::Normal, append: false }));

    bind(&mut t.normal, "p", Rhs::Action(ModalAction::Paste { before: false, count: 1, register: None }));
    bind(&mut t.normal, "P", Rhs::Action(ModalAction::Paste { before: true, count: 1, register: None }));
    bind(&mut t.visual, "p", Rhs::Action(ModalAction::Paste { before: false, count: 1, register: None }));

    bind(&mut t.normal, "u", Rhs::Action(ModalAction::Undo { count: 1 }));
    bind(&mut t.normal, "<C-r>", Rhs::Action(ModalAction::Redo { count: 1 }));

    for (lhs, action) in [
        ("/", ModalAction::SearchStart { forward: true }),
        ("?", ModalAction::SearchStart { forward: false }),
        ("*", ModalAction::SearchWord { forward: true }),
        ("#", ModalAction::SearchWord { forward: false }),
        ("n", ModalAction::SearchStep { forward: true, count: 1 }),
        ("N", ModalAction::SearchStep { forward: false, count: 1 }),
    ] {
        bind(&mut t.normal, lhs, Rhs::Action(action.clone()));
        bind(&mut t.visual, lhs, Rhs::Action(action));
    }
}
