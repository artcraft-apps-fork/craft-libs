//! Per-mode key sequence tries supporting exact matches, prefix waiting, and ambiguous timeouts.

use std::collections::HashMap;

use crate::action::{ModalAction, Operator};
use crate::token::KeyToken;

/// Binding table domain (`nmap`, `vmap`, `omap`, `imap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapDomain {
    Normal,
    Visual,
    OperatorPending,
    Insert,
}

/// Right-hand side of a mapped key sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rhs {
    /// Built-in or `<Action>(...)` operation.
    Action(ModalAction),
    /// Enters operator-pending state in Normal mode (`y`, `d`, `c`, `gh`, `gu`, `gs`, `gn`),
    /// or applies immediately in Visual mode.
    Operator { op: Operator, double_tail: KeyToken },
    /// Waits for a single character argument (`f`, `F`, `t`, `T`).
    CharMotion { forward: bool, till: bool },
    /// Macro / user mapping to feed back into the engine (`remap = false` for `*noremap`).
    Keys { keys: Vec<KeyToken>, remap: bool },
    /// `<Nop>`: swallow the sequence and do nothing.
    Nop,
}

/// Result of querying a [`KeyTrie`] with a sequence of [`KeyToken`]s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrieLookup<'a> {
    /// No binding starts with this sequence.
    None,
    /// Strict prefix of at least one longer binding.
    Prefix,
    /// Exact match with no longer continuations.
    Exact(&'a Rhs),
    /// Exact match AND prefix of at least one longer binding (resolved on next key or timeout).
    ExactAndPrefix(&'a Rhs),
}

#[derive(Clone, Debug, Default)]
struct Node {
    value: Option<Rhs>,
    children: HashMap<KeyToken, Node>,
}

impl Node {
    fn is_empty(&self) -> bool {
        self.value.is_none() && self.children.is_empty()
    }
}

/// Prefix tree of [`KeyToken`] sequences for one [`MapDomain`].
#[derive(Clone, Debug, Default)]
pub struct KeyTrie {
    root: Node,
}

impl KeyTrie {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.root = Node::default();
    }

    pub fn insert(&mut self, seq: &[KeyToken], rhs: Rhs) {
        if seq.is_empty() {
            return;
        }
        let mut node = &mut self.root;
        for &tok in seq {
            node = node.children.entry(tok.normalized()).or_default();
        }
        node.value = Some(rhs);
    }

    pub fn remove(&mut self, seq: &[KeyToken]) -> bool {
        if seq.is_empty() {
            return false;
        }
        remove_rec(&mut self.root, seq)
    }

    pub fn lookup(&self, seq: &[KeyToken]) -> TrieLookup<'_> {
        if seq.is_empty() {
            return TrieLookup::None;
        }
        let mut node = &self.root;
        for tok in seq {
            let norm = tok.normalized();
            let Some(next) = node.children.get(&norm) else {
                return TrieLookup::None;
            };
            node = next;
        }
        match (&node.value, node.children.is_empty()) {
            (Some(rhs), true) => TrieLookup::Exact(rhs),
            (Some(rhs), false) => TrieLookup::ExactAndPrefix(rhs),
            (None, false) => TrieLookup::Prefix,
            (None, true) => TrieLookup::None,
        }
    }
}

fn remove_rec(node: &mut Node, seq: &[KeyToken]) -> bool {
    let Some((&first, rest)) = seq.split_first() else {
        let had = node.value.is_some();
        node.value = None;
        return had;
    };
    let norm = first.normalized();
    let Some(child) = node.children.get_mut(&norm) else {
        return false;
    };
    let removed = remove_rec(child, rest);
    if child.is_empty() {
        node.children.remove(&norm);
    }
    removed
}

/// Collection of tries for Normal, Visual, OperatorPending, and Insert modes.
#[derive(Clone, Debug, Default)]
pub struct KeyTables {
    pub normal: KeyTrie,
    pub visual: KeyTrie,
    pub op_pending: KeyTrie,
    pub insert: KeyTrie,
}

impl KeyTables {
    pub fn domain(&self, domain: MapDomain) -> &KeyTrie {
        match domain {
            MapDomain::Normal => &self.normal,
            MapDomain::Visual => &self.visual,
            MapDomain::OperatorPending => &self.op_pending,
            MapDomain::Insert => &self.insert,
        }
    }

    pub fn domain_mut(&mut self, domain: MapDomain) -> &mut KeyTrie {
        match domain {
            MapDomain::Normal => &mut self.normal,
            MapDomain::Visual => &mut self.visual,
            MapDomain::OperatorPending => &mut self.op_pending,
            MapDomain::Insert => &mut self.insert,
        }
    }
}
