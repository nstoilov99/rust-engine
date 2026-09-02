//! Doc-local undo/redo with saved-cursor dirty tracking, generic over the
//! edit type. The contract `GraphEditStack` set and `CurveEditStack` copied:
//! dirty is the distance from the save point, not a sticky flag, and a
//! post-undo edit that truncates the branch holding the save point loses it.
//!
//! `CurveEditStack` and `BlendSpaceEditStack` are aliases of [`EditStack`];
//! each document type supplies its reversible edit via [`ReversibleEdit`].

/// A reversible document edit: stores enough to both apply and revert on its
/// own, and names itself verb-object for the Edit menu (M10).
pub trait ReversibleEdit {
    type Doc;
    fn apply(&self, doc: &mut Self::Doc);
    fn revert(&self, doc: &mut Self::Doc);
    fn description(&self) -> String;
}

pub struct EditStack<E> {
    undo: Vec<E>,
    redo: Vec<E>,
    saved: Option<usize>,
}

impl<E> Default for EditStack<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> EditStack<E> {
    /// A stack for a freshly loaded (clean) document.
    pub fn new() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), saved: Some(0) }
    }

    /// Record an edit that has *already* been applied to the doc.
    pub fn record(&mut self, edit: E) {
        if let Some(s) = self.saved {
            if s > self.undo.len() {
                self.saved = None;
            }
        }
        self.undo.push(edit);
        self.redo.clear();
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.undo.len());
    }

    pub fn is_dirty(&self) -> bool {
        self.saved != Some(self.undo.len())
    }
}

impl<E: ReversibleEdit> EditStack<E> {
    pub fn undo(&mut self, doc: &mut E::Doc) -> Option<String> {
        let edit = self.undo.pop()?;
        edit.revert(doc);
        let desc = edit.description();
        self.redo.push(edit);
        Some(desc)
    }

    pub fn redo(&mut self, doc: &mut E::Doc) -> Option<String> {
        let edit = self.redo.pop()?;
        edit.apply(doc);
        let desc = edit.description();
        self.undo.push(edit);
        Some(desc)
    }

    pub fn undo_description(&self) -> Option<String> {
        self.undo.last().map(E::description)
    }

    pub fn redo_description(&self) -> Option<String> {
        self.redo.last().map(E::description)
    }
}
