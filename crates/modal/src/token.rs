//! Case-sensitive modal key tokens and Vim/IdeaVim key sequence parser (`<C-d>`, `<leader>w`, `gg`).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Modifier state attached to a [`KeyToken`].
///
/// For printable [`KeyCode::Char`] tokens without `ctrl`/`alt`/`cmd`, `shift` is always `false`
/// because character case (`'g'` vs `'G'`, `'/'` vs `'?'`) already encodes shift in a
/// layout-independent way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub cmd: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Self = Self { ctrl: false, alt: false, cmd: false, shift: false };

    pub const fn is_none(self) -> bool {
        !self.ctrl && !self.alt && !self.cmd && !self.shift
    }

    pub const fn has_command_mod(self) -> bool {
        self.ctrl || self.alt || self.cmd
    }
}

/// Non-character keys used in modal bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NamedKey {
    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

/// Key identity within a [`KeyToken`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyCode {
    Char(char),
    Named(NamedKey),
}

/// A single step in a modal key sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyToken {
    pub key: KeyCode,
    pub mods: Mods,
}

impl KeyToken {
    pub const fn plain_char(ch: char) -> Self {
        if ch == ' ' { Self { key: KeyCode::Named(NamedKey::Space), mods: Mods::NONE } } else { Self { key: KeyCode::Char(ch), mods: Mods::NONE } }
    }

    pub const fn named(named: NamedKey) -> Self {
        Self { key: KeyCode::Named(named), mods: Mods::NONE }
    }

    pub const fn ctrl(ch: char) -> Self {
        Self { key: KeyCode::Char(ch.to_ascii_lowercase()), mods: Mods { ctrl: true, alt: false, cmd: false, shift: false } }
    }

    pub fn normalized(mut self) -> Self {
        if let KeyCode::Char(' ') = self.key {
            self.key = KeyCode::Named(NamedKey::Space);
        }
        if let KeyCode::Char(ch) = self.key {
            if !self.mods.has_command_mod() {
                self.mods.shift = false;
            } else if ch.is_ascii_alphabetic() {
                if ch.is_ascii_uppercase() {
                    self.mods.shift = true;
                }
                self.key = KeyCode::Char(ch.to_ascii_lowercase());
            }
        }
        // `<C-[>` is Escape in Vim.
        if self.key == KeyCode::Char('[') && self.mods == (Mods { ctrl: true, alt: false, cmd: false, shift: false }) {
            return Self::named(NamedKey::Escape);
        }
        self
    }

    pub const fn is_function_key(self) -> bool {
        matches!(self.key, KeyCode::Named(NamedKey::F(_)))
    }
}

impl fmt::Display for KeyToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.is_none() {
            return match self.key {
                KeyCode::Char(ch) => write!(f, "{ch}"),
                KeyCode::Named(NamedKey::Space) => write!(f, "<Space>"),
                KeyCode::Named(NamedKey::Escape) => write!(f, "<Esc>"),
                KeyCode::Named(NamedKey::Enter) => write!(f, "<CR>"),
                KeyCode::Named(NamedKey::Tab) => write!(f, "<Tab>"),
                KeyCode::Named(NamedKey::Backspace) => write!(f, "<BS>"),
                KeyCode::Named(NamedKey::Delete) => write!(f, "<Del>"),
                KeyCode::Named(NamedKey::Up) => write!(f, "<Up>"),
                KeyCode::Named(NamedKey::Down) => write!(f, "<Down>"),
                KeyCode::Named(NamedKey::Left) => write!(f, "<Left>"),
                KeyCode::Named(NamedKey::Right) => write!(f, "<Right>"),
                KeyCode::Named(NamedKey::Home) => write!(f, "<Home>"),
                KeyCode::Named(NamedKey::End) => write!(f, "<End>"),
                KeyCode::Named(NamedKey::PageUp) => write!(f, "<PageUp>"),
                KeyCode::Named(NamedKey::PageDown) => write!(f, "<PageDown>"),
                KeyCode::Named(NamedKey::F(n)) => write!(f, "<F{n}>"),
            };
        }
        write!(f, "<")?;
        if self.mods.ctrl {
            write!(f, "C-")?;
        }
        if self.mods.alt {
            write!(f, "M-")?;
        }
        if self.mods.cmd {
            write!(f, "D-")?;
        }
        if self.mods.shift {
            write!(f, "S-")?;
        }
        match self.key {
            KeyCode::Char(ch) => write!(f, "{ch}>"),
            KeyCode::Named(named) => write!(f, "{}>", named_label(named)),
        }
    }
}

fn named_label(named: NamedKey) -> &'static str {
    match named {
        NamedKey::Escape => "Esc",
        NamedKey::Enter => "CR",
        NamedKey::Tab => "Tab",
        NamedKey::Backspace => "BS",
        NamedKey::Delete => "Del",
        NamedKey::Space => "Space",
        NamedKey::Up => "Up",
        NamedKey::Down => "Down",
        NamedKey::Left => "Left",
        NamedKey::Right => "Right",
        NamedKey::Home => "Home",
        NamedKey::End => "End",
        NamedKey::PageUp => "PageUp",
        NamedKey::PageDown => "PageDown",
        NamedKey::F(1) => "F1",
        NamedKey::F(2) => "F2",
        NamedKey::F(3) => "F3",
        NamedKey::F(4) => "F4",
        NamedKey::F(5) => "F5",
        NamedKey::F(6) => "F6",
        NamedKey::F(7) => "F7",
        NamedKey::F(8) => "F8",
        NamedKey::F(9) => "F9",
        NamedKey::F(10) => "F10",
        NamedKey::F(11) => "F11",
        NamedKey::F(12) => "F12",
        NamedKey::F(_) => "Fn",
    }
}

/// Format a slice of tokens for status display.
pub fn format_tokens(tokens: &[KeyToken]) -> String {
    let mut out = String::new();
    for tok in tokens {
        out.push_str(&tok.to_string());
    }
    out
}

/// Parse a Vim/IdeaVim key sequence (such as `<leader>w`, `<C-d>`, `gg`, `<Esc>`) into tokens.
pub fn parse_sequence(input: &str, leader: &[KeyToken]) -> Result<Vec<KeyToken>, String> {
    let mut out = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        if let Some(after_lt) = rest.strip_prefix('<')
            && let Some(end) = after_lt.find('>')
        {
            let inner = &after_lt[..end];
            if !inner.is_empty() && !inner.contains('<') {
                if inner.eq_ignore_ascii_case("leader") {
                    out.extend_from_slice(leader);
                } else if !inner.eq_ignore_ascii_case("nop") {
                    out.push(parse_angle_token(inner)?);
                }
                rest = &after_lt[end + 1..];
                continue;
            }
        }
        let Some(ch) = rest.chars().next() else { break };
        out.push(KeyToken::plain_char(ch).normalized());
        rest = &rest[ch.len_utf8()..];
    }
    Ok(out)
}

fn parse_angle_token(inner: &str) -> Result<KeyToken, String> {
    let mut mods = Mods::NONE;
    let mut tail = inner;
    while let Some((prefix, after)) = tail.split_once('-') {
        if after.is_empty() {
            break;
        }
        match prefix.to_ascii_uppercase().as_str() {
            "C" | "CTRL" => mods.ctrl = true,
            "S" | "SHIFT" => mods.shift = true,
            "M" | "A" | "ALT" | "META" | "OPT" => mods.alt = true,
            "D" | "CMD" | "SUPER" => mods.cmd = true,
            _ => return Err(format!("unknown modifier `{prefix}` in `<{inner}>`")),
        }
        tail = after;
    }
    let key = parse_key_name(tail, &mut mods)?;
    Ok(KeyToken { key, mods }.normalized())
}

fn parse_key_name(name: &str, mods: &mut Mods) -> Result<KeyCode, String> {
    if name.chars().count() == 1
        && let Some(ch) = name.chars().next()
    {
        if mods.has_command_mod() && ch.is_ascii_uppercase() {
            mods.shift = true;
            return Ok(KeyCode::Char(ch.to_ascii_lowercase()));
        }
        return Ok(KeyCode::Char(ch));
    }
    let upper = name.to_ascii_uppercase();
    match upper.as_str() {
        "ESC" | "ESCAPE" => Ok(KeyCode::Named(NamedKey::Escape)),
        "CR" | "RETURN" | "ENTER" => Ok(KeyCode::Named(NamedKey::Enter)),
        "TAB" => Ok(KeyCode::Named(NamedKey::Tab)),
        "BS" | "BACKSPACE" => Ok(KeyCode::Named(NamedKey::Backspace)),
        "DEL" | "DELETE" => Ok(KeyCode::Named(NamedKey::Delete)),
        "SPACE" => Ok(KeyCode::Named(NamedKey::Space)),
        "UP" => Ok(KeyCode::Named(NamedKey::Up)),
        "DOWN" => Ok(KeyCode::Named(NamedKey::Down)),
        "LEFT" => Ok(KeyCode::Named(NamedKey::Left)),
        "RIGHT" => Ok(KeyCode::Named(NamedKey::Right)),
        "HOME" => Ok(KeyCode::Named(NamedKey::Home)),
        "END" => Ok(KeyCode::Named(NamedKey::End)),
        "PAGEUP" | "PGUP" => Ok(KeyCode::Named(NamedKey::PageUp)),
        "PAGEDOWN" | "PGDN" => Ok(KeyCode::Named(NamedKey::PageDown)),
        "BAR" => Ok(KeyCode::Char('|')),
        "BSLASH" => Ok(KeyCode::Char('\\')),
        "LT" => Ok(KeyCode::Char('<')),
        "GT" => Ok(KeyCode::Char('>')),
        "MINUS" => Ok(KeyCode::Char('-')),
        _ => parse_function_key(&upper).ok_or_else(|| format!("unknown key `<{name}>`")),
    }
}

fn parse_function_key(upper: &str) -> Option<KeyCode> {
    let num = upper.strip_prefix('F')?.parse::<u8>().ok()?;
    (1..=35).contains(&num).then_some(KeyCode::Named(NamedKey::F(num)))
}
