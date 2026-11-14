//! `egui` frame event interceptor and unobtrusive modal status badge renderer.

use std::time::Duration;

use ::egui::{Align2, Color32, Context, CornerRadius, Event, FontId, Id, Key, Modifiers, Order, Stroke, pos2, vec2};

use crate::action::ModalAction;
use crate::action::{Mode, VisualKind};
use crate::parser::{FeedOutcome, ModalEngine, ModalStatus};
use crate::token::{KeyCode, KeyToken, Mods, NamedKey};

/// Intercept this frame's keyboard events in `ctx` through `engine`, removing consumed `Event::Key`
/// and paired `Event::Text` events so downstream application handlers only see unhandled keys.
///
/// Set `bypass = true` when a modal dialog, command palette, menu bar, or Chrome text field (like
/// the Find query box or Go-to-Page box) owns keyboard input.
pub fn filter_frame(ctx: &Context, engine: &mut ModalEngine, bypass: bool) -> Vec<ModalAction> {
    if !engine.options.enabled || bypass {
        return Vec::new();
    }
    let is_mac = matches!(ctx.os(), ::egui::os::OperatingSystem::Mac | ::egui::os::OperatingSystem::IOS);
    let now = ctx.input(|i| i.time);
    let (mut actions, replay_insert) = engine.flush_timeout(now);

    ctx.input_mut(|input| {
        if !replay_insert.is_empty() {
            let mut replayed = Vec::new();
            for tok in replay_insert {
                replayed.extend(token_to_egui_events(tok, is_mac));
            }
            replayed.append(&mut input.events);
            input.events = replayed;
        }

        let old_events = std::mem::take(&mut input.events);
        let mut kept = Vec::with_capacity(old_events.len());
        let mut idx = 0;
        while idx < old_events.len() {
            let ev = &old_events[idx];
            // Pair `Event::Key { pressed: true }` with an immediately following `Event::Text` when
            // no command modifier is held, so layout-shifted symbols (`$`, `/`, `?`, `:`, `{`)
            // produce their exact character token and are stripped together when consumed.
            if let Event::Key { key, pressed: true, modifiers, .. } = ev {
                let paired_text = old_events.get(idx + 1).and_then(|next| match next {
                    Event::Text(s) if !modifiers.ctrl && !modifiers.mac_cmd && !modifiers.command && single_printable_char(s).is_some() => {
                        single_printable_char(s)
                    }
                    _ => None,
                });
                if let Some(tok) = egui_key_to_token(*key, *modifiers, paired_text, is_mac) {
                    match engine.feed_key(tok, now) {
                        FeedOutcome::Consumed(emitted) => {
                            actions.extend(emitted);
                            idx += if paired_text.is_some() { 2 } else { 1 };
                            continue;
                        }
                        FeedOutcome::PassThrough => {
                            kept.push(old_events[idx].clone());
                            if paired_text.is_some() {
                                kept.push(old_events[idx + 1].clone());
                                idx += 2;
                            } else {
                                idx += 1;
                            }
                            continue;
                        }
                    }
                }
            }
            // Also catch standalone `Event::Text` (e.g. dead-key / IME / synthetic `ui.text`) when
            // in Normal or Visual mode so raw text never leaks through unconsumed.
            if engine.mode() != Mode::Insert
                && let Event::Text(s) = ev
            {
                let mut all_consumed = false;
                for ch in s.chars().filter(|c| !c.is_control()) {
                    if let FeedOutcome::Consumed(emitted) = engine.feed_key(KeyToken::plain_char(ch), now) {
                        actions.extend(emitted);
                        all_consumed = true;
                    }
                }
                if all_consumed {
                    idx += 1;
                    continue;
                }
            }
            kept.push(old_events[idx].clone());
            idx += 1;
        }
        input.events = kept;
    });

    if let Some(deadline) = engine.pending_deadline() {
        let remain_secs = (deadline - now).max(0.005);
        ctx.request_repaint_after(Duration::from_secs_f64(remain_secs));
    }

    actions
}

/// Paint a compact, theme-neutral status badge in the bottom-right corner of the viewport showing
/// the current mode (`NORMAL`, `VISUAL`, `INSERT`) and any pending operator/count/chord (`2g`, `gh`).
pub fn paint_mode_badge(ctx: &Context, status: &ModalStatus) {
    if !status.enabled || !status.show_badge {
        return;
    }
    let (bg, fg) = badge_palette(status.mode);
    let label =
        if status.pending_keys.is_empty() { status.mode.label().to_string() } else { format!("{}  {}", status.mode.label(), status.pending_keys) };
    let screen = ctx.content_rect();
    if !screen.is_positive() {
        return;
    }
    let pos = pos2((screen.right() - 16.0).max(8.0), (screen.bottom() - 12.0).max(8.0));
    ::egui::Area::new(Id::unique("craft_modal_status_badge"))
        .order(Order::Foreground)
        .interactable(false)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            let font = FontId::monospace(11.5);
            let galley = ui.fonts_mut(|f| f.layout_no_wrap(label, font, fg));
            let pad = vec2(8.0, 3.5);
            let size = galley.size() + pad * 2.0;
            let (rect, _) = ui.allocate_exact_size(size, ::egui::Sense::hover());
            ui.painter().rect(
                rect,
                CornerRadius::same(5),
                bg,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 36)),
                ::egui::StrokeKind::Inside,
            );
            ui.painter().galley(rect.min + pad, galley, fg);
        });
}

fn badge_palette(mode: Mode) -> (Color32, Color32) {
    match mode {
        Mode::Normal => (Color32::from_rgba_unmultiplied(34, 43, 58, 225), Color32::from_rgb(226, 232, 240)),
        Mode::Insert => (Color32::from_rgba_unmultiplied(22, 101, 52, 230), Color32::from_rgb(240, 253, 244)),
        Mode::Visual(VisualKind::Char) => (Color32::from_rgba_unmultiplied(154, 52, 18, 230), Color32::from_rgb(255, 247, 237)),
        Mode::Visual(VisualKind::Line) => (Color32::from_rgba_unmultiplied(146, 64, 14, 230), Color32::from_rgb(254, 252, 232)),
        Mode::Visual(VisualKind::Block) => (Color32::from_rgba_unmultiplied(107, 33, 168, 230), Color32::from_rgb(250, 245, 255)),
    }
}

fn single_printable_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || ch.is_control() {
        return None;
    }
    Some(ch)
}

fn egui_mods_to_modal(m: Modifiers, is_mac: bool) -> Mods {
    if is_mac {
        Mods { ctrl: m.ctrl, alt: m.alt, cmd: m.mac_cmd || m.command, shift: m.shift }
    } else {
        // On Linux/Windows, Ctrl+Alt is often AltGr on international keyboards; if paired_text
        // resolved a printable char without command intent, caller handles it, else preserve mods.
        Mods { ctrl: m.ctrl || m.command, alt: m.alt, cmd: false, shift: m.shift }
    }
}

fn egui_key_to_token(key: Key, modifiers: Modifiers, paired_char: Option<char>, is_mac: bool) -> Option<KeyToken> {
    let mods = egui_mods_to_modal(modifiers, is_mac);
    // AltGr (Ctrl+Alt on non-Mac) producing a printable symbol is a plain character press.
    if let Some(ch) = paired_char
        && !mods.cmd
        && (!mods.ctrl || (!is_mac && mods.alt))
    {
        return Some(KeyToken::plain_char(ch).normalized());
    }
    if let Some(named) = egui_named_key(key) {
        return Some(KeyToken { key: KeyCode::Named(named), mods }.normalized());
    }
    let ch = egui_printable_key(key, mods.shift && !mods.has_command_mod())?;
    Some(KeyToken { key: KeyCode::Char(ch), mods }.normalized())
}

fn egui_named_key(key: Key) -> Option<NamedKey> {
    match key {
        Key::Escape => Some(NamedKey::Escape),
        Key::Enter => Some(NamedKey::Enter),
        Key::Tab => Some(NamedKey::Tab),
        Key::Backspace => Some(NamedKey::Backspace),
        Key::Delete => Some(NamedKey::Delete),
        Key::Space => Some(NamedKey::Space),
        Key::ArrowUp => Some(NamedKey::Up),
        Key::ArrowDown => Some(NamedKey::Down),
        Key::ArrowLeft => Some(NamedKey::Left),
        Key::ArrowRight => Some(NamedKey::Right),
        Key::Home => Some(NamedKey::Home),
        Key::End => Some(NamedKey::End),
        Key::PageUp => Some(NamedKey::PageUp),
        Key::PageDown => Some(NamedKey::PageDown),
        Key::F1 => Some(NamedKey::F(1)),
        Key::F2 => Some(NamedKey::F(2)),
        Key::F3 => Some(NamedKey::F(3)),
        Key::F4 => Some(NamedKey::F(4)),
        Key::F5 => Some(NamedKey::F(5)),
        Key::F6 => Some(NamedKey::F(6)),
        Key::F7 => Some(NamedKey::F(7)),
        Key::F8 => Some(NamedKey::F(8)),
        Key::F9 => Some(NamedKey::F(9)),
        Key::F10 => Some(NamedKey::F(10)),
        Key::F11 => Some(NamedKey::F(11)),
        Key::F12 => Some(NamedKey::F(12)),
        _ => None,
    }
}

fn egui_printable_key(key: Key, shifted: bool) -> Option<char> {
    let base = match key {
        Key::A => 'a',
        Key::B => 'b',
        Key::C => 'c',
        Key::D => 'd',
        Key::E => 'e',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::I => 'i',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::M => 'm',
        Key::N => 'n',
        Key::O => 'o',
        Key::P => 'p',
        Key::Q => 'q',
        Key::R => 'r',
        Key::S => 's',
        Key::T => 't',
        Key::U => 'u',
        Key::V => 'v',
        Key::W => 'w',
        Key::X => 'x',
        Key::Y => 'y',
        Key::Z => 'z',
        Key::Num0 => return Some(if shifted { ')' } else { '0' }),
        Key::Num1 => return Some(if shifted { '!' } else { '1' }),
        Key::Num2 => return Some(if shifted { '@' } else { '2' }),
        Key::Num3 => return Some(if shifted { '#' } else { '3' }),
        Key::Num4 => return Some(if shifted { '$' } else { '4' }),
        Key::Num5 => return Some(if shifted { '%' } else { '5' }),
        Key::Num6 => return Some(if shifted { '^' } else { '6' }),
        Key::Num7 => return Some(if shifted { '&' } else { '7' }),
        Key::Num8 => return Some(if shifted { '*' } else { '8' }),
        Key::Num9 => return Some(if shifted { '(' } else { '9' }),
        Key::Minus => return Some(if shifted { '_' } else { '-' }),
        Key::Equals => return Some(if shifted { '+' } else { '=' }),
        Key::Plus => return Some('+'),
        Key::OpenBracket => return Some(if shifted { '{' } else { '[' }),
        Key::CloseBracket => return Some(if shifted { '}' } else { ']' }),
        Key::Backslash => return Some(if shifted { '|' } else { '\\' }),
        Key::Semicolon => return Some(if shifted { ':' } else { ';' }),
        Key::Colon => return Some(':'),
        Key::Quote => return Some(if shifted { '"' } else { '\'' }),
        Key::Backtick => return Some(if shifted { '~' } else { '`' }),
        Key::Comma => return Some(if shifted { '<' } else { ',' }),
        Key::Period => return Some(if shifted { '>' } else { '.' }),
        Key::Slash => return Some(if shifted { '?' } else { '/' }),
        Key::Questionmark => return Some('?'),
        Key::Exclamationmark => return Some('!'),
        Key::Pipe => return Some('|'),
        Key::OpenCurlyBracket => return Some('{'),
        Key::CloseCurlyBracket => return Some('}'),
        _ => return None,
    };
    Some(if shifted { base.to_ascii_uppercase() } else { base })
}

fn token_to_egui_events(tok: KeyToken, _is_mac: bool) -> Vec<Event> {
    match tok.key {
        KeyCode::Char(ch) if tok.mods.is_none() => vec![Event::Text(ch.to_string())],
        KeyCode::Named(NamedKey::Space) if tok.mods.is_none() => {
            vec![
                Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
                Event::Text(" ".into()),
            ]
        }
        _ => Vec::new(),
    }
}
