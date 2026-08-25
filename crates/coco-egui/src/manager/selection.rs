//! The machine list's selection: zero, one, or many selected row indices,
//! plus the anchor a Shift-click range extends from
//! (`manager/list.rs`'s click handler builds these; `manager/toolbar.rs`'s
//! transport tiles, the bulk context menu, and `manager/detail.rs`'s
//! single-machine pane consume them).
//!
//! Indices are into `ManagerApp::entries`, so any mutation that reorders or
//! shrinks that list must fix the selection up in the same move:
//! [`Selection::snapshot`]/[`Selection::restore`] for a re-sort (a rename's
//! `lifecycle::migrate_slug`), [`Selection::remove_index`] for a removal
//! (`lifecycle::delete_machine`).

use std::cmp::Ordering;
use std::collections::BTreeSet;

use super::MachineEntry;

/// See the module doc. `rows` is a `BTreeSet` so [`Self::iter`] always
/// yields indices in ascending order — every caller that applies a bulk
/// action or builds a bulk-menu list wants that, and a `BTreeSet` gives it
/// for free instead of every caller sorting a `Vec`.
#[derive(Default, Debug, Clone)]
pub(crate) struct Selection {
    rows: BTreeSet<usize>,
    /// The row a Shift-click range is measured from. Set by a plain click
    /// or a Cmd/Ctrl-click (the row toggled either on or off becomes the
    /// new anchor either way); never moved by a Shift-click itself, so
    /// repeated Shift-clicks keep extending/shrinking from the same start —
    /// the Finder/Explorer convention. Always `None` exactly when `rows` is
    /// empty; every mutator maintains that pairing, via
    /// [`Self::drop_anchor_if_empty`] for the two (`remove_index`,
    /// `restore`) that only filter/remap existing indices rather than
    /// setting the anchor directly.
    anchor: Option<usize>,
}

impl Selection {
    /// Plain click: select exactly `i`, replacing whatever was selected.
    pub(super) fn set_single(&mut self, i: usize) {
        self.rows.clear();
        self.rows.insert(i);
        self.anchor = Some(i);
    }

    /// Cmd/Ctrl-click: flip `i`'s membership, leaving every other row's
    /// selection untouched.
    pub(super) fn toggle(&mut self, i: usize) {
        if !self.rows.remove(&i) {
            self.rows.insert(i);
        }
        self.anchor = if self.rows.is_empty() { None } else { Some(i) };
    }

    /// Shift-click: replace the selection with the inclusive range between
    /// `anchor` and `i`, in either order. `anchor` becomes the new anchor too.
    pub(super) fn select_range(&mut self, anchor: usize, i: usize) {
        self.rows = (anchor.min(i)..=anchor.max(i)).collect();
        self.anchor = Some(anchor);
    }

    /// Cmd/Ctrl-A: select every row `0..count`.
    pub(super) fn select_all(&mut self, count: usize) {
        self.rows = (0..count).collect();
        self.anchor = count.checked_sub(1);
    }

    pub(super) fn clear(&mut self) {
        self.rows.clear();
        self.anchor = None;
    }

    pub(crate) fn contains(&self, i: usize) -> bool {
        self.rows.contains(&i)
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The current anchor, for [`super::list`]'s Shift-click handler to
    /// extend from. An anchor-less Shift-click behaves as a plain click.
    pub(super) fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    /// The selected row, only when there is exactly one — the shape the
    /// single-machine detail pane needs ([`super::ManagerApp::draw_detail`]).
    pub(crate) fn single(&self) -> Option<usize> {
        let mut rows = self.rows.iter();
        let only = *rows.next()?;
        rows.next().is_none().then_some(only)
    }

    /// Every selected index, ascending.
    pub(super) fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().copied()
    }

    /// Capture every selected row's slug (and the anchor's), so
    /// [`Self::restore`] can re-find them once `entries` has been reordered.
    /// Call after any in-place slug edit but before the reorder itself.
    pub(super) fn snapshot(&self, entries: &[MachineEntry]) -> SelectionSnapshot {
        SelectionSnapshot {
            rows: self
                .rows
                .iter()
                .filter_map(|&i| entries.get(i))
                .map(|e| e.slug.clone())
                .collect(),
            anchor: self
                .anchor
                .and_then(|a| entries.get(a))
                .map(|e| e.slug.clone()),
        }
    }

    /// Restore a [`Self::snapshot`] against `entries`' current order. A
    /// slug the snapshot recorded but that no longer exists is simply dropped.
    pub(super) fn restore(&mut self, entries: &[MachineEntry], snapshot: &SelectionSnapshot) {
        self.rows = snapshot
            .rows
            .iter()
            .filter_map(|slug| entries.iter().position(|e| &e.slug == slug))
            .collect();
        self.anchor = snapshot
            .anchor
            .as_ref()
            .and_then(|slug| entries.iter().position(|e| &e.slug == slug));
        self.drop_anchor_if_empty();
    }

    /// Fix up indices after `entries.remove(index)`: `index` itself drops
    /// out of the selection, everything past it shifts down by one, and the
    /// anchor follows the same rule.
    pub(super) fn remove_index(&mut self, index: usize) {
        self.rows = self
            .rows
            .iter()
            .filter_map(|&i| shift_down(i, index))
            .collect();
        self.anchor = self.anchor.and_then(|a| shift_down(a, index));
        self.drop_anchor_if_empty();
    }

    /// Enforce "the anchor is `None` exactly when `rows` is empty" after a
    /// mutation that can empty `rows` without itself deciding the anchor's
    /// fate. Left unfixed, a later Shift-click with nothing selected would
    /// range-select from a ghost anchor instead of behaving like a plain click.
    fn drop_anchor_if_empty(&mut self) {
        if self.rows.is_empty() {
            self.anchor = None;
        }
    }
}

/// One index's fate when `entries.remove(removed)` runs: gone if it *was*
/// `removed`, shifted down by one if it came after, unchanged if before.
fn shift_down(i: usize, removed: usize) -> Option<usize> {
    match i.cmp(&removed) {
        Ordering::Less => Some(i),
        Ordering::Equal => None,
        Ordering::Greater => Some(i - 1),
    }
}

/// [`Selection::snapshot`]'s output: the selection's identity by slug
/// rather than index, so it survives an index shuffle.
#[derive(Debug, Clone)]
pub(crate) struct SelectionSnapshot {
    rows: Vec<String>,
    anchor: Option<String>,
}

#[cfg(test)]
#[path = "selection_test.rs"]
mod tests;
