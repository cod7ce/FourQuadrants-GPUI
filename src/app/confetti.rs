use super::*;
use std::{collections::HashMap, time::Duration};

const DURATION: Duration = Duration::from_millis(1000);
const SPREAD: f32 = 1.5;
const PARTICLE_COUNT: usize = 20;

#[derive(Default)]
pub(super) struct Confetti {
    generation: u64,
    burst: Option<Burst>,
    settling: HashMap<String, Settling>,
}

struct Settling {
    generation: u64,
    pinned: bool,
}

#[derive(Clone, Copy)]
struct Burst {
    window: AnyWindowHandle,
    origin: Point<Pixels>,
}

impl Confetti {
    fn start(&mut self, window: AnyWindowHandle, origin: Point<Pixels>) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.burst = Some(Burst { window, origin });
        self.generation
    }

    fn settle(&mut self, id: String, generation: u64, pinned: bool) {
        self.settling.insert(id, Settling { generation, pinned });
    }

    fn clear(&mut self, id: &str, generation: u64) -> bool {
        let mut changed = false;
        if self
            .settling
            .get(id)
            .is_some_and(|pending| pending.generation == generation)
        {
            self.settling.remove(id);
            changed = true;
        }
        if generation == self.generation {
            changed |= self.burst.take().is_some();
        }
        changed
    }

    fn grouped_as_completed(&self, task: &Task) -> bool {
        task.completed && !self.settling.contains_key(&task.id)
    }

    pub(super) fn holds_pin(&self, task: &Task) -> bool {
        task.completed
            && self
                .settling
                .get(&task.id)
                .is_some_and(|pending| pending.pinned)
    }

    fn retain_completed(&mut self, doc: &Document) {
        let generation = self.generation;
        let cancelled_current = self.settling.iter().any(|(id, pending)| {
            pending.generation == generation && !doc.task(id).is_some_and(|task| task.completed)
        });
        self.settling
            .retain(|id, _| doc.task(id).is_some_and(|task| task.completed));
        if cancelled_current {
            self.burst = None;
        }
    }

    pub(super) fn overlay(&self, window: &Window, p: Palette) -> Option<AnyElement> {
        let burst = self
            .burst
            .filter(|burst| burst.window == window.window_handle())?;
        let colors = [
            p.accent,
            p.quadrant[0],
            p.quadrant[1],
            p.quadrant[2],
            p.quadrant[3],
        ];
        Some(
            div()
                .absolute()
                .inset_0()
                .with_animation(
                    ("task-confetti", self.generation),
                    Animation::new(DURATION),
                    move |el, progress| {
                        el.child(
                            canvas(
                                |_, _, _| (),
                                move |_, (), window, _| {
                                    for index in 0..PARTICLE_COUNT {
                                        let Some(particle) = particle_frame(index, progress) else {
                                            continue;
                                        };
                                        let mut path = PathBuilder::fill();
                                        for (corner, offset) in particle.corners.iter().enumerate()
                                        {
                                            let position = burst.origin + *offset;
                                            if corner == 0 {
                                                path.move_to(position);
                                            } else {
                                                path.line_to(position);
                                            }
                                        }
                                        path.close();
                                        if let Ok(path) = path.build() {
                                            window.paint_path(
                                                path,
                                                colors[index % colors.len()]
                                                    .opacity(particle.opacity),
                                            );
                                        }
                                    }
                                },
                            )
                            .size_full(),
                        )
                    },
                )
                .into_any_element(),
        )
    }
}

struct ParticleFrame {
    corners: [Point<Pixels>; 4],
    opacity: f32,
}

fn particle_frame(index: usize, progress: f32) -> Option<ParticleFrame> {
    if !(0.0..1.0).contains(&progress) {
        return None;
    }
    let seed = index as f32;
    let direction = -std::f32::consts::PI
        + 0.2
        + (index * 7 % PARTICLE_COUNT) as f32 / (PARTICLE_COUNT - 1) as f32
            * (std::f32::consts::PI - 0.4);
    let speed = 100. + (index * 13 % 60) as f32;
    // Keep the arc's shape independent of its playback duration.
    let time = progress * 0.7;
    let x = direction.cos() * speed * time * SPREAD;
    let y = (direction.sin() * speed * time + 150. * time * time) * SPREAD;
    let spin = seed * 1.7 + time * if index.is_multiple_of(2) { 9. } else { -7. };
    let (sin, cos) = spin.sin_cos();
    let half_width =
        (2. + (index % 3) as f32 * 0.5) * (0.35 + 0.65 * (seed + time * 10.).cos().abs());
    let half_height = 1.5 + (index % 2) as f32;
    let corners = [
        (-half_width, -half_height),
        (half_width, -half_height),
        (half_width, half_height),
        (-half_width, half_height),
    ]
    .map(|(dx, dy)| point(px(x + dx * cos - dy * sin), px(y + dx * sin + dy * cos)));
    Some(ParticleFrame {
        corners,
        opacity: (progress / 0.06).min(1.) * ((1. - progress) / 0.35).min(1.),
    })
}

impl Workspace {
    pub(super) fn sync_completion_preferences(&mut self, previous: &model::Settings) {
        if previous.task_completion_sound != self.doc.settings.task_completion_sound {
            crate::native::set_task_complete_sound_enabled(self.doc.settings.task_completion_sound);
        }
        self.confetti.retain_completed(&self.doc);
        if !self.doc.settings.task_confetti {
            self.confetti.burst = None;
            self.confetti.settling.clear();
        }
    }

    pub(super) fn grouped_as_completed(&self, task: &Task) -> bool {
        self.confetti.grouped_as_completed(task)
    }

    fn completion_animation_enabled(&self) -> bool {
        if !self.doc.settings.task_confetti {
            return false;
        }
        // Let isolated debug previews demonstrate motion without changing macOS settings.
        #[cfg(debug_assertions)]
        if std::env::var("FOUR_QUADRANTS_PREVIEW_CONFETTI").as_deref() == Ok("1")
            && crate::store::default_data_path().is_ok_and(|path| path != self.store.path)
            && std::env::args().any(|arg| arg == "--data")
        {
            return true;
        }
        !crate::native::reduce_motion()
    }

    pub(super) fn toggle_task_complete(
        &mut self,
        id: &str,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task) = self.doc.task(id) else {
            return;
        };
        let was_completed = task.completed;
        let was_pinned = self.doc.pinned_task_ids.contains(id);
        let origin = match event {
            ClickEvent::Mouse(event) => event.up.position,
            ClickEvent::Keyboard(event) => event.bounds.center(),
        };
        self.change(cx, |doc| {
            doc.toggle_complete(id);
            Ok(())
        });
        // change installs the new document only after validation and saving succeed.
        if was_completed || !self.doc.task(id).is_some_and(|task| task.completed) {
            return;
        }
        if self.doc.settings.task_completion_sound {
            crate::native::play_task_complete_sound();
        }
        if !self.completion_animation_enabled() {
            return;
        }
        let generation = self.confetti.start(window.window_handle(), origin);
        let id = id.to_owned();
        // Save immediately, but keep each checked row in place until its feedback ends.
        self.confetti.settle(id.clone(), generation, was_pinned);
        cx.spawn(async move |this, cx| {
            // Regroup the row when the one-second burst has finished.
            cx.background_executor().timer(DURATION).await;
            let _ = this.update(cx, |this, cx| {
                if this.confetti.clear(&id, generation) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{Confetti, Document, PARTICLE_COUNT, Task, particle_frame};
    use gpui::{point, px};

    #[test]
    fn checked_tasks_stay_in_place_until_their_own_feedback_ends() {
        let first = Task {
            id: "first".into(),
            completed: true,
            ..Task::default()
        };
        let second = Task {
            id: "second".into(),
            completed: true,
            ..Task::default()
        };
        let mut confetti = Confetti::default();
        confetti.settle(first.id.clone(), 1, true);
        confetti.settle(second.id.clone(), 2, true);
        assert!(confetti.holds_pin(&first));
        assert!(confetti.holds_pin(&second));
        assert!(!confetti.grouped_as_completed(&first));
        assert!(!confetti.grouped_as_completed(&second));
        assert!(first.completed && second.completed);
        assert!(confetti.clear(&first.id, 1));
        assert!(!confetti.holds_pin(&first));
        assert!(confetti.grouped_as_completed(&first));
        assert!(confetti.holds_pin(&second));
        assert!(!confetti.grouped_as_completed(&second));
        assert!(confetti.clear(&second.id, 2));
        assert!(!confetti.holds_pin(&second));
        assert!(confetti.grouped_as_completed(&second));
    }

    #[test]
    fn cancel_delete_and_recomplete_do_not_leave_or_release_stale_rows() {
        let mut doc = Document::default();
        doc.tasks.push(Task {
            id: "task".into(),
            completed: true,
            ..Task::default()
        });
        let mut confetti = Confetti::default();
        confetti.settle("task".into(), 1, true);
        doc.tasks[0].completed = false;
        assert!(!confetti.holds_pin(&doc.tasks[0]));
        confetti.retain_completed(&doc);
        assert!(confetti.settling.is_empty());
        doc.tasks[0].completed = true;
        confetti.settle("task".into(), 2, false);
        assert!(!confetti.holds_pin(&doc.tasks[0]));
        assert!(!confetti.clear("task", 1));
        assert!(!confetti.grouped_as_completed(&doc.tasks[0]));
        doc.tasks.clear();
        confetti.retain_completed(&doc);
        assert!(confetti.settling.is_empty());
    }

    #[test]
    fn particles_stay_local_and_fade_out() {
        for index in 0..PARTICLE_COUNT {
            for step in 0..100 {
                let frame = particle_frame(index, step as f32 / 100.).unwrap();
                assert!((0.0..=1.0).contains(&frame.opacity));
                for corner in frame.corners {
                    assert!(f32::from(corner.x).abs() < 190.);
                    assert!(f32::from(corner.y).abs() < 190.);
                }
            }
            assert_eq!(particle_frame(index, 0.).unwrap().opacity, 0.);
            assert!(particle_frame(index, 0.999).unwrap().opacity < 0.01);
            assert!(particle_frame(index, 1.).is_none());
            assert!(particle_frame(index, 2.).is_none());
        }
    }

    #[test]
    fn particles_spread_on_both_sides_and_rise_before_falling() {
        let centers = (0..PARTICLE_COUNT)
            .map(|index| {
                let frame = particle_frame(index, 0.3).unwrap();
                frame
                    .corners
                    .into_iter()
                    .fold(point(px(0.), px(0.)), |sum, corner| sum + corner)
                    / 4.
            })
            .collect::<Vec<_>>();
        assert!(centers.iter().any(|center| center.x < px(-15.)));
        assert!(centers.iter().any(|center| center.x > px(15.)));
        assert!(centers.iter().any(|center| center.y < px(-15.)));
        let first = particle_frame(0, 0.3).unwrap().corners[0];
        let last = particle_frame(0, 0.9).unwrap().corners[0];
        assert!(last.y > first.y);
    }
}
