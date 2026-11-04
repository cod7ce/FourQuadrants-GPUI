use super::*;

const REACTION: TimeDuration = TimeDuration::from_millis(640);

// Original 9×9 sprite. Each cell is painted on the physical pixel grid, so
// neither Retina scaling nor the Island's changing width blurs the artwork.
const SPRITE: [u16; 9] = [
    0b001000100,
    0b000101000,
    0b001111100,
    0b011111110,
    0b111111111,
    0b101111101,
    0b001111100,
    0b001000100,
    0b011000110,
];

fn cell(window: &mut Window, origin: Point<Pixels>, x: f32, y: f32, unit: f32, color: Hsla) {
    let scale = window.scale_factor();
    let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
    let start = point(snap(origin.x + px(x * unit)), snap(origin.y + px(y * unit)));
    let end = point(
        snap(origin.x + px((x + 1.) * unit)),
        snap(origin.y + px((y + 1.) * unit)),
    );
    window.paint_quad(fill(Bounds::from_corners(start, end), color));
}

fn sprite(empty: bool, celebrating: bool, progress: f32, p: Palette) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            // Deliberately stepped poses instead of smoothly scaling pixel art.
            let frame = (progress * 8.).floor() as u8;
            let hop = match frame {
                1 | 4 => 1.,
                2 | 3 => 2.,
                _ => 0.,
            };
            // Center the 18pt sprite in its 24pt box; the old 4pt top inset
            // left it visibly lower than the counter.
            let origin = point(bounds.left() + px(3.), bounds.top() + px(3. - hop));
            let body = if empty && !celebrating {
                p.accent.opacity(0.6)
            } else {
                p.accent
            };
            for (y, row) in SPRITE.iter().enumerate() {
                for x in 0..9 {
                    if row & (1 << (8 - x)) != 0 {
                        let color = if y == 2 {
                            body.blend(white().opacity(0.35))
                        } else {
                            body
                        };
                        cell(window, origin, x as f32, y as f32, 2., color);
                    }
                }
            }
            if celebrating {
                // Happy, upturned eyes and a small smile.
                for (x, y) in [(2., 4.), (3., 3.), (5., 3.), (6., 4.), (4., 6.)] {
                    cell(window, origin, x, y, 2., black());
                }
                if (1..6).contains(&frame) {
                    for (x, y) in [(0., 2.), (11., 4.), (10., 0.)] {
                        cell(window, bounds.origin, x, y, 2., p.quadrant[2]);
                    }
                }
            } else {
                let blink = empty || frame == 3;
                for x in [2., 5.] {
                    cell(window, origin, x, 4., 2., black());
                    if !blink {
                        cell(window, origin, x, 5., 2., black());
                    }
                }
            }
        },
    )
    .size(px(24.))
}

pub(super) fn mascot(
    empty: bool,
    celebrating: bool,
    generation: u64,
    motion: bool,
    p: Palette,
) -> AnyElement {
    let view = div().size(px(24.)).flex_shrink_0();
    if motion && generation > 0 {
        view.with_animation(
            ("island-pixel-mascot", generation),
            Animation::new(REACTION),
            move |el, progress| el.child(sprite(empty, celebrating, progress, p)),
        )
        .into_any_element()
    } else {
        view.child(sprite(empty, celebrating, 1., p))
            .into_any_element()
    }
}

fn digit(character: u8) -> [u8; 5] {
    match character {
        b'0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        b'1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        b'2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        b'3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        b'4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        b'5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        b'6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        b'7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        b'8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        b'9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        b'+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => [0; 5],
    }
}

fn label(count: usize, celebrating: bool) -> String {
    if celebrating {
        "+1".into()
    } else if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}

fn digits(text: String, progress: f32, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let width = text.len() as f32 * 8. - 2.;
            let lift = if progress < 0.5 { 1. } else { 0. };
            let origin = point(
                bounds.center().x - px(width / 2.),
                bounds.center().y - px(5. + lift),
            );
            for (index, character) in text.bytes().enumerate() {
                for (y, row) in digit(character).iter().enumerate() {
                    for x in 0..3 {
                        if row & (1 << (2 - x)) != 0 {
                            cell(window, origin, (index * 4 + x) as f32, y as f32, 2., color);
                        }
                    }
                }
            }
        },
    )
    .w(px(30.))
    .h(px(20.))
}

pub(super) fn counter(
    count: usize,
    celebrating: bool,
    generation: u64,
    motion: bool,
    p: Palette,
) -> AnyElement {
    // At most three glyphs (`99+`): large counts cannot cover the camera or
    // widen the Island. The parent tooltip retains the full numeric value.
    let text = label(count, celebrating);
    let color = if celebrating {
        p.accent
    } else if count == 0 {
        p.muted
    } else {
        p.text
    };
    let view = div().w(px(30.)).h(px(20.)).flex_shrink_0();
    if motion && generation > 0 {
        view.with_animation(
            ("island-pixel-counter", generation),
            Animation::new(TimeDuration::from_millis(240)),
            move |el, progress| el.child(digits(text.clone(), progress, color)),
        )
        .into_any_element()
    } else {
        view.child(digits(text, 1., color)).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::label;

    #[test]
    fn counter_caps_at_99_and_keeps_celebration() {
        assert_eq!(label(0, false), "0");
        assert_eq!(label(99, false), "99");
        assert_eq!(label(100, false), "99+");
        assert_eq!(label(100, true), "+1");
    }
}
