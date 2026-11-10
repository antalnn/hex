//! Reordering a saved list by dragging one of its rows.
//!
//! The Modes pane's transformations and the Output pane's steps share this.
//! A pane may show fewer rows than the list holds: Output leaves AI cleanup
//! out while its switch is off. The dragged row moves among the visible rows
//! only, and every hidden entry keeps its slot, so turning it back on finds
//! it where it was.
//!
//! The list as it was when the drag began is kept beside the previewed
//! order; the caller saves the preview once, on release, and drops it on a
//! cancelled drag.

/// One row being dragged: the list at the start, the visible rows' slots in
/// that list, and the list as the drag has rearranged it.
pub(super) struct OrderDrag {
    id: String,
    /// The dragged row's place among the visible rows.
    start: usize,
    /// The visible rows' indices in `committed`, in the order they show.
    slots: Vec<usize>,
    committed: Vec<String>,
    order: Vec<String>,
}

impl OrderDrag {
    pub(super) fn begin(id: &str, start: usize, slots: Vec<usize>, committed: Vec<String>) -> Self {
        Self {
            id: id.to_string(),
            start,
            slots,
            order: committed.clone(),
            committed,
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn order(&self) -> &[String] {
        &self.order
    }

    /// Moves the dragged row `moved` visible rows from where it started. The
    /// visible rows are put back into their own slots, so a hidden entry
    /// stays exactly where it was.
    pub(super) fn preview(&mut self, moved: isize) {
        if self.slots.is_empty() {
            return;
        }
        let last = self.slots.len().saturating_sub(1) as isize;
        let target = (self.start as isize + moved).clamp(0, last) as usize;
        let mut visible = self
            .slots
            .iter()
            .map(|&slot| self.committed[slot].clone())
            .collect::<Vec<_>>();
        let Some(from) = visible.iter().position(|id| id == &self.id) else {
            return;
        };
        let value = visible.remove(from);
        visible.insert(target.min(visible.len()), value);
        let mut order = self.committed.clone();
        for (row, &slot) in self.slots.iter().enumerate() {
            order[slot] = visible[row].clone();
        }
        self.order = order;
    }

    /// The order to save on release, or `None` when the drag was cancelled or
    /// left the list as it was.
    pub(super) fn finish(self, cancelled: bool) -> Option<Vec<String>> {
        (!cancelled && self.order != self.committed).then_some(self.order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn a_dragged_row_moves_among_its_visible_neighbours() {
        let committed = list(&["one", "ai-rewrite", "two", "three"]);
        let mut drag = OrderDrag::begin("three", 2, vec![0, 2, 3], committed);
        drag.preview(-1);
        assert_eq!(
            drag.finish(false),
            Some(list(&["one", "ai-rewrite", "three", "two"]))
        );
    }

    #[test]
    fn a_hidden_entry_keeps_its_slot() {
        let committed = list(&["one", "ai-rewrite", "two", "three"]);
        let mut drag = OrderDrag::begin("three", 2, vec![0, 2, 3], committed);
        drag.preview(-2);
        assert_eq!(
            drag.finish(false),
            Some(list(&["three", "ai-rewrite", "one", "two"]))
        );
    }

    #[test]
    fn a_cancelled_or_unchanged_drag_saves_nothing() {
        let committed = list(&["one", "two"]);
        let mut drag = OrderDrag::begin("one", 0, vec![0, 1], committed.clone());
        drag.preview(0);
        assert!(drag.finish(false).is_none());

        let mut drag = OrderDrag::begin("one", 0, vec![0, 1], committed);
        drag.preview(1);
        assert!(drag.finish(true).is_none());
    }

    #[test]
    fn an_unknown_row_leaves_the_list_alone() {
        let committed = list(&["one", "two"]);
        let mut drag = OrderDrag::begin("missing", 0, vec![0, 1], committed);
        drag.preview(1);
        assert!(drag.finish(false).is_none());
    }
}
