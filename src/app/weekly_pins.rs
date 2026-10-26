use super::*;

struct PinnedTask<'a> {
    task: &'a Task,
    children: Vec<PinnedTask<'a>>,
    completed: bool,
    count: usize,
}

fn insert_path<'a>(nodes: &mut Vec<PinnedTask<'a>>, path: &[&'a Task], completed: bool) {
    let Some((task, rest)) = path.split_first() else {
        return;
    };
    let index = nodes
        .iter()
        .position(|node| node.task.id == task.id)
        .unwrap_or_else(|| {
            nodes.push(PinnedTask {
                task,
                children: Vec::new(),
                completed,
                count: 0,
            });
            nodes.len() - 1
        });
    let node = &mut nodes[index];
    node.completed &= completed;
    node.count += 1;
    insert_path(&mut node.children, rest, completed);
}

fn pin_tree<'a>(
    doc: &'a Document,
    pinned: &'a [Task],
    completed: impl Fn(&Task) -> bool,
) -> Vec<PinnedTask<'a>> {
    let mut tasks = pinned.iter().collect::<Vec<_>>();
    // Keep each family together, ordered by its first visible pin rather than its context parents.
    tasks.sort_by_key(|task| completed(task));
    let mut roots = Vec::new();
    for task in tasks {
        let mut path = vec![task];
        let mut parent = task.parent_id.as_deref();
        while let Some(ancestor) = parent.and_then(|id| doc.task(id)) {
            path.push(ancestor);
            parent = ancestor.parent_id.as_deref();
        }
        path.reverse();
        insert_path(&mut roots, &path, completed(task));
    }
    roots
}

impl Workspace {
    pub(super) fn weekly_pinned_rows(
        &self,
        tasks: &[Task],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let roots = pin_tree(&self.doc, tasks, |task| self.grouped_as_completed(task));
        let completed = roots
            .iter()
            .filter(|node| node.completed)
            .map(|node| node.count)
            .sum::<usize>();
        let p = self.palette(window);
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .children(
                roots
                    .iter()
                    .filter(|node| !node.completed)
                    .map(|node| self.weekly_pinned_branch(node, window, cx)),
            )
            .when(completed > 0, |el| {
                el.child(self.completed_heading(completed, p))
            })
            .children(
                roots
                    .iter()
                    .filter(|node| node.completed)
                    .map(|node| self.weekly_pinned_branch(node, window, cx)),
            )
    }

    fn weekly_pinned_branch(
        &self,
        node: &PinnedTask<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette(window);
        let branch_y = window.rem_size() * (0.5 + 0.875 * 1.45 / 2.);
        let child_indent = px(TASK_CHECKBOX_SIZE + TASK_TITLE_GAP);
        let branch_width = child_indent - px(TASK_CHECKBOX_SIZE / 2. + TASK_CONNECTOR_GAP);
        let stem_top = branch_y + px(TASK_CHECKBOX_SIZE / 2. + TASK_CONNECTOR_GAP);
        let branch_color = p.muted.opacity(0.22);
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .child(
                div()
                    .relative()
                    .pb(px(3.))
                    .child(self.flat_row(node.task, window, cx))
                    .when(!node.children.is_empty(), |el| {
                        el.child(task_connector(
                            TaskConnector::Stem { top: stem_top },
                            branch_color,
                        ))
                    }),
            )
            .children(node.children.iter().enumerate().map(|(index, child)| {
                div()
                    .relative()
                    .min_w_0()
                    .flex_shrink_0()
                    .pl(child_indent)
                    .child(task_connector(
                        TaskConnector::Branch {
                            y: branch_y,
                            width: branch_width,
                            continues: index + 1 < node.children.len(),
                        },
                        branch_color,
                    ))
                    .child(self.weekly_pinned_branch(child, window, cx))
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::{PinnedTask, pin_tree};
    use crate::model::{Document, Task};

    fn task(id: &str, parent: Option<&str>) -> Task {
        Task {
            id: id.into(),
            parent_id: parent.map(str::to_owned),
            ..Default::default()
        }
    }

    fn rows<'a>(nodes: &[PinnedTask<'a>]) -> Vec<(&'a str, usize)> {
        fn visit<'a>(nodes: &[PinnedTask<'a>], depth: usize, result: &mut Vec<(&'a str, usize)>) {
            for node in nodes {
                result.push((node.task.id.as_str(), depth));
                visit(&node.children, depth + 1, result);
            }
        }
        let mut result = Vec::new();
        visit(nodes, 0, &mut result);
        result
    }

    #[test]
    fn pinned_descendants_include_ancestors_once_without_unselected_siblings() {
        let doc = Document {
            tasks: vec![
                task("root", None),
                task("parent", Some("root")),
                task("first", Some("parent")),
                task("second", Some("parent")),
                task("unselected", Some("parent")),
            ],
            ..Default::default()
        };
        let pinned = vec![
            doc.tasks[3].clone(),
            doc.tasks[2].clone(),
            doc.tasks[1].clone(),
        ];
        let tree = pin_tree(&doc, &pinned, |task| task.completed);
        assert_eq!(
            rows(&tree),
            [("root", 0), ("parent", 1), ("second", 2), ("first", 2)]
        );
        assert_eq!(tree[0].count, 3);
        assert_eq!(doc.tasks.len(), 5);

        let remaining = vec![doc.tasks[2].clone()];
        assert_eq!(
            rows(&pin_tree(&doc, &remaining, |task| task.completed)),
            [("root", 0), ("parent", 1), ("first", 2)]
        );
        assert!(pin_tree(&doc, &[], |task| task.completed).is_empty());
    }

    #[test]
    fn ancestor_context_survives_its_own_week_and_completion_filters() {
        let mut parent = task("parent", None);
        parent.completed = true;
        parent.week_start -= chrono::Duration::weeks(1);
        let child = task("child", Some("parent"));
        let doc = Document {
            tasks: vec![parent, child.clone()],
            ..Default::default()
        };
        // Only the child passed the selected-week/search/hide-completed filters.
        let pinned = vec![child];
        let tree = pin_tree(&doc, &pinned, |task| task.completed);
        assert_eq!(rows(&tree), [("parent", 0), ("child", 1)]);
        assert!(!tree[0].completed);
        assert!(tree[0].task.completed);
        assert_eq!(tree[0].count, 1);
    }

    #[test]
    fn family_order_and_completion_sections_follow_visible_pins() {
        let parent = task("parent", None);
        let mut done = task("done", Some("parent"));
        done.completed = true;
        let open = task("open", Some("parent"));
        let other = task("other", None);
        let doc = Document {
            tasks: vec![parent, done.clone(), open.clone(), other.clone()],
            ..Default::default()
        };
        // Completion sorting partitions the existing date order before parents are added.
        let pinned = vec![done.clone(), other, open];
        let tree = pin_tree(&doc, &pinned, |task| task.completed);
        assert_eq!(
            rows(&tree),
            [("other", 0), ("parent", 0), ("open", 1), ("done", 1)]
        );
        assert!(!tree[1].completed);
        assert_eq!(tree[1].count, 2);

        let pinned = vec![done];
        let tree = pin_tree(&doc, &pinned, |_| true);
        assert!(tree[0].completed);
        assert_eq!(tree[0].count, 1);
        // During completion feedback, the same family remains in the open section.
        assert!(!pin_tree(&doc, &pinned, |_| false)[0].completed);
    }
}
