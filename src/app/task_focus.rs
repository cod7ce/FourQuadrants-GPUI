use super::*;

#[derive(Default)]
pub(super) struct TaskFocus {
    ids: HashSet<String>,
}

impl TaskFocus {
    // Opening a task preserves marks; only Shift-click toggles one task.
    pub(super) fn click(&mut self, id: &str, shift: bool, count: usize) -> bool {
        if count >= 2 {
            return count == 2;
        }
        if shift && !self.ids.remove(id) {
            self.ids.insert(id.to_owned());
        }
        false
    }

    pub(super) fn retain_existing(&mut self, doc: &Document) {
        self.ids.retain(|id| doc.task(id).is_some());
    }

    pub(super) fn style(&self, row: Stateful<Div>, id: &str, p: Palette) -> Stateful<Div> {
        let focused = self.ids.contains(id);
        row.when(focused, |row| {
            row.bg(p.accent.opacity(if p.dark { 0.16 } else { 0.12 }))
        })
        .hover(move |style| {
            style.bg(if focused {
                p.accent.opacity(if p.dark { 0.22 } else { 0.18 })
            } else {
                p.hover
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TaskFocus;
    use crate::model::{Document, Task};
    use std::collections::HashSet;

    #[test]
    fn ordinary_clicks_preserve_marks_on_both_marked_and_unmarked_tasks() {
        let mut focus = TaskFocus::default();
        assert!(!focus.click("a", false, 1));
        assert!(focus.ids.is_empty());
        focus.click("a", true, 1);
        focus.click("b", true, 1);
        assert!(!focus.click("a", false, 1));
        assert!(!focus.click("c", false, 1));
        assert_eq!(focus.ids, HashSet::from(["a".into(), "b".into()]));
    }

    #[test]
    fn shift_click_toggles_one_member_without_changing_the_others() {
        let mut focus = TaskFocus::default();
        focus.click("a", true, 1);
        focus.click("b", true, 1);
        focus.click("c", true, 1);
        focus.click("b", true, 1);
        assert_eq!(focus.ids, HashSet::from(["a".into(), "c".into()]));
        focus.click("a", true, 1);
        assert_eq!(focus.ids, HashSet::from(["c".into()]));
    }

    #[test]
    fn double_click_opens_without_adding_or_clearing_marks() {
        let mut focus = TaskFocus::default();
        focus.click("marked", true, 1);
        for id in ["marked", "unmarked"] {
            assert!(!focus.click(id, false, 1));
            assert!(focus.click(id, false, 2));
            assert!(!focus.click(id, false, 3));
            assert_eq!(focus.ids, HashSet::from(["marked".into()]));
        }
    }

    #[test]
    fn deleted_subtree_is_pruned_without_clearing_other_tasks() {
        let mut doc = Document {
            tasks: vec![
                Task {
                    id: "parent".into(),
                    ..Default::default()
                },
                Task {
                    id: "child".into(),
                    parent_id: Some("parent".into()),
                    ..Default::default()
                },
                Task {
                    id: "other".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut focus = TaskFocus::default();
        for id in ["parent", "child", "other"] {
            focus.click(id, true, 1);
        }
        doc.delete_task("parent");
        focus.retain_existing(&doc);
        assert_eq!(focus.ids, HashSet::from(["other".into()]));
    }
}
