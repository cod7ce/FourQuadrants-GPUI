use super::*;

pub(super) fn style(row: Stateful<Div>, id: &str, doc: &Document, p: Palette) -> Stateful<Div> {
    let focused = doc.pinned_task_ids.contains(id);
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
