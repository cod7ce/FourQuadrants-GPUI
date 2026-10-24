use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use gpui::{prelude::*, *};
use gpui_component::{
    Selectable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    popover::Popover,
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
};
use uuid::Uuid;

use super::*;
use crate::{
    model::{Document, ParseRule, ParsedTaskInput, Settings, Tag},
    theme::Palette,
};

const MAX_CAPTURE_GROUP: usize = 20;
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(200);

#[cfg(test)]
mod slider_tests {
    use super::normalize_slider_value;

    #[test]
    fn slider_values_snap_to_steps_and_stay_in_range() {
        for (value, limits, expected) in [
            (9., (10., 28., 1.), 10.),
            (10.49, (10., 28., 1.), 10.),
            (10.5, (10., 28., 1.), 11.),
            (29., (10., 28., 1.), 28.),
            (-2., (0., 100., 5.), 0.),
            (47.5, (0., 100., 5.), 50.),
            (101., (0., 100., 5.), 100.),
            (11., (0., 40., 2.), 12.),
            (41., (0., 40., 2.), 40.),
            (101., (0., 100., 5.), 100.),
            (1.6, (0., 20., 1.), 2.),
            (21., (0., 20., 1.), 20.),
        ] {
            let (min, max, step) = limits;
            let result = normalize_slider_value(value, min, max, step);
            assert_eq!(result, Some(expected));
            assert_eq!(normalize_slider_value(expected, min, max, step), result);
        }
    }

    #[test]
    fn slider_values_reject_non_finite_input() {
        for value in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            assert_eq!(normalize_slider_value(value, 0., 100., 5.), None);
        }
    }
}

fn normalize_slider_value(value: f32, min: f32, max: f32, step: f32) -> Option<f32> {
    value
        .is_finite()
        .then(|| (min + ((value.clamp(min, max) - min) / step).round() * step).clamp(min, max))
}

#[derive(Clone)]
struct SliderField {
    state: Entity<SliderState>,
    focus: FocusHandle,
    limits: (f32, f32, f32),
    accepted: Rc<Cell<f32>>,
}

impl SliderField {
    fn new(value: f32, limits: (f32, f32, f32), cx: &mut App) -> Self {
        let (min, max, step) = limits;
        Self {
            state: cx.new(|_| {
                SliderState::new()
                    .min(min)
                    .max(max)
                    .step(step)
                    .default_value(value)
            }),
            focus: cx.focus_handle().tab_stop(true),
            limits,
            accepted: Rc::new(Cell::new(value)),
        }
    }

    fn value(&self, cx: &App) -> f32 {
        self.state.read(cx).value().start()
    }

    fn set_value(&self, value: f32, window: &mut Window, cx: &mut App) {
        // Mark programmatic changes before updating the component, so syncing a
        // preset, shortcut or history entry cannot become a new saved mutation.
        self.accepted.set(value);
        if self.value(cx) != value {
            self.state
                .update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    fn accept_change(&self, value: f32, window: &mut Window, cx: &mut App) -> Option<f32> {
        if !value.is_finite() {
            self.set_value(self.accepted.get(), window, cx);
            return None;
        }
        if self.value(cx) != value || value == self.accepted.get() {
            return None; // A sync, duplicate, or superseded event needs no mutation.
        }
        let (min, max, step) = self.limits;
        let value = normalize_slider_value(value, min, max, step)?;
        let changed = value != self.accepted.get();
        self.set_value(value, window, cx);
        changed.then_some(value)
    }

    fn render(&self, label: &str, unit: &str, p: Palette, cx: &App) -> AnyElement {
        let keyboard = self.clone();
        let focus = self.focus.clone();
        let (min, max, _) = self.limits;
        let value = self.value(cx);
        div()
            .id(("settings-slider", self.state.entity_id()))
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(6.))
            .w(px(220.))
            .flex_shrink_0()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(transparent_black())
            .focus(|style| style.border_color(p.accent))
            // Capture also catches the thumb, which stops mouse-down bubbling.
            .capture_any_mouse_down(move |event, window, _| {
                if event.button == MouseButton::Left {
                    focus.focus(window);
                }
            })
            .on_key_down(move |event, window, cx| {
                let modifiers = event.keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
                    return;
                }
                let (min, max, step) = keyboard.limits;
                let current = keyboard.value(cx);
                let value = match event.keystroke.key.as_str() {
                    "left" | "down" => current - step,
                    "right" | "up" => current + step,
                    "home" => min,
                    "end" => max,
                    _ => return,
                };
                if let Some(value) = normalize_slider_value(value, min, max, step)
                    && value != current
                {
                    keyboard.state.update(cx, |state, cx| {
                        state.set_value(value, window, cx);
                        cx.emit(SliderEvent::Change(state.value()));
                    });
                }
                cx.stop_propagation();
                window.prevent_default();
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .child(div().flex_1().text_color(p.muted).child(label.to_owned()))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(if unit.is_empty() {
                                format!("{value:.0}")
                            } else {
                                format!("{value:.0} {unit}")
                            }),
                    ),
            )
            .child(Slider::new(&self.state).bg(p.accent).text_color(p.text))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(10.))
                    .text_color(p.muted)
                    .child(format!("{min}"))
                    .child(format!("{max}")),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum SettingsTab {
    #[default]
    Appearance,
    Background,
    Tags,
    Parsing,
    Data,
    Experiments,
    Updates,
}

impl SettingsTab {
    fn id(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Background => "background",
            Self::Tags => "tags",
            Self::Parsing => "parsing",
            Self::Data => "data",
            Self::Experiments => "experiments",
            Self::Updates => "updates",
        }
    }
}

pub struct SettingsEditor {
    active_tab: SettingsTab,
    font_family: Entity<InputState>,
    font_size: SliderField,
    accent: Entity<accent_picker::AccentPicker>,
    window_transparency: SliderField,
    window_blur: SliderField,
    hue: SliderField,
    saturation: SliderField,
    dark_lightness: SliderField,
    background_opacity: SliderField,
    background_blur: SliderField,
    background_scrim: SliderField,
    tag_name: Entity<InputState>,
    tag_color: Entity<InputState>,
    selected_tag: Option<Tag>,
    rule_name: Entity<InputState>,
    rule_pattern: Entity<InputState>,
    rule_issue: SliderField,
    rule_title: SliderField,
    rule_link: SliderField,
    rule_tag: String,
    selected_rule: Option<ParseRule>,
    preview: Entity<InputState>,
    preview_result: Option<ParsedTaskInput>,
    preview_error: Option<String>,
    preview_pending: bool,
    preview_signature: Option<String>,
    preview_version: u64,
    builtin_tag: String,
    last_settings: Settings,
    last_dark: bool,
    picking_background: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsEditor {
    pub fn new(doc: &Document, window: &mut Window, cx: &mut App) -> Self {
        let s = &doc.settings;
        let dark = Palette::is_dark(s, window.appearance());
        Self {
            active_tab: SettingsTab::default(),
            font_family: field(
                s.font_family.clone(),
                theme::tr(s, "系统默认", "System font"),
                false,
                window,
                cx,
            ),
            font_size: NumberSetting::FontSize.slider(s, dark, cx),
            accent: cx.new(|cx| {
                accent_picker::AccentPicker::new(&s.appearance_values().accent_hex, window, cx)
            }),
            window_transparency: NumberSetting::WindowTransparency.slider(s, dark, cx),
            window_blur: NumberSetting::WindowBlur.slider(s, dark, cx),
            hue: NumberSetting::Hue.slider(s, dark, cx),
            saturation: NumberSetting::Saturation.slider(s, dark, cx),
            dark_lightness: NumberSetting::DarkLightness.slider(s, dark, cx),
            background_opacity: NumberSetting::BackgroundOpacity.slider(s, dark, cx),
            background_blur: NumberSetting::BackgroundBlur.slider(s, dark, cx),
            background_scrim: NumberSetting::BackgroundScrim.slider(s, dark, cx),
            tag_name: field("", theme::tr(s, "标签名称", "Tag name"), false, window, cx),
            tag_color: field("#8B7DD8", "#RRGGBB", false, window, cx),
            selected_tag: None,
            rule_name: field("", theme::tr(s, "规则名称", "Rule name"), false, window, cx),
            rule_pattern: field(
                "",
                theme::tr(s, "正则表达式", "Regular expression"),
                true,
                window,
                cx,
            ),
            rule_issue: SliderField::new(0., (0., MAX_CAPTURE_GROUP as f32, 1.), cx),
            rule_title: SliderField::new(1., (0., MAX_CAPTURE_GROUP as f32, 1.), cx),
            rule_link: SliderField::new(0., (0., MAX_CAPTURE_GROUP as f32, 1.), cx),
            rule_tag: String::new(),
            selected_rule: None,
            preview: field(
                "ABC-123 修复登录 https://example.com/issue/123",
                theme::tr(s, "粘贴样例", "Paste a sample"),
                true,
                window,
                cx,
            ),
            preview_result: None,
            preview_error: None,
            preview_pending: false,
            preview_signature: None,
            preview_version: 0,
            builtin_tag: s.builtin_parse_tag_name.clone(),
            last_settings: s.clone(),
            last_dark: dark,
            picking_background: false,
            _subscriptions: Vec::new(),
        }
    }

    fn inputs(&self) -> Vec<Entity<InputState>> {
        vec![
            self.font_family.clone(),
            self.tag_name.clone(),
            self.tag_color.clone(),
            self.rule_name.clone(),
            self.rule_pattern.clone(),
            self.preview.clone(),
        ]
    }

    fn appearance_sliders(&self) -> [(NumberSetting, SliderField); 9] {
        [
            (NumberSetting::FontSize, self.font_size.clone()),
            (
                NumberSetting::WindowTransparency,
                self.window_transparency.clone(),
            ),
            (NumberSetting::WindowBlur, self.window_blur.clone()),
            (NumberSetting::Hue, self.hue.clone()),
            (NumberSetting::Saturation, self.saturation.clone()),
            (NumberSetting::DarkLightness, self.dark_lightness.clone()),
            (
                NumberSetting::BackgroundOpacity,
                self.background_opacity.clone(),
            ),
            (NumberSetting::BackgroundBlur, self.background_blur.clone()),
            (
                NumberSetting::BackgroundScrim,
                self.background_scrim.clone(),
            ),
        ]
    }

    fn sync(&mut self, doc: &Document, window: &mut Window, cx: &mut App) {
        let old = &self.last_settings;
        let new = &doc.settings;
        if old.language != new.language {
            for (input, zh, en) in [
                (&self.font_family, "系统默认", "System font"),
                (&self.tag_name, "标签名称", "Tag name"),
                (&self.rule_name, "规则名称", "Rule name"),
                (&self.rule_pattern, "正则表达式", "Regular expression"),
                (&self.preview, "粘贴样例", "Paste a sample"),
            ] {
                input.update(cx, |input, cx| {
                    input.set_placeholder(theme::tr(new, zh, en), window, cx);
                });
            }
        }
        if old.font_family != new.font_family {
            set_input(&self.font_family, new.font_family.clone(), window, cx);
        }
        if old.builtin_parse_tag_name != new.builtin_parse_tag_name {
            self.builtin_tag = new.builtin_parse_tag_name.clone();
        }
        let dark = Palette::is_dark(new, window.appearance());
        for (kind, slider) in self.appearance_sliders() {
            if kind.current(old, self.last_dark) != kind.current(new, dark) {
                slider.set_value(kind.current(new, dark), window, cx);
            }
        }
        self.accent.update(cx, |picker, cx| {
            picker.set_value(&new.appearance_values().accent_hex, window, cx);
        });
        self.last_settings = new.clone();
        self.last_dark = dark;
        if let Some(selected) = self.selected_tag.clone() {
            match doc.tags.iter().find(|tag| tag.id == selected.id).cloned() {
                Some(current) if current != selected => {
                    // Reordering changes sort_order. Only refresh fields that really
                    // changed so an unsaved name/color draft survives the reorder.
                    if current.name != selected.name {
                        set_input_if_changed(&self.tag_name, current.name.clone(), window, cx);
                    }
                    if current.color_hex != selected.color_hex {
                        set_input_if_changed(
                            &self.tag_color,
                            current.color_hex.clone(),
                            window,
                            cx,
                        );
                    }
                    self.selected_tag = Some(current);
                }
                Some(_) => {}
                None => self.load_tag(None, window, cx),
            }
        }
        if let Some(selected) = self.selected_rule.clone() {
            match new
                .parse_rules
                .iter()
                .find(|rule| rule.id == selected.id)
                .cloned()
            {
                Some(current) if current != selected => {
                    // Rule reordering does not touch fields; external edits only
                    // replace the fields that changed, preserving other drafts.
                    for (changed, input, value) in [
                        (
                            current.name != selected.name,
                            &self.rule_name,
                            current.name.clone(),
                        ),
                        (
                            current.pattern != selected.pattern,
                            &self.rule_pattern,
                            current.pattern.clone(),
                        ),
                    ] {
                        if changed {
                            set_input_if_changed(input, value, window, cx);
                        }
                    }
                    if current.tag_name != selected.tag_name {
                        self.rule_tag = current.tag_name.clone();
                    }
                    for (changed, slider, value) in [
                        (
                            current.issue_group != selected.issue_group,
                            &self.rule_issue,
                            current.issue_group,
                        ),
                        (
                            current.title_group != selected.title_group,
                            &self.rule_title,
                            current.title_group,
                        ),
                        (
                            current.link_group != selected.link_group,
                            &self.rule_link,
                            current.link_group,
                        ),
                    ] {
                        if changed {
                            slider.set_value(value as f32, window, cx);
                        }
                    }
                    self.selected_rule = Some(current);
                }
                Some(_) => {}
                None => self.load_rule(None, window, cx),
            }
        }
    }

    fn load_tag(&mut self, tag: Option<Tag>, window: &mut Window, cx: &mut App) {
        set_input(
            &self.tag_name,
            tag.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
            window,
            cx,
        );
        set_input(
            &self.tag_color,
            tag.as_ref()
                .map(|t| t.color_hex.clone())
                .unwrap_or_else(|| "#8B7DD8".into()),
            window,
            cx,
        );
        self.selected_tag = tag;
    }

    fn load_rule(&mut self, rule: Option<ParseRule>, window: &mut Window, cx: &mut App) {
        let draft = rule.clone().unwrap_or(ParseRule {
            title_group: 1,
            ..Default::default()
        });
        for (input, value) in [
            (&self.rule_name, draft.name),
            (&self.rule_pattern, draft.pattern),
        ] {
            set_input(input, value, window, cx);
        }
        for (slider, value) in [
            (&self.rule_issue, draft.issue_group),
            (&self.rule_title, draft.title_group),
            (&self.rule_link, draft.link_group),
        ] {
            slider.set_value(value as f32, window, cx);
        }
        self.rule_tag = draft.tag_name;
        self.selected_rule = rule;
    }

    fn rule_draft(&self, cx: &App, id: String) -> Result<ParseRule> {
        let pattern = input_text(&self.rule_pattern, cx);
        ensure!(
            !pattern.trim().is_empty(),
            "请输入正则表达式 / Enter a regular expression"
        );
        // Regex legality is decided by the Foundation helper. Here we only validate
        // fields needed by the rule model, so ICU-only lookbehind/backreferences survive.
        let group = |slider: &SliderField| -> Result<usize> {
            let value = slider.value(cx);
            ensure!(
                value.is_finite() && value >= 0. && value.fract() == 0.,
                "捕获组必须是非负整数 / Capture groups must be non-negative integers"
            );
            ensure!(
                value <= MAX_CAPTURE_GROUP as f32,
                "捕获组不能超过 {MAX_CAPTURE_GROUP} / Capture group must not exceed {MAX_CAPTURE_GROUP}"
            );
            Ok(value as usize)
        };
        Ok(ParseRule {
            id,
            name: input_text(&self.rule_name, cx).trim().into(),
            pattern,
            issue_group: group(&self.rule_issue)?,
            title_group: group(&self.rule_title)?,
            link_group: group(&self.rule_link)?,
            tag_name: self.rule_tag.clone(),
        })
    }
}

fn input_text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}
fn set_input(input: &Entity<InputState>, value: String, window: &mut Window, cx: &mut App) {
    input.update(cx, |state, cx| state.set_value(value, window, cx));
}
fn set_input_if_changed(
    input: &Entity<InputState>,
    value: String,
    window: &mut Window,
    cx: &mut App,
) {
    if input_text(input, cx) != value {
        set_input(input, value, window, cx);
    }
}
fn section(title: &str, description: &str, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .min_w_0()
        .p_6()
        .rounded(px(8.))
        .border_1()
        .border_color(p.border)
        .bg(p.panel)
        .text_size(px(13.))
        .text_color(p.text)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .pb_3()
                .border_b_1()
                .border_color(p.border)
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .text_color(p.muted)
                        .child(description.to_owned()),
                ),
        )
}
fn labeled(label: &str, input: &Entity<InputState>, p: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .flex_1()
        .min_w(px(120.))
        .child(div().text_xs().text_color(p.muted).child(label.to_owned()))
        .child(text_input(input))
}

const THEME_PREVIEW_HEIGHT: f32 = 82.;

fn system_theme_preview(dark: Palette, light: Palette) -> Div {
    div()
        .flex()
        .w_full()
        .h(px(THEME_PREVIEW_HEIGHT))
        .children([(dark, false), (light, true)].map(|(palette, right)| {
            div()
                .relative()
                .w(relative(0.5))
                .h_full()
                .overflow_hidden()
                .child(
                    theme_preview(palette)
                        .absolute()
                        .top_0()
                        .w(relative(2.))
                        .when(right, |el| el.right_0())
                        .when(!right, |el| el.left_0()),
                )
        }))
}

fn theme_preview(p: Palette) -> Div {
    let mut quadrants = div()
        .grid()
        .grid_cols(2)
        .grid_rows(2)
        .gap(px(4.))
        .flex_1()
        .h_full()
        .min_w_0();
    for color in p.quadrant {
        quadrants = quadrants.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(5.))
                .rounded(px(3.))
                .border_1()
                .border_color(p.border)
                .bg(p.panel)
                .child(div().w(px(12.)).h(px(3.)).rounded_full().bg(color))
                .child(div().w_full().h(px(5.)).rounded(px(2.)).bg(p.card))
                .child(
                    div()
                        .w(relative(0.65))
                        .h(px(3.))
                        .rounded_full()
                        .bg(p.muted.opacity(0.3)),
                ),
        );
    }
    div()
        .flex()
        .w_full()
        .h(px(THEME_PREVIEW_HEIGHT))
        .rounded(px(4.))
        .border_1()
        .border_color(p.border)
        .overflow_hidden()
        .bg(p.bg)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .w(px(27.))
                .h_full()
                .flex_shrink_0()
                .px(px(5.))
                .py(px(8.))
                .border_r_1()
                .border_color(p.border)
                .bg(p.panel)
                .child(div().size(px(7.)).rounded(px(2.)).bg(p.accent))
                .child(div().w_full().h(px(4.)).rounded(px(2.)).bg(p.selected))
                .child(
                    div()
                        .w_full()
                        .h(px(3.))
                        .rounded_full()
                        .bg(p.muted.opacity(0.3)),
                )
                .child(
                    div()
                        .w(relative(0.7))
                        .h(px(3.))
                        .rounded_full()
                        .bg(p.muted.opacity(0.3)),
                ),
        )
        .child(div().flex().flex_1().min_w_0().p(px(5.)).child(quadrants))
}
fn color_hex(value: &str) -> Result<u32> {
    let value = value.trim().trim_start_matches('#');
    ensure!(
        value.len() == 6 && value.bytes().all(|c| c.is_ascii_hexdigit()),
        "颜色格式为 #RRGGBB / Use #RRGGBB for colors"
    );
    Ok(u32::from_str_radix(value, 16)?)
}

#[derive(Clone, Copy)]
enum NumberSetting {
    FontSize,
    WindowTransparency,
    WindowBlur,
    Hue,
    Saturation,
    DarkLightness,
    BackgroundOpacity,
    BackgroundBlur,
    BackgroundScrim,
}
impl NumberSetting {
    fn unit(self) -> &'static str {
        match self {
            Self::FontSize | Self::WindowBlur | Self::BackgroundBlur => "px",
            Self::Hue => "°",
            Self::WindowTransparency
            | Self::Saturation
            | Self::DarkLightness
            | Self::BackgroundOpacity
            | Self::BackgroundScrim => "%",
        }
    }
    fn slider(self, settings: &Settings, dark: bool, cx: &mut App) -> SliderField {
        SliderField::new(self.current(settings, dark), self.limits(), cx)
    }
    fn limits(self) -> (f32, f32, f32) {
        match self {
            Self::FontSize => (10., 28., 1.),
            Self::WindowTransparency => (0., 100., 1.),
            Self::WindowBlur => (0., 64., 1.),
            Self::Hue => (0., 360., 1.),
            Self::Saturation => (0., 100., 1.),
            Self::DarkLightness => (0., 20., 1.),
            Self::BackgroundOpacity => (0., 100., 5.),
            Self::BackgroundBlur => (0., 40., 2.),
            Self::BackgroundScrim => (0., 100., 5.),
        }
    }
    fn current(self, s: &Settings, dark: bool) -> f32 {
        match self {
            Self::FontSize => s.font_size,
            Self::WindowTransparency => s.window_effects_values(dark).transparency * 100.,
            Self::WindowBlur => s.window_effects_values(dark).blur_radius,
            Self::Hue => s.appearance_values().hue,
            Self::Saturation => s.appearance_values().saturation * 100.,
            Self::DarkLightness => s.appearance_values().dark_lightness * 100.,
            Self::BackgroundOpacity => s.background_opacity * 100.,
            Self::BackgroundBlur => s.background_blur,
            Self::BackgroundScrim => s.background_scrim * 100.,
        }
    }
    fn write(self, s: &mut Settings, value: f32, dark: bool) {
        match self {
            Self::FontSize => s.font_size = value,
            Self::WindowTransparency => {
                s.customize_window_effects(dark).transparency = value / 100.
            }
            Self::WindowBlur => s.customize_window_effects(dark).blur_radius = value,
            Self::Hue => s.customize_appearance().hue = value,
            Self::Saturation => s.customize_appearance().saturation = value / 100.,
            Self::DarkLightness => s.customize_appearance().dark_lightness = value / 100.,
            Self::BackgroundOpacity => s.background_opacity = value / 100.,
            Self::BackgroundBlur => s.background_blur = value,
            Self::BackgroundScrim => s.background_scrim = value / 100.,
        }
    }
}

impl Workspace {
    fn settings_number(
        &self,
        kind: NumberSetting,
        label: &str,
        state: &SliderField,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        state.render(label, kind.unit(), p, cx)
    }

    fn subscribe_settings_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let subscription = cx.subscribe_in(
            &self.settings_editor.accent,
            window,
            |this, picker, event: &accent_picker::AccentChanged, window, cx| {
                if !this
                    .doc
                    .settings
                    .appearance_values()
                    .accent_hex
                    .eq_ignore_ascii_case(&event.0)
                {
                    this.change(cx, |doc| {
                        doc.settings.customize_appearance().accent_hex = event.0.clone();
                        Ok(())
                    });
                    this.apply_theme(window, cx);
                }
                picker.update(cx, |picker, cx| {
                    picker.set_value(
                        &this.doc.settings.appearance_values().accent_hex,
                        window,
                        cx,
                    )
                });
                cx.notify();
            },
        );
        self.settings_editor._subscriptions.push(subscription);
        for (kind, slider) in self.settings_editor.appearance_sliders() {
            let state = slider.state.clone();
            let subscription =
                cx.subscribe_in(&state, window, move |this, _, event, window, cx| {
                    let SliderEvent::Change(value) = event;
                    let Some(value) = slider.accept_change(value.start(), window, cx) else {
                        return;
                    };
                    let dark = Palette::is_dark(&this.doc.settings, window.appearance());
                    if (kind.current(&this.doc.settings, dark) - value).abs() > 0.0001 {
                        // Commit each distinct step through the existing atomic save path.
                        // Keeping no pending document changes makes close/history safe.
                        this.change(cx, |doc| {
                            kind.write(&mut doc.settings, value, dark);
                            Ok(())
                        });
                        this.apply_theme(window, cx);
                    }
                    // On save failure restore the committed value; keep change's error.
                    // Also absorb percentage conversion rounding before the next event.
                    slider.set_value(kind.current(&this.doc.settings, dark), window, cx);
                    cx.notify();
                });
            self.settings_editor._subscriptions.push(subscription);
        }
        for slider in [
            self.settings_editor.rule_issue.clone(),
            self.settings_editor.rule_title.clone(),
            self.settings_editor.rule_link.clone(),
        ] {
            let state = slider.state.clone();
            let subscription = cx.subscribe_in(&state, window, move |_, _, event, window, cx| {
                let SliderEvent::Change(value) = event;
                if slider.accept_change(value.start(), window, cx).is_some() {
                    // Rendering refreshes the parser preview without saving the draft.
                    cx.notify();
                }
            });
            self.settings_editor._subscriptions.push(subscription);
        }
    }

    fn choose_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_editor.picking_background {
            return;
        }
        self.settings_editor.picking_background = true;
        let directory = self
            .store
            .path
            .parent()
            .unwrap_or(Path::new("."))
            .join("Backgrounds");
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                self.t(
                    "选择背景图片（PNG、JPEG、WebP、GIF）",
                    "Choose a background image (PNG, JPEG, WebP, GIF)",
                )
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<PathBuf>> = async {
                let Some(path) = receiver.await??.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                cx.background_executor()
                    .spawn(async move {
                        let extension = path
                            .extension()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        ensure!(
                            matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif"),
                            "请选择 PNG、JPEG、WebP 或 GIF / Choose PNG, JPEG, WebP or GIF"
                        );
                        ensure!(
                            std::fs::metadata(&path)?.is_file(),
                            "所选路径不是文件 / Selection is not a file"
                        );
                        std::fs::create_dir_all(&directory)?;
                        let destination =
                            directory.join(format!("{}.{}", Uuid::new_v4(), extension));
                        std::fs::copy(&path, &destination)?;
                        Ok(Some(destination))
                    })
                    .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.settings_editor.picking_background = false;
                match result {
                    Ok(Some(path)) => {
                        this.change(cx, |doc| {
                            doc.settings.background_image =
                                Some(path.to_string_lossy().into_owned());
                            Ok(())
                        });
                        if this.error.is_some() {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.error = Some(format!("背景图片 / Background image: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_settings_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = input_text(&self.settings_editor.tag_name, cx)
            .trim()
            .to_owned();
        let color = input_text(&self.settings_editor.tag_color, cx);
        let selected = self.settings_editor.selected_tag.clone();
        let id = selected
            .as_ref()
            .map(|t| t.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        self.change(cx, |doc| {
            ensure!(!name.is_empty(), "标签名称不能为空 / Tag name is required");
            ensure!(
                !doc.tags.iter().any(|t| t.id != id && t.name == name),
                "标签名称已存在 / Tag name already exists"
            );
            let color = format!("#{:06X}", color_hex(&color)?);
            if let Some(selected) = selected {
                let tag = doc
                    .tags
                    .iter_mut()
                    .find(|t| t.id == id)
                    .ok_or_else(|| anyhow!("标签已被删除 / Tag was deleted"))?;
                tag.name = name.clone();
                tag.color_hex = color;
                for rule in &mut doc.settings.parse_rules {
                    if rule.tag_name == selected.name {
                        rule.tag_name = name.clone();
                    }
                }
                if doc.settings.builtin_parse_tag_name == selected.name {
                    doc.settings.builtin_parse_tag_name = name;
                }
            } else {
                let order = doc.tags.iter().map(|t| t.sort_order).fold(-1_f64, f64::max) + 1.;
                doc.tags.push(Tag {
                    id: id.clone(),
                    name,
                    color_hex: color,
                    sort_order: order,
                });
            }
            Ok(())
        });
        if self.error.is_none() {
            let tag = self.doc.tags.iter().find(|tag| tag.id == id).cloned();
            self.settings_editor.load_tag(tag, window, cx);
        }
    }

    fn reorder_settings_tag(&mut self, id: &str, direction: isize, cx: &mut Context<Self>) {
        self.change(cx, |doc| {
            doc.tags
                .sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order).then(a.id.cmp(&b.id)));
            if let Some(index) = doc.tags.iter().position(|tag| tag.id == id) {
                let next = index as isize + direction;
                if next >= 0 && next < doc.tags.len() as isize {
                    doc.tags.swap(index, next as usize);
                }
                for (order, tag) in doc.tags.iter_mut().enumerate() {
                    tag.sort_order = order as f64;
                }
            }
            Ok(())
        });
    }

    fn save_settings_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .settings_editor
            .selected_rule
            .as_ref()
            .map(|r| r.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let draft = self.settings_editor.rule_draft(cx, id);
        let mut saved_id = String::new();
        self.change(cx, |doc| {
            let rule = draft?;
            ensure!(
                !rule.name.is_empty(),
                "规则名称不能为空 / Rule name is required"
            );
            saved_id = rule.id.clone();
            if let Some(existing) = doc
                .settings
                .parse_rules
                .iter_mut()
                .find(|r| r.id == rule.id)
            {
                *existing = rule;
            } else {
                doc.settings.parse_rules.push(rule);
            }
            Ok(())
        });
        if self.error.is_none() {
            let rule = self
                .doc
                .settings
                .parse_rules
                .iter()
                .find(|rule| rule.id == saved_id)
                .cloned();
            self.settings_editor.load_rule(rule, window, cx);
        }
    }

    fn refresh_rule_preview(&mut self, s: &Settings, window: &mut Window, cx: &mut Context<Self>) {
        let sample = input_text(&self.settings_editor.preview, cx);
        let builtin = self.settings_editor.builtin_tag.clone();
        let pattern = input_text(&self.settings_editor.rule_pattern, cx);
        let selected_id = self
            .settings_editor
            .selected_rule
            .as_ref()
            .map(|r| r.id.clone());
        let editing = selected_id.is_some() || !pattern.trim().is_empty();
        let draft = if editing {
            Some(
                self.settings_editor.rule_draft(
                    cx,
                    selected_id
                        .clone()
                        .unwrap_or_else(|| "__settings_preview__".into()),
                ),
            )
        } else {
            None
        };
        let signature = format!(
            "{:?}\u{1f}{}\u{1f}{}\u{1f}{:?}\u{1f}{:?}\u{1f}{}",
            s.parse_rules, s.builtin_parse_tag_name, builtin, sample, draft, editing
        );
        if self.settings_editor.preview_signature.as_deref() == Some(signature.as_str()) {
            return;
        }

        self.settings_editor.preview_signature = Some(signature);
        self.settings_editor.preview_version = self.settings_editor.preview_version.wrapping_add(1);
        self.settings_editor.preview_result = None;
        self.settings_editor.preview_error = None;
        self.settings_editor.preview_pending = false;
        if sample.trim().is_empty() {
            return;
        }

        let mut preview_settings = s.clone();
        preview_settings.builtin_parse_tag_name = builtin;
        match draft {
            Some(Err(error)) => {
                self.settings_editor.preview_error = Some(error.to_string());
                return;
            }
            Some(Ok(rule)) => {
                if let Some(existing) = preview_settings
                    .parse_rules
                    .iter_mut()
                    .find(|r| r.id == rule.id)
                {
                    *existing = rule;
                } else {
                    preview_settings.parse_rules.push(rule);
                }
            }
            None => {}
        }

        let version = self.settings_editor.preview_version;
        self.settings_editor.preview_pending = true;
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
            let current = this
                .update(cx, |this, _| {
                    this.settings_editor.preview_version == version
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { crate::native::parse_input(&preview_settings, &sample) })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Inputs may have changed while the Foundation helper was running.
                if this.settings_editor.preview_version != version {
                    return;
                }
                this.settings_editor.preview_pending = false;
                match result {
                    Ok(parsed) => {
                        this.settings_editor.preview_result = Some(parsed);
                        this.settings_editor.preview_error = None;
                    }
                    Err(error) => {
                        this.settings_editor.preview_result = None;
                        this.settings_editor.preview_error = Some(format!("{error:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn settings_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.settings_editor._subscriptions.is_empty() {
            for input in self.settings_editor.inputs() {
                let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
                self.settings_editor._subscriptions.push(subscription);
            }
            self.subscribe_settings_sliders(window, cx);
        }
        self.settings_editor.sync(&self.doc, window, cx);
        let p = self.palette(window);
        let s = self.doc.settings.clone();
        self.refresh_rule_preview(&s, window, cx);
        let active_tab = self.settings_editor.active_tab;
        let mut tabs = div()
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .gap_1()
            .pb_3()
            .border_b_1()
            .border_color(p.border);
        for (tab, zh, en) in [
            (SettingsTab::Appearance, "外观", "Appearance"),
            (SettingsTab::Background, "背景", "Background"),
            (SettingsTab::Tags, "标签", "Tags"),
            (SettingsTab::Parsing, "解析", "Parsing"),
            (SettingsTab::Data, "数据", "Data"),
            (SettingsTab::Experiments, "实验", "Experiments"),
            (SettingsTab::Updates, "版本", "Version"),
        ] {
            let selected = active_tab == tab;
            tabs = tabs.child(
                chip(
                    SharedString::from(format!("settings-tab-{}", tab.id())),
                    self.t(zh, en).to_owned(),
                    p,
                )
                .text_size(px(13.))
                .bg(if selected {
                    p.selected
                } else {
                    transparent_black()
                })
                .text_color(if selected { p.accent } else { p.muted })
                .when(selected, |el| el.font_weight(FontWeight::MEDIUM))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings_editor.active_tab = tab;
                    cx.notify();
                })),
            );
        }
        let content = match active_tab {
            SettingsTab::Appearance => self.settings_appearance(&s, p, cx),
            SettingsTab::Background => self.settings_background(&s, p, cx),
            SettingsTab::Tags => self.settings_tags(p, cx),
            SettingsTab::Parsing => self.settings_rules(&s, p, cx),
            SettingsTab::Data => self.settings_storage(p, cx),
            SettingsTab::Experiments => self.settings_experiments(&s, p, cx),
            SettingsTab::Updates => self.settings_updates(p, cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .p_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .w_full()
                    .max_w(px(900.))
                    .mx_auto()
                    .child(tabs)
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "settings-scroll-{}",
                                active_tab.id()
                            )))
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .pb_1()
                            .child(content),
                    ),
            )
            .into_any_element()
    }

    fn settings_appearance(&self, s: &Settings, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut themes = div().flex().flex_wrap().gap_2();
        for (value, zh, en) in [
            ("system", "跟随系统", "System"),
            ("dark", "深色", "Dark"),
            ("light", "浅色", "Light"),
        ] {
            let selected = s.theme_mode() == value;
            let mut preview_settings = s.clone();
            preview_settings.customize_appearance();
            preview_settings.theme = value.into();
            preview_settings.glass = false;
            preview_settings.window_effects = None;
            let preview_palette = Palette::new(
                &preview_settings,
                if p.dark {
                    WindowAppearance::Dark
                } else {
                    WindowAppearance::Light
                },
            );
            let preview = if value == "system" {
                system_theme_preview(
                    Palette::new(&preview_settings, WindowAppearance::Dark),
                    Palette::new(&preview_settings, WindowAppearance::Light),
                )
            } else {
                theme_preview(preview_palette)
            };
            themes = themes.child(
                div()
                    .id(SharedString::from(format!("settings-theme-{value}")))
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .w(px(132.))
                    .gap_2()
                    .p(px(6.))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(if selected { p.accent } else { p.border })
                    .bg(if selected { p.selected } else { p.subtle })
                    .cursor_pointer()
                    .hover(move |el| el.bg(p.hover).border_color(p.accent))
                    .child(preview)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .px_1()
                            .text_size(px(12.))
                            .text_color(if selected { p.accent } else { p.text })
                            .child(div().size(px(7.)).rounded_full().bg(preview_palette.accent))
                            .child(div().flex_1().child(self.t(zh, en).to_owned()))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(16.))
                                    .when(selected, |el| el.child(glyph(IconName::Check))),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change(cx, |doc| {
                            doc.settings.customize_appearance();
                            doc.settings.theme = value.into();
                            Ok(())
                        });
                        this.apply_theme(window, cx);
                    })),
            );
        }
        let appearance = s.appearance_values();
        let mut accents = div().flex().flex_wrap().items_center().gap_2();
        let mut custom_accent = true;
        for (id, zh, en, hex) in [
            ("blue", "蓝", "Blue", "#6AA9EF"),
            ("violet", "紫", "Violet", "#9B8AFB"),
            ("pink", "粉", "Pink", "#DB8ABE"),
            ("coral", "珊瑚", "Coral", "#E08E8E"),
            ("amber", "琥珀", "Amber", "#DAAA63"),
            ("sage", "鼠尾草", "Sage", "#83B59C"),
            ("ocean", "海蓝", "Ocean", "#7DB8D9"),
            ("forest", "森林", "Forest", "#A3C1A3"),
            ("mint", "薄荷", "Mint", "#8FC9A0"),
        ] {
            let selected = appearance.accent_hex.eq_ignore_ascii_case(hex);
            custom_accent &= !selected;
            let mut swatch_settings = s.clone();
            swatch_settings.customize_appearance().accent_hex = hex.into();
            let swatch = Palette::new(
                &swatch_settings,
                if p.dark {
                    WindowAppearance::Dark
                } else {
                    WindowAppearance::Light
                },
            );
            let label = self.t(zh, en).to_owned();
            accents = accents.child(
                div()
                    .id(SharedString::from(format!("settings-accent-{id}")))
                    .size(px(32.))
                    .rounded_full()
                    .border_1()
                    .border_color(if selected {
                        p.text
                    } else {
                        transparent_black()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
                    .child(
                        div()
                            .size(px(22.))
                            .rounded_full()
                            .bg(swatch.accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(selected, |el| {
                                el.text_color(swatch.accent_text)
                                    .child(glyph(IconName::Check))
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change(cx, |doc| {
                            doc.settings.customize_appearance().accent_hex = hex.into();
                            Ok(())
                        });
                        this.apply_theme(window, cx);
                    })),
            );
        }
        let accent_picker = self.settings_editor.accent.clone();
        let reset_picker = accent_picker.clone();
        let custom_label = self.t("自定义", "Custom").to_owned();
        accents = accents.child(
            Popover::new("settings-accent-custom")
                .anchor(Corner::TopRight)
                .trigger(
                    Button::new("custom-accent-trigger")
                        .ghost()
                        .compact()
                        .h(px(32.))
                        .px(px(6.))
                        .selected(custom_accent)
                        .border_1()
                        .border_color(if custom_accent {
                            p.text
                        } else {
                            transparent_black()
                        })
                        .child(
                            div()
                                .relative()
                                .size(px(22.))
                                .rounded_full()
                                .overflow_hidden()
                                .when(custom_accent, |el| {
                                    el.bg(p.accent)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_color(p.accent_text)
                                        .child(glyph(IconName::Check))
                                })
                                .when(!custom_accent, |el| el.child(accent_picker::spectrum_dot())),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(custom_label),
                        ),
                )
                .on_open_change(move |_, window, cx| {
                    reset_picker.update(cx, |picker, cx| picker.reset_input(window, cx));
                })
                .content(move |_, _, _| accent_picker.clone()),
        );
        let mut languages = div().flex().flex_wrap().items_center().gap_2();
        for (code, label) in [("zh", "简体中文"), ("en", "English")] {
            languages = languages.child(
                chip(
                    SharedString::from(format!("settings-language-{code}")),
                    label,
                    p,
                )
                .border_1()
                .border_color(p.border)
                .bg(if s.language == code {
                    p.selected
                } else {
                    p.subtle
                })
                .text_color(if s.language == code { p.accent } else { p.text })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.change(cx, |doc| {
                        doc.settings.language = code.into();
                        Ok(())
                    });
                    this.apply_theme(window, cx);
                })),
            );
        }
        let mut fonts = div().flex().flex_wrap().gap_2();
        for (id, label, family) in [
            ("system", "系统 / System", ".SystemUIFont"),
            ("pingfang", "苹方", "PingFang SC"),
            ("mono", "Menlo", "Menlo"),
        ] {
            fonts = fonts.child(
                chip(SharedString::from(format!("settings-font-{id}")), label, p)
                    .border_1()
                    .border_color(p.border)
                    .bg(if s.font_family == family {
                        p.selected
                    } else {
                        p.subtle
                    })
                    .text_color(if s.font_family == family {
                        p.accent
                    } else {
                        p.text
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change(cx, |doc| {
                            doc.settings.customize_appearance();
                            doc.settings.font_family = family.into();
                            Ok(())
                        });
                        this.apply_theme(window, cx);
                    })),
            );
        }
        section(
            self.t("外观与偏好", "Appearance & preferences"),
            self.t(
                "让工作空间更适合你的节奏。",
                "Make the workspace feel like yours.",
            ),
            p,
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted)
                        .child(self.t("显示模式", "Appearance mode").to_owned()),
                )
                .child(themes),
        )
        .child(
            div().flex().flex_col().gap_2()
                .child(div().text_xs().text_color(p.muted)
                    .child(self.t("强调色", "Accent color").to_owned()))
                .child(accents),
        )
        .child(
            div().flex().flex_wrap().gap_2()
                .child(self.settings_number(NumberSetting::Hue,
                    self.t("界面色相", "Surface hue"), &self.settings_editor.hue, p, cx))
                .child(self.settings_number(NumberSetting::Saturation,
                    self.t("界面饱和度", "Surface saturation"), &self.settings_editor.saturation, p, cx))
                .child(self.settings_number(NumberSetting::DarkLightness,
                    self.t("深色模式亮度", "Dark mode lightness"), &self.settings_editor.dark_lightness, p, cx)),
        )
        .child(
            div().flex().flex_wrap().items_center().gap_3()
                .child(div().flex_1().min_w(px(180.)).text_xs().text_color(p.muted)
                    .child(self.t(
                        "色相与饱和度调整界面底色；亮度只影响深色模式。修改立即生效。",
                        "Hue and saturation tint the surfaces. Lightness applies to dark mode. Changes appear immediately.",
                    ).to_owned()))
                .child(chip("settings-reset-colors", self.t("重置色彩", "Reset colors").to_owned(), p)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.change(cx, |doc| {
                            *doc.settings.customize_appearance() = Default::default();
                            Ok(())
                        });
                        this.apply_theme(window, cx);
                    }))),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .pt_4()
                .border_t_1()
                .border_color(p.border)
                .gap_4()
                .items_center()
                .child(self.settings_number(
                    NumberSetting::WindowTransparency,
                    self.t("窗口透明度", "Window transparency"),
                    &self.settings_editor.window_transparency,
                    p,
                    cx,
                ))
                .child(self.settings_number(
                    NumberSetting::WindowBlur,
                    self.t("窗口模糊 · Blur", "Window blur"),
                    &self.settings_editor.window_blur,
                    p,
                    cx,
                )),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted)
                .child(self.t(
                    "透明度越高，越能看到窗口后方；0% 为实色。Blur 为 0 时清晰透视，调高后逐渐模糊。文字保持清晰，背景图片的模糊在「背景」中单独调整。",
                    "Higher transparency reveals more behind the window; 0% is solid. Blur at 0 keeps the view clear; higher values soften it. Text stays crisp. Image blur is separate under Background.",
                ).to_owned()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .pt_4()
                .border_t_1()
                .border_color(p.border)
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted)
                        .child(self.t("语言", "Language").to_owned()),
                )
                .child(languages),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .items_end()
                .child(labeled(
                    self.t("字体名称", "Font family"),
                    &self.settings_editor.font_family,
                    p,
                ))
                .child(
                    chip(
                        "settings-save-font",
                        self.t("应用字体", "Apply font").to_owned(),
                        p,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        let font = input_text(&this.settings_editor.font_family, cx)
                            .trim()
                            .to_owned();
                        this.change(cx, |doc| {
                            doc.settings.customize_appearance();
                            doc.settings.font_family = if font.is_empty() {
                                ".SystemUIFont".into()
                            } else {
                                font
                            };
                            Ok(())
                        });
                        this.apply_theme(window, cx);
                    })),
                )
                .child(self.settings_number(
                    NumberSetting::FontSize,
                    self.t("字号", "Font size"),
                    &self.settings_editor.font_size,
                    p,
                    cx,
                )),
        )
        .child(fonts)
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .pt_4()
                .border_t_1()
                .border_color(p.border)
                .child(
                    Switch::new("settings-show-inbox")
                        .small()
                        .checked(s.show_inbox)
                        .label(self.t("显示收集箱", "Show inbox").to_owned())
                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                            this.change(cx, |doc| {
                                doc.settings.show_inbox = *value;
                                Ok(())
                            })
                        })),
                )
                .child(
                    Switch::new("settings-hide-completed")
                        .small()
                        .checked(s.hide_completed)
                        .label(self.t("隐藏已完成任务", "Hide completed tasks").to_owned())
                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                            this.change(cx, |doc| {
                                doc.settings.hide_completed = *value;
                                Ok(())
                            })
                        })),
                ),
        )
        .into_any_element()
    }

    fn settings_background(&self, s: &Settings, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let filename = s
            .background_image
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.t("尚未设置背景图片", "No background image").into());
        let mut fills = div().flex().flex_wrap().gap_2();
        for (value, zh, en) in [
            ("fill", "填充", "Fill"),
            ("fit", "适应", "Fit"),
            ("tile", "平铺", "Tile"),
            ("stretch", "拉伸", "Stretch"),
        ] {
            fills = fills.child(
                chip(
                    SharedString::from(format!("settings-background-fill-{value}")),
                    self.t(zh, en).to_owned(),
                    p,
                )
                .border_1()
                .border_color(p.border)
                .bg(if s.background_fill == value {
                    p.selected
                } else {
                    p.subtle
                })
                .text_color(if s.background_fill == value {
                    p.accent
                } else {
                    p.text
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change(cx, |doc| {
                        doc.settings.background_fill = value.into();
                        Ok(())
                    })
                })),
            );
        }
        section(
            self.t("背景图片", "Background image"),
            self.t(
                "图片会复制到应用数据目录，原始文件可以移动。",
                "Images are copied into app storage, so the original can move.",
            ),
            p,
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(px(6.))
                .border_1()
                .border_color(p.border)
                .bg(p.subtle)
                .text_size(px(12.))
                .text_color(p.muted)
                .child(glyph(IconName::File))
                .child(div().flex_1().min_w_0().truncate().child(filename)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    chip(
                        "settings-choose-background",
                        if self.settings_editor.picking_background {
                            self.t("正在选择…", "Choosing…")
                        } else {
                            self.t("选择图片", "Choose image")
                        }
                        .to_owned(),
                        p,
                    )
                    .on_click(
                        cx.listener(|this, _, window, cx| this.choose_background(window, cx)),
                    ),
                )
                .when(s.background_image.is_some(), |el| {
                    el.child(
                        chip(
                            "settings-remove-background",
                            self.t("移除图片", "Remove image").to_owned(),
                            p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.change(cx, |doc| {
                                doc.settings.background_image = None;
                                Ok(())
                            })
                        })),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_4()
                .child(self.settings_number(
                    NumberSetting::BackgroundOpacity,
                    self.t("图片不透明度", "Image opacity"),
                    &self.settings_editor.background_opacity,
                    p,
                    cx,
                ))
                .child(self.settings_number(
                    NumberSetting::BackgroundBlur,
                    self.t("图片模糊半径", "Image blur radius"),
                    &self.settings_editor.background_blur,
                    p,
                    cx,
                ))
                .child(self.settings_number(
                    NumberSetting::BackgroundScrim,
                    self.t("遮罩强度", "Scrim strength"),
                    &self.settings_editor.background_scrim,
                    p,
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted)
                        .child(self.t("显示方式", "Image layout").to_owned()),
                )
                .child(fills),
        )
        .into_any_element()
    }

    fn settings_tags(&self, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut tags = self.doc.tags.clone();
        tags.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order).then(a.id.cmp(&b.id)));
        let mut list = div().flex().flex_col().gap_2();
        for tag in tags {
            let id = tag.id.clone();
            let up = id.clone();
            let down = id.clone();
            let remove = id.clone();
            let selected = self
                .settings_editor
                .selected_tag
                .as_ref()
                .is_some_and(|t| t.id == id);
            let color = color_hex(&tag.color_hex)
                .map(rgb)
                .map(Hsla::from)
                .unwrap_or(p.accent);
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.border)
                    .bg(if selected { p.selected } else { p.subtle })
                    .child(div().size_3().rounded_full().bg(color))
                    .child(
                        chip(
                            SharedString::from(format!("settings-tag-edit-{id}")),
                            tag.name.clone(),
                            p,
                        )
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .justify_start()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                let tag = this.doc.tags.iter().find(|t| t.id == id).cloned();
                                this.settings_editor.load_tag(tag, window, cx);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        icon_button(
                            SharedString::from(format!("settings-tag-up-{up}")),
                            IconName::ArrowUp,
                            self.t("上移", "Move up"),
                            p,
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.reorder_settings_tag(&up, -1, cx)
                            }),
                        ),
                    )
                    .child(
                        icon_button(
                            SharedString::from(format!("settings-tag-down-{down}")),
                            IconName::ArrowDown,
                            self.t("下移", "Move down"),
                            p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reorder_settings_tag(&down, 1, cx)
                        })),
                    )
                    .child(
                        icon_button(
                            SharedString::from(format!("settings-tag-delete-{remove}")),
                            IconName::Delete,
                            self.t("删除标签", "Delete tag"),
                            p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.change(cx, |doc| {
                                doc.tags.retain(|t| t.id != remove);
                                for task in &mut doc.tasks {
                                    task.tags.retain(|id| id != &remove);
                                }
                                Ok(())
                            });
                            if this.error.is_none() && this.tag_filter.as_ref() == Some(&remove) {
                                this.tag_filter = None;
                            }
                        })),
                    ),
            );
        }
        section(self.t("标签", "Tags"), self.t("点击标签编辑，用上下箭头调整侧栏顺序。删除标签不会删除任务。", "Select a tag to edit; arrows change sidebar order. Deleting a tag keeps its tasks."), p)
            .child(list)
            .child(div().flex().flex_wrap().items_end().gap_3()
                .child(labeled(self.t("名称", "Name"), &self.settings_editor.tag_name, p))
                .child(div().w(px(145.)).child(labeled(self.t("颜色", "Color"), &self.settings_editor.tag_color, p)))
                .child(chip("settings-tag-save", if self.settings_editor.selected_tag.is_some() { self.t("保存标签", "Save tag") } else { self.t("添加标签", "Add tag") }.to_owned(), p)
                    .bg(p.selected).on_click(cx.listener(|this, _, window, cx| this.save_settings_tag(window, cx))))
                .child(chip("settings-tag-new", self.t("新建", "New").to_owned(), p)
                    .on_click(cx.listener(|this, _, window, cx| { this.settings_editor.load_tag(None, window, cx); cx.notify(); }))))
            .into_any_element()
    }

    fn rule_tag_selector(&self, builtin: bool, p: Palette, cx: &mut Context<Self>) -> Div {
        let selected = if builtin {
            &self.settings_editor.builtin_tag
        } else {
            &self.settings_editor.rule_tag
        };
        let no_tag = self.t("无标签", "No tag").to_owned();
        let label = if selected.is_empty() {
            no_tag.clone()
        } else {
            selected.clone()
        };
        let id = if builtin {
            "builtin-rule-tag"
        } else {
            "custom-rule-tag"
        };
        let mut tags = self.doc.tags.iter().collect::<Vec<_>>();
        tags.sort_by(|a, b| a.sort_order.total_cmp(&b.sort_order));
        let mut choices = vec![(String::new(), no_tag)];
        choices.extend(
            tags.into_iter()
                .map(|tag| (tag.name.clone(), tag.name.clone())),
        );
        // Keep imported rule tags visible until the user explicitly replaces them.
        if !selected.is_empty() && !choices.iter().any(|(name, _)| name == selected) {
            choices.push((selected.clone(), selected.clone()));
        }
        let selected = selected.clone();
        let workspace = cx.entity().downgrade();
        div()
            .flex_1()
            .min_w(px(160.))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted)
                    .child(self.t("匹配后标签", "Tag after match")),
            )
            .child(
                Popover::new(id)
                    .trigger(
                        Button::new(id)
                            .outline()
                            .w_full()
                            .h(px(32.))
                            .justify_start()
                            .label(label)
                            .icon(IconName::ChevronDown),
                    )
                    .content(move |_, _, cx| {
                        let mut list = div()
                            .id("rule-tag-options")
                            .w(px(240.))
                            .max_h(px(280.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_1();
                        for (index, (name, label)) in choices.iter().enumerate() {
                            let workspace = workspace.clone();
                            let name = name.clone();
                            list = list.child(
                                Button::new(("rule-tag-option", index))
                                    .ghost()
                                    .w_full()
                                    .justify_start()
                                    .label(label.clone())
                                    .selected(name == selected)
                                    .on_click(cx.listener(move |popover, _, window, cx| {
                                        let _ = workspace.update(cx, |this, cx| {
                                            if builtin {
                                                this.settings_editor.builtin_tag = name.clone();
                                            } else {
                                                this.settings_editor.rule_tag = name.clone();
                                            }
                                            cx.notify();
                                        });
                                        popover.dismiss(window, cx);
                                    })),
                            );
                        }
                        list
                    }),
            )
    }

    fn settings_rules(&self, s: &Settings, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div().flex().flex_col().gap_2();
        for (index, rule) in s.parse_rules.iter().enumerate() {
            let id = rule.id.clone();
            let remove = id.clone();
            let up = id.clone();
            let down = id.clone();
            let selected = self
                .settings_editor
                .selected_rule
                .as_ref()
                .is_some_and(|r| r.id == id);
            let mut row = div()
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(p.border)
                .bg(if selected { p.selected } else { p.subtle })
                .child(
                    chip(
                        SharedString::from(format!("settings-rule-edit-{id}")),
                        format!("{}. {}", index + 1, rule.name),
                        p,
                    )
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .justify_start()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let rule = this
                            .doc
                            .settings
                            .parse_rules
                            .iter()
                            .find(|r| r.id == id)
                            .cloned();
                        this.settings_editor.load_rule(rule, window, cx);
                        cx.notify();
                    })),
                );
            for (action, rule_id, direction) in [("up", up, -1_isize), ("down", down, 1)] {
                row = row.child(
                    icon_button(
                        SharedString::from(format!("settings-rule-{action}-{rule_id}")),
                        if direction < 0 {
                            IconName::ArrowUp
                        } else {
                            IconName::ArrowDown
                        },
                        if direction < 0 {
                            self.t("上移", "Move up")
                        } else {
                            self.t("下移", "Move down")
                        },
                        p,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |doc| {
                            let rules = &mut doc.settings.parse_rules;
                            if let Some(index) = rules.iter().position(|r| r.id == rule_id) {
                                let next = index as isize + direction;
                                if next >= 0 && next < rules.len() as isize {
                                    rules.swap(index, next as usize);
                                }
                            }
                            Ok(())
                        })
                    })),
                );
            }
            list = list.child(
                row.child(
                    icon_button(
                        SharedString::from(format!("settings-rule-delete-{remove}")),
                        IconName::Delete,
                        self.t("删除规则", "Delete rule"),
                        p,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(cx, |doc| {
                            doc.settings.parse_rules.retain(|r| r.id != remove);
                            Ok(())
                        })
                    })),
                ),
            );
        }
        let preview = if input_text(&self.settings_editor.preview, cx)
            .trim()
            .is_empty()
        {
            self.t(
                "输入样例后将使用 Foundation 自动预览。",
                "Enter a sample to preview it with Foundation.",
            )
            .to_owned()
        } else if self.settings_editor.preview_pending {
            self.t("正在用 Foundation 解析…", "Parsing with Foundation…")
                .to_owned()
        } else if let Some(error) = &self.settings_editor.preview_error {
            format!("⚠ {error}")
        } else if let Some(parsed) = &self.settings_editor.preview_result {
            format!(
                "{}: {}\n{}: {}\n{}: {}\n{}: {}",
                self.t("标题", "Title"),
                parsed.title,
                self.t("工单", "Issue"),
                parsed.issue_key.as_deref().unwrap_or("—"),
                self.t("链接", "Links"),
                if parsed.links.is_empty() {
                    "—".to_owned()
                } else {
                    parsed.links.join(" · ")
                },
                self.t("标签", "Tag"),
                parsed.tag_name.as_deref().unwrap_or("—")
            )
        } else {
            self.t("等待解析结果…", "Waiting for a parsing result…")
                .to_owned()
        };
        section(self.t("粘贴解析规则", "Paste parsing rules"), self.t("按顺序使用首个有结果的规则，否则使用内置解析。捕获组从 1 开始，0 表示忽略。预览由 Foundation 执行，无法编译的规则会被跳过。", "The first rule with a result wins; otherwise built-in parsing applies. Groups start at 1; 0 skips a field. Preview runs in Foundation; rules it cannot compile are skipped."), p)
            .child(div().flex().flex_col().gap_3().p_4().rounded(px(6.)).border_1().border_color(p.border).bg(p.subtle)
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(self.t("默认匹配规则", "Default matching rule")))
                .child(div().text_xs().text_color(p.muted).child(self.t("未命中自定义规则时使用，自动识别标题、工单和链接。选择“无标签”则不添加标签。", "Used when no custom rule matches; detects the title, issue and links. Choose “No tag” to add no tag.")))
                .child(div().flex().flex_wrap().items_end().gap_3()
                    .child(self.rule_tag_selector(true, p, cx))
                    .child(chip("settings-builtin-tag-save", self.t("保存默认标签", "Save default tag").to_owned(), p)
                        .bg(p.selected)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let name = this.settings_editor.builtin_tag.clone();
                            this.change(cx, |doc| { doc.settings.builtin_parse_tag_name = name; Ok(()) });
                        })))))
            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(self.t("自定义规则", "Custom rules")))
            .child(list)
            .child(div().flex().flex_wrap().gap_3()
                .child(labeled(self.t("规则名称", "Rule name"), &self.settings_editor.rule_name, p))
                .child(self.rule_tag_selector(false, p, cx)))
            .child(div().flex().flex_col().gap_2()
                .child(div().text_xs().text_color(p.muted).child(self.t("正则表达式", "Regular expression").to_owned()))
                .child(Input::new(&self.settings_editor.rule_pattern).small().h(px(84.))))
            .child(div().flex().flex_wrap().gap_3()
                .child(self.settings_editor.rule_issue.render(self.t("工单捕获组", "Issue group"), "", p, cx))
                .child(self.settings_editor.rule_title.render(self.t("标题捕获组", "Title group"), "", p, cx))
                .child(self.settings_editor.rule_link.render(self.t("链接捕获组", "Link group"), "", p, cx)))
            .child(div().flex().flex_wrap().gap_2()
                .child(chip("settings-rule-save", if self.settings_editor.selected_rule.is_some() { self.t("保存规则", "Save rule") } else { self.t("添加规则", "Add rule") }.to_owned(), p).bg(p.selected)
                    .on_click(cx.listener(|this, _, window, cx| this.save_settings_rule(window, cx))))
                .child(chip("settings-rule-new", self.t("新建", "New").to_owned(), p)
                    .on_click(cx.listener(|this, _, window, cx| { this.settings_editor.load_rule(None, window, cx); cx.notify(); }))))
            .child(div().border_t_1().border_color(p.border).pt_4().flex().flex_col().gap_3()
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(self.t("实时预览（包含未保存的草稿）", "Live preview (includes unsaved draft)").to_owned()))
                .child(Input::new(&self.settings_editor.preview).small().h(px(85.)))
                .child(div().p_3().rounded(px(6.)).border_1().border_color(p.border).bg(p.subtle).text_size(px(12.)).line_height(px(18.)).text_color(p.muted).child(preview)))
            .into_any_element()
    }

    fn settings_experiments(&self, s: &Settings, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        section(
            self.t("实验", "Experiments"),
            self.t("试用实验功能，按喜好开启或关闭。", "Try experimental features and choose which to enable."),
            p,
        )
        .child(
            div().flex().flex_col().gap_2()
                .child(
                    Switch::new("settings-task-confetti")
                        .small()
                        .checked(s.task_confetti)
                        .label(self.t("撒花特效", "Confetti effect").to_owned())
                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                            this.change(cx, |doc| {
                                doc.settings.task_confetti = *value;
                                Ok(())
                            });
                        })),
                )
                .child(div().text_size(px(12.)).text_color(p.muted).child(self.t(
                    "完成任务时播放撒花动画，遵循系统的“减少动态效果”设置。",
                    "Celebrate completed tasks with confetti. Respects Reduce Motion in system settings.",
                ).to_owned())),
        )
        .child(
            div().flex().flex_col().gap_2().pt_4().border_t_1().border_color(p.border)
                .child(
                    Switch::new("settings-task-completion-sound")
                        .small()
                        .checked(s.task_completion_sound)
                        .label(self.t("完成音效", "Completion sound").to_owned())
                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                            this.change(cx, |doc| {
                                doc.settings.task_completion_sound = *value;
                                Ok(())
                            });
                        })),
                )
                .child(div().text_size(px(12.)).text_color(p.muted).child(self.t(
                    "完成任务时播放简短的双音提示。",
                    "Play a short two-note sound when a task is completed.",
                ).to_owned())),
        )
        .into_any_element()
    }

    fn settings_updates(&self, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        use crate::native::{RELEASES_URL, UpdateState};
        let update = &self.update_status;
        let (zh, en) = match update.state {
            UpdateState::Ready => ("随时检查新版本", "Ready to check for updates"),
            UpdateState::Checking => ("正在检查更新…", "Checking for updates…"),
            UpdateState::Available => ("发现新版本", "An update is available"),
            UpdateState::Current => ("当前已是最新版本", "You’re up to date"),
            UpdateState::InstallPending => ("正在保存并准备重启…", "Saving before restarting…"),
            UpdateState::Disabled => (
                "测试数据模式下已停用更新",
                "Updates are disabled in test data mode",
            ),
            UpdateState::Unavailable => (
                "当前运行方式不支持应用内更新",
                "In-app updates are unavailable here",
            ),
            UpdateState::Error => ("更新尚未完成", "The update could not complete"),
        };
        let detail = match update.state {
            UpdateState::Disabled => self.t(
                "此窗口使用独立测试数据。请在正常启动的应用中检查和安装更新。",
                "This window uses a separate test store. Check and install updates in the normally launched application.",
            ).to_owned(),
            UpdateState::Unavailable => format!("{}\n{}", self.t(
                "当前构建无法启用更新。请从发布记录获取配置完整的安装包。",
                "Updates could not start in this build. Get a configured package from the releases page.",
            ), update.message),
            UpdateState::Error => update.message.clone(),
            UpdateState::Available => update.version.as_ref().map(|version| format!("{} {version}", self.t("可用版本", "Available version"))).unwrap_or_default(),
            _ => String::new(),
        };
        section(
            self.t("版本", "Version"),
            self.t("获取最新版本，查看此应用的版本信息。", "Get the latest release and view app version information."),
            p,
        )
        .child(div().flex().items_center().gap_4().py_2()
            .child(img(ImageSource::Resource(Resource::Embedded("app-icon.png".into()))).size(px(64.)).flex_shrink_0().object_fit(ObjectFit::Contain))
            .child(div().flex().flex_col().gap_1()
                .child(div().flex().flex_wrap().items_center().gap_2()
                    .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(self.t("肆", "Four Quadrants")))
                    .child(div().px_2().py(px(2.)).rounded(px(4.)).border_1().border_color(p.border).bg(p.subtle).text_size(px(10.)).text_color(p.muted).child("GPUI")))
                .child(div().text_size(px(13.)).text_color(p.muted).child(format!("{} {}", self.t("版本", "Version"), env!("CARGO_PKG_VERSION"))))))
        .child(div().flex().flex_col().gap_2().p_4().rounded(px(6.)).border_1().border_color(p.border).bg(p.subtle)
            .child(div().font_weight(FontWeight::MEDIUM).child(self.t(zh, en).to_owned()))
            .when(!detail.is_empty(), |el| el.child(div().text_size(px(12.)).text_color(p.muted).child(detail)))
            .child(div().flex().flex_wrap().gap_2().pt_2()
                .child(chip("settings-check-updates", self.t("检查更新", "Check for updates").to_owned(), p)
                    .when(!update.can_check, |el| el.opacity(0.45).cursor_default())
                    .on_click(cx.listener(|this, _, window, cx| this.check_for_updates(window, cx))))
                .child(chip("settings-update-releases", self.t("发布记录与下载", "Releases & downloads").to_owned(), p)
                    .on_click(|_, _, cx| cx.open_url(RELEASES_URL)))))
        .child(div().text_size(px(12.)).line_height(px(19.)).text_color(p.muted).child(self.t(
            "通过 Sparkle 检查更新并验证更新包签名。安装前会先保存任务和笔记；自动检查与后台下载默认关闭。",
            "Sparkle checks for updates and verifies package signatures. Tasks and notes are saved before installation. Automatic checks and background downloads are off by default.",
        ).to_owned()))
        .into_any_element()
    }

    fn settings_storage(&self, p: Palette, cx: &mut Context<Self>) -> AnyElement {
        let directory = self
            .store
            .path
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        section(self.t("数据与系统集成", "Data & integrations"), self.t("JSON 导入合并任务和标签，保留本机偏好。导出可作为备份。", "JSON import merges tasks and tags while keeping local preferences. Export a backup whenever you need."), p)
            .child(div().flex().flex_wrap().gap_2()
                .child(chip("settings-json-import", self.t("导入 JSON", "Import JSON").to_owned(), p)
                    .on_click(cx.listener(|this, _, window, cx| this.import_file(false, window, cx))))
                .child(chip("settings-json-export", self.t("导出 JSON", "Export JSON").to_owned(), p)
                    .on_click(cx.listener(|this, _, window, cx| this.export_file(false, window, cx))))
                .child(chip("settings-legacy-import", self.t("导入旧版 .store", "Import legacy .store").to_owned(), p)
                    .on_click(cx.listener(|this, _, window, cx| this.import_file(true, window, cx))))
                .child(chip("settings-sync-reminders", self.t("同步系统通知", "Sync notifications").to_owned(), p)
                    .on_click(cx.listener(|this, _, window, cx| this.sync_reminders(window, cx)))))
            .child(div().flex().items_start().gap_2().p_3().rounded(px(6.)).border_1().border_color(p.border).bg(p.subtle).text_size(px(11.)).text_color(p.muted)
                .child(glyph(IconName::Folder))
                .child(div().flex_1().min_w_0().child(directory.to_string_lossy().into_owned())))
            .child(chip("settings-reveal-storage", self.t("在 Finder 中显示数据目录", "Reveal data folder in Finder").to_owned(), p)
                .on_click(move |_, _, cx| cx.reveal_path(&directory)))
            .into_any_element()
    }
}
