use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable,
    input::{Input, InputEvent, InputState},
};

pub(super) struct AccentChanged(pub String);

#[derive(Clone, Copy)]
enum Channel {
    Color,
    Hue,
}

#[derive(Clone)]
struct ColorDrag(EntityId, Channel);

impl Render for ColorDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone, Copy)]
struct Hsv {
    hue: f32,
    saturation: f32,
    value: f32,
}

impl Hsv {
    fn from_hex(hex: &str) -> Self {
        let value = u32::from_str_radix(&hex[1..], 16).expect("validated accent hex");
        let color = rgb(value);
        let max = color.r.max(color.g).max(color.b);
        let min = color.r.min(color.g).min(color.b);
        let delta = max - min;
        let hue = if delta == 0. {
            0.
        } else if max == color.r {
            ((color.g - color.b) / delta).rem_euclid(6.) / 6.
        } else if max == color.g {
            ((color.b - color.r) / delta + 2.) / 6.
        } else {
            ((color.r - color.g) / delta + 4.) / 6.
        };
        Self {
            hue,
            saturation: if max == 0. { 0. } else { delta / max },
            value: max,
        }
    }

    fn color(self) -> Hsla {
        let lightness = self.value * (1. - self.saturation / 2.);
        let saturation = if lightness == 0. || lightness == 1. {
            0.
        } else {
            (self.value - lightness) / lightness.min(1. - lightness)
        };
        hsla(self.hue, saturation, lightness, 1.)
    }

    fn hex(self) -> String {
        let color = Rgba::from(self.color());
        let byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
        format!(
            "#{:02X}{:02X}{:02X}",
            byte(color.r),
            byte(color.g),
            byte(color.b)
        )
    }
}

fn normalize_hex(value: &str) -> Option<String> {
    let value = value.trim();
    let digits = value.strip_prefix('#').unwrap_or(value);
    (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", digits.to_ascii_uppercase()))
}

pub(super) struct AccentPicker {
    hsv: Hsv,
    hex: String,
    input: Entity<InputState>,
    color_bounds: Bounds<Pixels>,
    hue_bounds: Bounds<Pixels>,
    color_focus: FocusHandle,
    hue_focus: FocusHandle,
    invalid: bool,
    _subscription: Subscription,
}

impl AccentPicker {
    pub(super) fn new(hex: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hex = normalize_hex(hex).expect("validated accent hex");
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(hex.clone())
                .placeholder("#RRGGBB")
        });
        let subscription = cx.subscribe_in(&input, window, |this, input, event, _, cx| {
            if matches!(event, InputEvent::Change | InputEvent::PressEnter { .. }) {
                let value = input.read(cx).value();
                if let Some(hex) = normalize_hex(&value) {
                    this.invalid = false;
                    if this.hex != hex {
                        this.hsv = Hsv::from_hex(&hex);
                        this.hex = hex.clone();
                        cx.emit(AccentChanged(hex));
                    }
                } else {
                    this.invalid = matches!(event, InputEvent::PressEnter { .. });
                }
                cx.notify();
            }
        });
        Self {
            hsv: Hsv::from_hex(&hex),
            hex,
            input,
            color_bounds: Bounds::default(),
            hue_bounds: Bounds::default(),
            color_focus: cx.focus_handle().tab_stop(true),
            hue_focus: cx.focus_handle().tab_stop(true),
            invalid: false,
            _subscription: subscription,
        }
    }

    pub(super) fn set_value(&mut self, hex: &str, window: &mut Window, cx: &mut Context<Self>) {
        let hex = normalize_hex(hex).expect("validated accent hex");
        if self.hex != hex {
            self.hsv = Hsv::from_hex(&hex);
            self.hex = hex;
            self.reset_input(window, cx);
        }
    }

    pub(super) fn reset_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalid = false;
        self.input.update(cx, |input, cx| {
            input.set_value(self.hex.clone(), window, cx)
        });
        cx.notify();
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = self.hsv.hex();
        if self.hex != hex {
            self.hex = hex.clone();
            self.reset_input(window, cx);
            cx.emit(AccentChanged(hex));
        }
        cx.notify();
    }

    fn pick(
        &mut self,
        channel: Channel,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = match channel {
            Channel::Color => self.color_bounds,
            Channel::Hue => self.hue_bounds,
        };
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return;
        }
        let x = ((point.x - bounds.left()) / bounds.size.width).clamp(0., 1.);
        match channel {
            Channel::Color => {
                self.hsv.saturation = x;
                self.hsv.value = 1. - ((point.y - bounds.top()) / bounds.size.height).clamp(0., 1.);
            }
            Channel::Hue => self.hsv.hue = x,
        }
        self.commit(window, cx);
    }

    fn surface(&self, channel: Channel, cx: &mut Context<Self>) -> Stateful<Div> {
        let focus = match channel {
            Channel::Color => self.color_focus.clone(),
            Channel::Hue => self.hue_focus.clone(),
        };
        let id = cx.entity_id();
        let ring = cx.theme().ring;
        div()
            .id(match channel {
                Channel::Color => "accent-color-plane",
                Channel::Hue => "accent-hue",
            })
            .track_focus(&focus)
            .relative()
            .w_full()
            .cursor_crosshair()
            .rounded(px(5.))
            .focus(move |style| {
                style.shadow(vec![BoxShadow {
                    color: ring,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(2.),
                }])
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    focus.focus(window);
                    this.pick(channel, event.position, window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_drag(ColorDrag(id, channel), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<ColorDrag>, window, cx| {
                    let drag = event.drag(cx);
                    if drag.0 == id {
                        this.pick(drag.1, event.event.position, window, cx);
                    }
                }),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.platform
                    || event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                {
                    return;
                }
                let step = if event.keystroke.modifiers.shift {
                    0.1
                } else {
                    0.01
                };
                match (channel, event.keystroke.key.as_str()) {
                    (Channel::Color, "left") => {
                        this.hsv.saturation = (this.hsv.saturation - step).max(0.)
                    }
                    (Channel::Color, "right") => {
                        this.hsv.saturation = (this.hsv.saturation + step).min(1.)
                    }
                    (Channel::Color, "up") => this.hsv.value = (this.hsv.value + step).min(1.),
                    (Channel::Color, "down") => this.hsv.value = (this.hsv.value - step).max(0.),
                    (Channel::Hue, "left" | "down") => this.hsv.hue = (this.hsv.hue - step).max(0.),
                    (Channel::Hue, "right" | "up") => this.hsv.hue = (this.hsv.hue + step).min(1.),
                    _ => return,
                }
                this.commit(window, cx);
                window.prevent_default();
                cx.stop_propagation();
            }))
    }
}

impl EventEmitter<AccentChanged> for AccentPicker {}

pub(super) fn spectrum_dot() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds, _, window, _| {
            let radius = bounds.size.width.min(bounds.size.height) / 2.;
            let center = bounds.center();
            let count = (f32::from(radius) * 2. * window.scale_factor()).ceil() as usize;
            let width = radius * 2. / count.max(1) as f32;
            for index in 0..count {
                let x = -radius + width * (index as f32 + 0.5);
                let half_height = px((f32::from(radius).powi(2) - f32::from(x).powi(2))
                    .max(0.)
                    .sqrt());
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(center.x + x - width / 2., center.y - half_height),
                        point(center.x + x + width / 2., center.y + half_height),
                    ),
                    hsla(index as f32 / count as f32, 0.8, 0.6, 1.),
                ));
            }
        },
    )
    .absolute()
    .inset_0()
}

fn spectrum() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds, _, window, _| {
            for index in 0..6 {
                let left = bounds.left() + bounds.size.width * (index as f32 / 6.);
                let right = bounds.left() + bounds.size.width * ((index + 1) as f32 / 6.);
                window.paint_quad(fill(
                    Bounds::from_corners(point(left, bounds.top()), point(right, bounds.bottom())),
                    linear_gradient(
                        90.,
                        linear_color_stop(hsla(index as f32 / 6., 1., 0.5, 1.), 0.),
                        linear_color_stop(hsla((index + 1) as f32 / 6., 1., 0.5, 1.), 1.),
                    )
                    .color_space(ColorSpace::Srgb),
                ));
            }
        },
    )
    .absolute()
    .inset_0()
}

impl Render for AccentPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity();
        let hue_owner = owner.clone();
        let english = &*gpui_component::locale() == "en";
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .w(px(224.))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if english {
                        "Custom accent"
                    } else {
                        "自定义强调色"
                    }),
            )
            .child(
                self.surface(Channel::Color, cx)
                    .h(px(150.))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(rgb(0xffffff), 0.),
                        linear_color_stop(hsla(self.hsv.hue, 1., 0.5, 1.), 1.),
                    )
                    .color_space(ColorSpace::Srgb))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(5.))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(rgba(0x00000000), 0.),
                                linear_color_stop(rgb(0x000000), 1.),
                            )
                            .color_space(ColorSpace::Srgb)),
                    )
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                owner.update(cx, |this, _| this.color_bounds = bounds)
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(self.hsv.saturation))
                            .top(relative(1. - self.hsv.value))
                            .ml(px(-5.))
                            .mt(px(-5.))
                            .size(px(10.))
                            .rounded_full()
                            .border_2()
                            .border_color(rgb(0xffffff))
                            .bg(self.hsv.color())
                            .shadow_sm(),
                    ),
            )
            .child(
                self.surface(Channel::Hue, cx)
                    .h(px(14.))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(7.))
                            .overflow_hidden()
                            .child(spectrum()),
                    )
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                hue_owner.update(cx, |this, _| this.hue_bounds = bounds)
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(self.hsv.hue))
                            .ml(px(-4.))
                            .top(px(-2.))
                            .w(px(8.))
                            .h(px(18.))
                            .rounded(px(3.))
                            .border_2()
                            .border_color(rgb(0xffffff))
                            .bg(hsla(self.hsv.hue, 1., 0.5, 1.))
                            .shadow_sm(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_shrink_0()
                            .rounded(px(5.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(self.hsv.color()),
                    )
                    .child(div().flex_1().child(Input::new(&self.input).small())),
            )
            .when(self.invalid, |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(if english {
                            "Enter six hex digits, e.g. #7C5CFC"
                        } else {
                            "请输入六位色值，例如 #7C5CFC"
                        }),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{Hsv, normalize_hex};

    #[test]
    fn hex_input_accepts_complete_rgb_only() {
        assert_eq!(normalize_hex(" 7c5cFc "), Some("#7C5CFC".into()));
        assert_eq!(normalize_hex("#000000"), Some("#000000".into()));
        for value in [
            "",
            "#",
            "#123",
            "#12345",
            "#12345678",
            "#12GGFF",
            "#你好蓝色",
        ] {
            assert_eq!(normalize_hex(value), None);
        }
    }

    #[test]
    fn hsv_roundtrip_preserves_rgb_including_gray_and_extremes() {
        for hex in [
            "#000000", "#FFFFFF", "#808080", "#FF0000", "#FFFF00", "#00FF00", "#00FFFF", "#0000FF",
            "#FF00FF", "#6AA9EF", "#7C5CFC", "#012345",
        ] {
            assert_eq!(Hsv::from_hex(hex).hex(), hex);
        }
    }
}
