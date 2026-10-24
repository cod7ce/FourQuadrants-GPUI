use super::*;

impl Workspace {
    pub(super) fn calendar(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let workspace = cx.entity().downgrade();
        canvas(
            move |bounds, window, cx| {
                let mut calendar = workspace
                    .update(cx, |this, cx| this.calendar_layout(bounds.size, window, cx))
                    .ok()?;
                calendar.layout_as_root(bounds.size.map(AvailableSpace::Definite), window, cx);
                calendar.prepaint_at(bounds.origin, window, cx);
                Some(calendar)
            },
            |_, calendar, window, cx| {
                if let Some(mut calendar) = calendar {
                    calendar.paint(window, cx);
                }
            },
        )
        .flex_1()
        .min_w_0()
        .min_h_0()
        .into_any_element()
    }

    fn calendar_layout(
        &self,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette(window);
        let en = self.doc.settings.language == "en";
        let stacked = size.width < px(720.);
        let detail_width = px((f32::from(size.width) * 0.30).clamp(240., 320.));
        let month_width = if stacked {
            size.width
        } else {
            size.width - detail_width
        };
        let month_height = if stacked {
            px((f32::from(size.height) * 0.62).clamp(420., 560.))
        } else {
            size.height
        };
        let today = model::today();
        let first = self.day.with_day(1).unwrap();
        let start = model::week_start(first);
        let mut tasks = self
            .doc
            .tasks
            .iter()
            .filter(|task| self.visible(task, cx))
            .collect::<Vec<_>>();
        tasks.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order).then(a.id.cmp(&b.id)));
        let month_title = if en {
            first.format("%B %Y").to_string()
        } else {
            format!("{} 年 {} 月", first.year(), first.month())
        };
        let mut month = div()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .p_4()
            .when(stacked, |el| el.w_full().h(month_height).flex_shrink_0())
            .when(!stacked, |el| el.flex_1())
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .min_h(px(32.))
                    .mb_2()
                    .child(
                        icon_button(
                            "calendar-previous",
                            IconName::ChevronLeft,
                            self.t("上个月", "Previous month"),
                            p,
                        )
                        .rounded_full()
                        .on_click(cx.listener(|this, _, window, cx| this.navigate(-1, window, cx))),
                    )
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(month_title))
                    .child(
                        icon_button(
                            "calendar-next",
                            IconName::ChevronRight,
                            self.t("下个月", "Next month"),
                            p,
                        )
                        .rounded_full()
                        .on_click(cx.listener(|this, _, window, cx| this.navigate(1, window, cx))),
                    )
                    .child(div().flex_1())
                    .when(self.day != today, |el| {
                        el.child(
                            chip("calendar-today", self.t("今天", "Today").to_string(), p)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.return_to_current_week(window, cx)
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .h(px(28.))
                    .flex_shrink_0()
                    .children((0..7).map(|col| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(p.muted)
                            .child(if en {
                                ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"][col]
                            } else {
                                ["一", "二", "三", "四", "五", "六", "日"][col]
                            })
                    })),
            );
        let mut grid = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .border_t_1()
            .border_l_1()
            .border_color(p.border);
        // Short cells use one marker row; reserve space for the overflow count.
        let dot_rows = if month_height < px(480.) { 1 } else { 2 };
        let dots_per_row = (((f32::from(month_width) - 32.) / 7. - 13. + 4.) / 10.)
            .floor()
            .max(1.) as usize;
        for row in 0..6 {
            let mut cells = div().flex().flex_1().min_h_0();
            for col in 0..7 {
                let n = row * 7 + col;
                let date = start + Duration::days(n as i64);
                let day_tasks = tasks
                    .iter()
                    .copied()
                    .filter(|task| task.agenda_date() == Some(date))
                    .collect::<Vec<_>>();
                let capacity = (dots_per_row * dot_rows).min(12);
                let dot_count = if day_tasks.len() > capacity {
                    day_tasks
                        .len()
                        .min((dots_per_row * dot_rows).saturating_sub(3).clamp(1, 12))
                } else {
                    day_tasks.len()
                };
                let extra_count = day_tasks.len() - dot_count;
                let selected = date == self.day;
                let tooltip = format!(
                    "{} · {} {}",
                    date,
                    day_tasks.len(),
                    self.t("项任务", "tasks")
                );
                cells = cells.child(
                    div()
                        .id(("date", n as usize))
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .p(px(6.))
                        .border_r_1()
                        .border_b_1()
                        .border_color(p.border)
                        .when(col >= 5, |el| {
                            el.bg(p.text.opacity(if p.dark { 0.035 } else { 0.025 }))
                        })
                        .when(selected, |el| el.bg(p.accent.opacity(0.08)))
                        .hover(move |style| style.bg(p.hover))
                        .cursor_pointer()
                        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.save_week_note(cx) && this.save_editor(window, cx) {
                                this.day = date;
                                this.week = model::week_start(date);
                                this.load_week_note(window, cx);
                                cx.notify();
                            }
                        }))
                        .drag_over::<DragTask>(move |style, _, _, _| {
                            style.bg(p.accent.opacity(0.18)).border_color(p.accent)
                        })
                        .on_drop(cx.listener(move |this, drag: &DragTask, _, cx| {
                            cx.stop_propagation();
                            let id = drag.id.clone();
                            this.change(cx, |doc| {
                                doc.schedule(&id, date);
                                Ok(())
                            });
                        }))
                        .child(
                            div()
                                .size(px(24.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .text_sm()
                                .text_color(if date.month() == first.month() {
                                    p.text
                                } else {
                                    p.muted.opacity(0.55)
                                })
                                .when(date == today, |el| {
                                    el.bg(p.accent)
                                        .text_color(p.accent_text)
                                        .font_weight(FontWeight::SEMIBOLD)
                                })
                                .child(date.day().to_string()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(4.))
                                .children(day_tasks.iter().take(dot_count).map(|task| {
                                    div()
                                        .size(px(6.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(p.quadrant[task.quadrant()])
                                }))
                                .when(extra_count > 0, |el| {
                                    el.child(
                                        div()
                                            .text_size(px(9.))
                                            .line_height(px(10.))
                                            .text_color(p.muted)
                                            .child(format!("+{extra_count}")),
                                    )
                                }),
                        )
                        .when(selected, |el| {
                            el.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .border_1()
                                    .border_color(p.accent.opacity(0.55)),
                            )
                        }),
                );
            }
            grid = grid.child(cells);
        }
        month = month.child(grid).child(
            div()
                .flex()
                .flex_wrap()
                .flex_shrink_0()
                .items_center()
                .gap_x_3()
                .gap_y_1()
                .pt_3()
                .children((0..4).map(|q| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .text_xs()
                        .text_color(p.muted)
                        .child(
                            div()
                                .size(px(6.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(p.quadrant[q]),
                        )
                        .child(theme::quadrant_name(q, en))
                })),
        );
        let day_tasks = tasks
            .into_iter()
            .filter(|task| task.agenda_date() == Some(self.day))
            .collect::<Vec<_>>();
        let completed = day_tasks
            .iter()
            .filter(|task| self.grouped_as_completed(task))
            .count();
        let pending = day_tasks.len() - completed;
        let date_title = if en {
            self.day.format("%B %-d, %A").to_string()
        } else {
            let weekday = [
                "星期一",
                "星期二",
                "星期三",
                "星期四",
                "星期五",
                "星期六",
                "星期日",
            ][self.day.weekday().num_days_from_monday() as usize];
            format!("{}月{}日  {}", self.day.month(), self.day.day(), weekday)
        };
        let mut counts = Vec::new();
        if pending > 0 {
            counts.push(format!("{} {}", pending, self.t("待办", "pending")));
        }
        if completed > 0 {
            counts.push(format!("{} {}", completed, self.t("已完成", "completed")));
        }
        if counts.is_empty() {
            counts.push(self.t("无任务", "No tasks").to_string());
        }
        let detail =
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .border_color(p.border)
                .when(stacked, |el| {
                    el.w_full().min_h(px(200.)).flex_shrink_0().border_t_1()
                })
                .when(!stacked, |el| {
                    el.w(detail_width).min_h_0().flex_shrink_0().border_l_1()
                })
                .child(
                    div()
                        .p_4()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .border_b_1()
                        .border_color(p.border)
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(date_title)
                                .when(self.day == today, |el| {
                                    el.child(
                                        div()
                                            .px_2()
                                            .py(px(2.))
                                            .rounded_full()
                                            .bg(p.accent.opacity(0.14))
                                            .text_color(p.accent)
                                            .text_xs()
                                            .child(self.t("今天", "Today").to_string()),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted)
                                .child(counts.join(" · ")),
                        ),
                )
                .child(
                    div()
                        .id("calendar-day-list")
                        .min_w_0()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when(!stacked, |el| el.flex_1().min_h_0().overflow_y_scroll())
                        .when(day_tasks.is_empty(), |el| {
                            el.child(
                                div().py_4().text_sm().text_color(p.muted).child(
                                    self.t("当天暂无任务", "No tasks for this day").to_string(),
                                ),
                            )
                        })
                        .children(
                            day_tasks
                                .iter()
                                .filter(|task| !self.grouped_as_completed(task))
                                .map(|task| self.flat_row(task, window, cx)),
                        )
                        .when(completed > 0, |el| {
                            el.child(self.completed_heading(completed, p))
                        })
                        .children(
                            day_tasks
                                .iter()
                                .filter(|task| self.grouped_as_completed(task))
                                .map(|task| self.flat_row(task, window, cx)),
                        ),
                );
        div()
            .id("calendar-layout")
            .w(size.width)
            .h(size.height)
            .flex()
            .when(stacked, |el| el.flex_col().overflow_y_scroll())
            .child(month)
            .child(detail)
            .into_any_element()
    }
}
