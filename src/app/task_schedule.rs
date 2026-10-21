use super::*;
use chrono::{Duration, Local, NaiveDateTime, Timelike};
use gpui_component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants},
    calendar::Date,
    date_picker::{DatePicker, DatePickerEvent, DatePickerState},
    popover::Popover,
};

pub(super) struct SchedulePicker {
    date: Entity<DatePickerState>,
    hour: u32,
    minute: u32,
    default_hour: u32,
    _subscription: Subscription,
}

impl SchedulePicker {
    pub(super) fn new(
        value: &str,
        default_hour: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").ok();
        let date = cx.new(|cx| {
            let mut state = DatePickerState::new(window, cx).date_format("%Y-%m-%d");
            state.set_date(Date::Single(selected.map(|date| date.date())), window, cx);
            state
        });
        let subscription = cx.subscribe(&date, |_, _, _: &DatePickerEvent, cx| cx.notify());
        Self {
            date,
            hour: selected.map_or(default_hour, |date| date.hour()),
            minute: selected.map_or(0, |date| date.minute()),
            default_hour,
            _subscription: subscription,
        }
    }

    pub(super) fn value(&self, cx: &App) -> String {
        let Some(date) = self.date.read(cx).date().start() else {
            return String::new();
        };
        format!("{date} {:02}:{:02}", self.hour, self.minute)
    }

    pub(super) fn set_value(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let selected = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").ok();
        self.date.update(cx, |state, cx| {
            state.set_date(Date::Single(selected.map(|date| date.date())), window, cx)
        });
        self.hour = selected.map_or(self.default_hour, |date| date.hour());
        self.minute = selected.map_or(0, |date| date.minute());
        cx.notify();
    }

    fn number_picker(&self, hours: bool, cx: &mut Context<Self>) -> Popover {
        let selected = if hours { self.hour } else { self.minute };
        let numbers: Vec<u32> = if hours {
            (0..24).collect()
        } else {
            (0..60).step_by(5).collect()
        };
        let id = if hours { "hour" } else { "minute" };
        let english = &*gpui_component::locale() == "en";
        let title = match (hours, english) {
            (true, true) => "Hour",
            (false, true) => "Minute",
            (true, false) => "小时",
            (false, false) => "分钟",
        };
        let owner = cx.entity().downgrade();
        Popover::new(id)
            .trigger(
                Button::new(id)
                    .outline()
                    .w(px(62.))
                    .h(px(32.))
                    .label(format!("{selected:02}"))
                    .icon(Icon::new(IconName::ChevronDown).with_size(px(10.)))
                    .disabled(self.date.read(cx).date().start().is_none()),
            )
            .content(move |_, _, cx| {
                let mut grid = div().grid().grid_cols(6).gap(px(3.));
                for number in numbers.iter().copied() {
                    let owner = owner.clone();
                    grid = grid.child(
                        Button::new(("number", number as usize))
                            .ghost()
                            .w(px(32.))
                            .h(px(26.))
                            .label(format!("{number:02}"))
                            .when(number == selected, |button| {
                                button
                                    .bg(cx.theme().primary)
                                    .text_color(cx.theme().primary_foreground)
                            })
                            .on_click(cx.listener(move |popover, _, window, cx| {
                                let _ = owner.update(cx, |picker, cx| {
                                    if hours {
                                        picker.hour = number;
                                    } else {
                                        picker.minute = number;
                                    }
                                    cx.notify();
                                });
                                popover.dismiss(window, cx);
                            })),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(title),
                    )
                    .child(grid)
            })
    }

    fn shortcuts(&self, cx: &mut Context<Self>) -> Div {
        let today = Local::now().date_naive();
        let english = &*gpui_component::locale() == "en";
        let actions = [
            (
                "schedule-today",
                if english { "Today" } else { "今天" },
                Some((today, 18, 30)),
            ),
            (
                "schedule-tomorrow",
                if english { "Tomorrow" } else { "明天" },
                Some((today + Duration::days(1), 18, 30)),
            ),
            (
                "schedule-week",
                if english { "This week" } else { "本周" },
                Some((model::week_start(today) + Duration::days(4), 18, 30)),
            ),
            (
                "schedule-clear",
                if english { "Clear" } else { "清空" },
                None,
            ),
        ];
        let mut row = div().flex().items_center().gap(px(4.));
        for (id, label, schedule) in actions {
            row = row.child(
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(22.))
                    .px(px(7.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().secondary))
                    .child(label)
                    .on_click(cx.listener(move |picker, _, window, cx| {
                        if let Some((date, hour, minute)) = schedule {
                            picker.date.update(cx, |state, cx| {
                                state.set_date(Date::Single(Some(date)), window, cx)
                            });
                            picker.hour = hour;
                            picker.minute = minute;
                        } else {
                            picker.date.update(cx, |state, cx| {
                                state.set_date(Date::Single(None), window, cx)
                            });
                            picker.hour = picker.default_hour;
                            picker.minute = 0;
                        }
                        cx.notify();
                    })),
            );
        }
        row
    }
}

impl Render for SchedulePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let placeholder = if &*gpui_component::locale() == "en" {
            "Select date"
        } else {
            "选择日期"
        };
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div().flex_1().min_w_0().child(
                            DatePicker::new(&self.date)
                                .number_of_months(1)
                                .cleanable(true)
                                .placeholder(placeholder)
                                .w_full(),
                        ),
                    )
                    .child(self.number_picker(true, cx))
                    .child(div().text_xs().child(":"))
                    .child(self.number_picker(false, cx)),
            )
            .child(self.shortcuts(cx))
    }
}
