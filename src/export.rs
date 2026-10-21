//! Markdown export for a single week.
//!
//! Mirrors the native `MarkdownExporter`: only top-level tasks in the chosen
//! week are grouped, a task with several tags appears under each tag, untagged
//! tasks are listed last, and descendants are indented beneath their parent.

use chrono::NaiveDate;
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::model::{Document, Task, week_start};

/// Generates Markdown for `week`. Language and default-title strings follow
/// `document.settings.language` (`zh` or `en`).
pub fn markdown(document: &Document, week: NaiveDate) -> String {
    let week = week_start(week);
    let english = document.settings.language == "en";
    let title = if english { "This Week" } else { "本周工作" };
    let untagged_label = if english { "Untagged" } else { "未分类" };
    let default_task_title = if english { "New task" } else { "新任务" };

    let tag_names: HashMap<&str, &str> = document
        .tags
        .iter()
        .map(|tag| (tag.id.as_str(), tag.name.as_str()))
        .collect();

    let mut top_level: Vec<&Task> = document
        .tasks
        .iter()
        .filter(|task| task.parent_id.is_none() && week_start(task.week_start) == week)
        .collect();
    top_level.sort_by(|a, b| {
        a.sort_order
            .total_cmp(&b.sort_order)
            .then_with(|| a.id.cmp(&b.id))
    });

    let task_names = |task: &Task| -> BTreeSet<String> {
        task.tags
            .iter()
            .filter_map(|tag_id| tag_names.get(tag_id.as_str()))
            .map(|name| (*name).to_string())
            .collect()
    };

    let mut tag_groups: BTreeSet<String> = BTreeSet::new();
    for task in &top_level {
        tag_groups.extend(task_names(task));
    }

    let mut groups: Vec<(String, Vec<&Task>)> = tag_groups
        .into_iter()
        .map(|name| {
            let tasks = top_level
                .iter()
                .copied()
                .filter(|task| task_names(task).contains(&name))
                .collect();
            (name, tasks)
        })
        .collect();

    let untagged: Vec<&Task> = top_level
        .iter()
        .copied()
        .filter(|task| task_names(task).is_empty())
        .collect();
    if !untagged.is_empty() {
        groups.push((untagged_label.to_string(), untagged));
    }

    let mut output = format!("# {title}\n");
    for (name, tasks) in groups {
        output.push_str(&format!("\n## {name}\n\n"));
        for task in tasks {
            render_task(
                document,
                task,
                0,
                default_task_title,
                &mut HashSet::new(),
                &mut output,
            );
        }
    }
    output
}

fn render_task(
    document: &Document,
    task: &Task,
    indent: usize,
    default_title: &str,
    visiting: &mut HashSet<String>,
    output: &mut String,
) {
    if !visiting.insert(task.id.clone()) {
        return;
    }

    let pad = "  ".repeat(indent);
    let mut title = if task.title.is_empty() {
        default_title.to_string()
    } else {
        task.title.clone()
    };
    if let Some(issue_key) = task.issue_key.as_deref()
        && !issue_key.is_empty()
    {
        title = format!("{issue_key} {title}");
    }
    if let Some(link) = task.links.iter().find(|link| !link.is_empty()) {
        title.push_str(&format!(" [[↗]({link})]"));
    }
    output.push_str(&format!("{pad}- {title}\n"));

    for child in document.children(&task.id) {
        render_task(document, child, indent + 1, default_title, visiting, output);
    }
    visiting.remove(&task.id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Tag, WeekNote};

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn add_tag(document: &mut Document, id: &str, name: &str, order: f64) {
        document.tags.push(Tag {
            id: id.to_string(),
            name: name.to_string(),
            color_hex: "#123456".to_string(),
            sort_order: order,
        });
    }

    #[test]
    fn groups_by_tag_duplicates_multi_tag_tasks_and_indents_children() {
        let week = date(2026, 7, 6);
        let mut document = Document::default();
        add_tag(&mut document, "work", "Work", 0.0);
        add_tag(&mut document, "personal", "Personal", 1.0);

        let parent = document.add_task("Parent", week, 0);
        {
            let task = document.task_mut(&parent).unwrap();
            task.issue_key = Some("ABC-1".to_string());
            task.links = vec![
                "https://example.test/first".to_string(),
                "https://example.test/second".to_string(),
            ];
            task.tags = vec!["work".to_string(), "personal".to_string()];
        }

        let child = document.add_task("Child", week, 0);
        document.reparent(&child, Some(&parent)).unwrap();
        document.task_mut(&child).unwrap().links = vec!["https://child.test/item".to_string()];

        let grandchild = document.add_task("Grandchild", week, 0);
        document.reparent(&grandchild, Some(&child)).unwrap();

        let loose = document.add_task("Loose", week, 1);
        document.task_mut(&loose).unwrap().sort_order = 0.0;
        let empty = document.add_task("", week, 3);
        document.task_mut(&empty).unwrap().sort_order = 1.0;

        let other_week = document.add_task("Other week", date(2026, 7, 13), 0);
        document.task_mut(&other_week).unwrap().title = "Other week".to_string();

        document.week_notes.push(WeekNote {
            week_start: week,
            text: "week note".to_string(),
            rich_content: None,
            markdown: None,
        });

        let markdown = markdown(&document, week);

        let personal = markdown.find("## Personal").unwrap();
        let work = markdown.find("## Work").unwrap();
        let untagged = markdown.find("## 未分类").unwrap();
        assert!(personal < work && work < untagged);

        assert_eq!(
            markdown
                .matches("- ABC-1 Parent [[↗](https://example.test/first)]")
                .count(),
            2
        );
        assert!(!markdown.contains("second"));
        assert!(markdown.contains("  - Child [[↗](https://child.test/item)]"));
        assert!(markdown.contains("    - Grandchild"));
        assert!(markdown.contains("- Loose"));
        assert!(markdown.contains("- 新任务"));
        assert!(!markdown.contains("Other week"));
        assert!(!markdown.contains("week note"));
    }

    #[test]
    fn english_headers_and_empty_document() {
        let week = date(2026, 7, 6);
        let mut document = Document::default();
        document.settings.language = "en".to_string();
        assert_eq!(markdown(&document, week), "# This Week\n");

        let id = document.add_task("", week, 0);
        document.task_mut(&id).unwrap().sort_order = 0.0;
        let output = markdown(&document, week);
        assert!(output.starts_with("# This Week\n"));
        assert!(output.contains("## Untagged"));
        assert!(output.contains("- New task"));
    }

    #[test]
    fn only_top_level_tasks_of_the_week_define_groups() {
        let week = date(2026, 7, 6);
        let mut document = Document::default();
        add_tag(&mut document, "work", "Work", 0.0);

        let parent = document.add_task("Parent", week, 0);
        let child = document.add_task("Tagged child", week, 0);
        document.reparent(&child, Some(&parent)).unwrap();
        document.task_mut(&child).unwrap().tags = vec!["work".to_string()];

        let output = markdown(&document, week);
        assert!(!output.contains("## Work"));
        assert!(output.contains("## 未分类"));
        assert!(output.contains("  - Tagged child"));
    }

    #[test]
    fn task_in_same_week_with_unnormalized_date_is_included() {
        let week = date(2026, 7, 6);
        let mut document = Document::default();
        let id = document.add_task("Sunday task", date(2026, 7, 12), 0);
        document.task_mut(&id).unwrap().week_start = date(2026, 7, 12);
        let output = markdown(&document, week);
        assert!(output.contains("Sunday task"));
    }
}
