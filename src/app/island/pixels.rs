use super::*;

const REACTION: TimeDuration = TimeDuration::from_millis(640);
pub(super) const JUGGLE_DURATION: TimeDuration = TimeDuration::from_millis(2400);

// Original monochrome mascot, with a separate app-icon ball at its feet.
// Each cell is snapped to the physical pixel grid to keep Retina edges crisp.
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
// Same quadrant order and RGB values as assets/app-icon.png.
const ICON_COLORS: [u32; 4] = [0xff3b30, 0x007aff, 0xff9500, 0x8e8e93];
// A muted light gray reads as the icon's white backing without becoming the
// brightest element in the compact Island.
const BALL_LIGHT: u32 = 0xe5e5e5;
// White-backed pixel ball with an icon-colored core and white cross.
const BALL: [u16; 7] = [
    0b0111110, 0b1111111, 0b1111111, 0b1111111, 0b1111111, 0b1111111, 0b0111110,
];

fn ball_quadrant(mut x: usize, mut y: usize, turns: usize) -> Option<usize> {
    for _ in 0..turns % 4 {
        (x, y) = (y, BALL.len() - 1 - x);
    }
    if matches!(x, 1..=2 | 4..=5) && matches!(y, 1..=2 | 4..=5) {
        Some((y / 4) * 2 + x / 4)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct JugglePose {
    reach: usize,
    foot_lift: f32,
    ball_x: f32,
    ball_y: f32,
    turns: usize,
}

fn juggle_pose(progress: f32) -> JugglePose {
    let frame = ((progress.clamp(0., 1.) * 18.).floor() as usize).min(18);
    // Scoop, two vertical arcs, then catch. At each contact the ball's
    // bottom meets the raised foot, rather than bouncing off empty space.
    let (reach, foot_lift, dx, lift, turns) = match frame {
        1 => (1, 0., 0., 0., 0),
        2 => (2, 0., -1., 2., 0),
        3 => (2, 1., -2., 5., 0),
        4 => (1, 1., -2., 9., 0),
        5 => (0, 0., -2., 12., 1),
        6 => (0, 0., -2., 14., 1),
        7 => (0, 0., -2., 13., 1),
        8 => (0, 0., -2., 11., 1),
        9 => (1, 0., -2., 8., 1),
        10 => (2, 1., -2., 5., 2),
        11 => (1, 1., -2., 9., 2),
        12 => (0, 0., -2., 12., 2),
        13 => (0, 0., -2., 13., 2),
        14 => (0, 0., -2., 12., 3),
        15 => (1, 0., -2., 9., 3),
        16 => (2, 1., -2., 5., 3),
        17 => (1, 0., -1., 2., 0),
        _ => (0, 0., 0., 0., 0),
    };
    JugglePose {
        reach,
        foot_lift,
        ball_x: 22. + dx,
        ball_y: 16. - lift,
        turns,
    }
}

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

fn sprite(
    empty: bool,
    celebrating: bool,
    progress: f32,
    juggle: Option<f32>,
    p: Palette,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            // Deliberately stepped poses instead of smoothly scaling pixel art.
            let frame = if juggle.is_some() {
                8
            } else {
                (progress * 8.).floor() as u8
            };
            let hop = match frame {
                1 | 4 => 1.,
                2 | 3 => 2.,
                _ => 0.,
            };
            let origin = point(bounds.left() + px(3.), bounds.top() + px(3. - hop));
            let playing = juggle.is_some_and(|progress| progress < 1.);
            let opacity = if empty && !celebrating && !playing {
                0.6
            } else {
                1.
            };
            let pose = juggle_pose(juggle.unwrap_or(1.));
            for (y, row) in SPRITE.iter().enumerate() {
                for x in 0..9 {
                    if row & (1 << (8 - x)) != 0 {
                        let body = p.accent;
                        let color = if y == 2 {
                            body.blend(white().opacity(0.35))
                        } else {
                            body
                        };
                        let leg_offset = if y == 8 && x >= 6 {
                            pose.reach
                        } else if y == 7 && x == 6 {
                            pose.reach / 2
                        } else {
                            0
                        };
                        cell(
                            window,
                            origin,
                            (x + leg_offset) as f32,
                            y as f32
                                - if y == 8 && x >= 6 {
                                    pose.foot_lift / 2.
                                } else {
                                    0.
                                },
                            2.,
                            color.opacity(opacity),
                        );
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
                let blink = (empty && !playing) || frame == 3;
                for x in [2., 5.] {
                    cell(window, origin, x, 4., 2., black());
                    if !blink {
                        cell(window, origin, x, 5., 2., black());
                    }
                }
            }
            let ball_origin = point(
                bounds.left() + px(pose.ball_x),
                bounds.top() + px(pose.ball_y),
            );
            // Quarter turns imply spin without blurred transforms. The muted
            // white backing and cross rotate together with the colored core.
            for (y, row) in BALL.iter().enumerate() {
                for x in 0..BALL.len() {
                    if row & (1 << (BALL.len() - 1 - x)) == 0 {
                        continue;
                    }
                    let color = if let Some(quadrant) = ball_quadrant(x, y, pose.turns) {
                        Hsla::from(rgb(ICON_COLORS[quadrant]))
                    } else {
                        Hsla::from(rgb(BALL_LIGHT))
                    };
                    cell(
                        window,
                        ball_origin,
                        x as f32,
                        y as f32,
                        1.,
                        color.opacity(opacity),
                    );
                }
            }
        },
    )
    .w(px(36.))
    .h(px(24.))
}

pub(super) fn mascot(
    empty: bool,
    celebrating: bool,
    generation: u64,
    juggle_generation: Option<u64>,
    motion: bool,
    p: Palette,
) -> AnyElement {
    let view = div().w(px(36.)).h(px(24.)).flex_shrink_0();
    if let Some(juggle_generation) = juggle_generation.filter(|_| motion) {
        view.with_animation(
            ("island-pixel-juggle", juggle_generation),
            Animation::new(JUGGLE_DURATION),
            move |el, progress| el.child(sprite(empty, celebrating, 1., Some(progress), p)),
        )
        .into_any_element()
    } else if motion && generation > 0 {
        view.with_animation(
            ("island-pixel-mascot", generation),
            Animation::new(REACTION),
            move |el, progress| el.child(sprite(empty, celebrating, progress, None, p)),
        )
        .into_any_element()
    } else {
        view.child(sprite(empty, celebrating, 1., None, p))
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
    use super::{BALL, ball_quadrant, juggle_pose, label};

    #[test]
    fn white_cross_and_four_color_cores_rotate_without_splitting() {
        for turns in 0..=4 {
            let mut counts = [0; 4];
            for (y, row) in BALL.iter().enumerate() {
                for x in 0..BALL.len() {
                    if row & (1 << (BALL.len() - 1 - x)) == 0 {
                        continue;
                    }
                    if let Some(quadrant) = ball_quadrant(x, y, turns) {
                        counts[quadrant] += 1;
                    }
                }
            }
            assert_eq!(counts, [4; 4], "turn {turns}");
        }
        for turns in 0..4 {
            for coordinate in 0..BALL.len() {
                assert_eq!(ball_quadrant(coordinate, 3, turns), None);
                assert_eq!(ball_quadrant(3, coordinate, turns), None);
            }
        }
        assert_eq!(BALL[0], 0b0111110);
        assert_eq!(BALL[6], 0b0111110);
    }

    #[test]
    fn juggle_returns_to_rest_and_stays_inside_its_camera_safe_box() {
        assert_eq!(juggle_pose(0.), juggle_pose(1.));
        assert_eq!(juggle_pose(-1.), juggle_pose(0.));
        assert_eq!(juggle_pose(2.), juggle_pose(1.));
        let mut lifted = false;
        let mut extended = false;
        for step in 0..=160 {
            let pose = juggle_pose(step as f32 / 160.);
            assert!((20. ..=22.).contains(&pose.ball_x));
            assert!(pose.ball_x + 7. <= 36.);
            assert!(pose.ball_y >= 0. && pose.ball_y + 7. <= 24.);
            assert!(3. + (8 + pose.reach) as f32 * 2. <= 36.);
            lifted |= pose.ball_y < 16.;
            extended |= pose.reach > 0;
        }
        assert!(lifted && extended);
    }

    #[test]
    fn juggling_rebounds_above_the_foot_and_catches_the_last_descent() {
        let at_frame = |frame| juggle_pose((frame as f32 + 0.25) / 18.);
        for frame in [3, 10, 16] {
            let contact = at_frame(frame);
            assert_eq!(contact.ball_y + 7., 19. - contact.foot_lift);
            let foot_left = 3. + (6 + contact.reach) as f32 * 2.;
            assert!(contact.ball_x < foot_left + 4.);
            assert!(contact.ball_x + 7. > foot_left);
        }
        let rebound = at_frame(10);
        assert!(at_frame(9).ball_y < rebound.ball_y);
        assert!(at_frame(11).ball_y < rebound.ball_y);
        assert!(at_frame(6).ball_y < at_frame(3).ball_y);
        assert!(at_frame(13).ball_y < rebound.ball_y);
        for frame in 3..=16 {
            assert_eq!(at_frame(frame).ball_x, rebound.ball_x);
        }
        assert!(at_frame(17).ball_y > at_frame(16).ball_y);
    }

    #[test]
    fn counter_caps_at_99_and_keeps_celebration() {
        assert_eq!(label(0, false), "0");
        assert_eq!(label(99, false), "99");
        assert_eq!(label(100, false), "99+");
        assert_eq!(label(100, true), "+1");
    }
}
