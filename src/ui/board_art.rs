use eframe::egui::{self, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

const BLUE_FACE: Color32 = Color32::from_rgb(90, 200, 240);
const BLUE_SIDE: Color32 = Color32::from_rgb(40, 152, 195);
const RED_FACE: Color32 = Color32::from_rgb(248, 90, 93);
const RED_SIDE: Color32 = Color32::from_rgb(196, 52, 63);
const BLUE_INK: Color32 = Color32::from_rgb(11, 34, 53);
const CORAL: Color32 = Color32::from_rgb(255, 152, 149);

#[derive(Clone, Copy, Default)]
pub(crate) struct Motion {
    pub(crate) hover: f32,
    pub(crate) depth: f32,
    focused: bool,
}

impl Motion {
    pub(crate) fn animate(
        ctx: &egui::Context,
        id: egui::Id,
        hover: bool,
        down: bool,
        selected: bool,
        focused: bool,
    ) -> Self {
        Self {
            hover: ctx.animate_bool_with_time(id.with("hover"), hover, 0.14),
            depth: ctx.animate_value_with_time(
                id.with("depth"),
                if down {
                    1.0
                } else if selected {
                    0.57
                } else {
                    0.0
                },
                0.1,
            ),
            focused,
        }
    }
}

fn color_between(a: Color32, b: Color32, blend: f32) -> Color32 {
    let channel = |a: u8, b: u8| egui::lerp(f32::from(a)..=f32::from(b), blend).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

fn key_shape(tile: Rect, motion: Motion) -> (Rect, Rect) {
    let side = tile.width().min(tile.height());
    let base = Rect::from_center_size(tile.center(), Vec2::splat(side)).shrink(side * 0.045);
    let height = side * 0.15;
    let offset = height * motion.depth - side * 0.03 * motion.hover * (1.0 - motion.depth);
    let face = Rect::from_min_max(base.min, base.max - Vec2::new(0.0, height))
        .translate(Vec2::new(0.0, offset));
    (base, face)
}

pub(crate) fn key(painter: &Painter, tile: Rect, slot: u8, motion: Motion) {
    let (base, face) = key_shape(tile, motion);
    let corner = (base.width() * 0.145).round() as u8;
    let shadow = base.translate(Vec2::new(0.0, base.height() * 0.027));
    painter.rect_filled(shadow, corner, Color32::BLACK.gamma_multiply(0.32));
    painter.rect_filled(Rect::from_min_max(face.min, base.max), corner, BLUE_SIDE);
    let surface = color_between(BLUE_FACE, Color32::WHITE, motion.hover * 0.13);
    painter.rect_filled(face, corner, surface);
    painter.rect_stroke(
        face,
        corner,
        Stroke::new(
            1.0_f32,
            Color32::WHITE.gamma_multiply(if motion.focused { 0.92 } else { 0.18 }),
        ),
        egui::StrokeKind::Inside,
    );
    painter.text(
        face.center(),
        egui::Align2::CENTER_CENTER,
        slot,
        FontId::proportional((face.width() * 0.32).clamp(9.0, 31.0)),
        BLUE_INK,
    );
}

#[derive(Clone, Copy)]
pub(crate) struct Dial {
    tile: Rect,
}

impl Dial {
    pub(crate) fn new(rect: Rect) -> Self {
        let side = rect.width().min(rect.height());
        Self {
            tile: Rect::from_center_size(rect.center(), Vec2::splat(side)),
        }
    }

    pub(crate) fn targets(self) -> [Rect; 3] {
        let tile = self.tile;
        let side = tile.width();
        let left = tile.left();
        let top = tile.top();
        let half = tile.center().x;
        let arrow_end = top + side * 0.30;
        [
            Rect::from_min_max(
                Pos2::new(left, top),
                Pos2::new(half - side * 0.02, arrow_end),
            ),
            Rect::from_min_max(
                Pos2::new(left + side * 0.10, arrow_end + side * 0.02),
                Pos2::new(tile.right() - side * 0.10, tile.bottom()),
            ),
            Rect::from_min_max(
                Pos2::new(half + side * 0.02, top),
                Pos2::new(tile.right(), arrow_end),
            ),
        ]
    }

    fn circles(self, motion: Motion) -> (Pos2, f32, Pos2, f32) {
        let button = self.targets()[1];
        let radius = button.height() * 0.47;
        let center = button.center();
        let recess =
            radius * (0.10 * motion.depth - 0.06 - 0.025 * motion.hover * (1.0 - motion.depth));
        (
            center,
            radius,
            center + Vec2::new(0.0, recess),
            radius * 0.91,
        )
    }

    pub(crate) fn body(self, painter: &Painter, number: u8, motion: Motion, angle: f32) {
        let (center, radius, face_center, face_radius) = self.circles(motion);
        painter.circle_filled(center, radius, RED_SIDE);
        let surface = color_between(RED_FACE, Color32::WHITE, motion.hover * 0.14);
        painter.circle_filled(face_center, face_radius, surface);
        painter.circle_stroke(
            face_center,
            face_radius - 0.6,
            Stroke::new(
                1.0_f32,
                Color32::WHITE.gamma_multiply(if motion.focused { 0.92 } else { 0.18 }),
            ),
        );
        let indicator = Vec2::angled(-std::f32::consts::FRAC_PI_2 + angle);
        let start = face_center + indicator * (face_radius * 0.58);
        let end = face_center + indicator * (face_radius * 0.82);
        let width = (face_radius * 0.14).clamp(1.2, 4.3);
        let white = Color32::from_rgb(255, 202, 202);
        painter.line_segment([start, end], Stroke::new(width, white));
        painter.circle_filled(end, width * 0.48, white);
        painter.circle_filled(start, width * 0.48, white);
        painter.text(
            face_center,
            egui::Align2::CENTER_CENTER,
            format!("G{number}"),
            FontId::proportional((face_radius * 0.58).clamp(8.0, 20.0)),
            Color32::WHITE,
        );
    }

    pub(crate) fn arrow(self, painter: &Painter, left: bool, motion: Motion) {
        let rect = self.targets()[if left { 0 } else { 2 }];
        let polarity = if left { -1.0 } else { 1.0 };
        let inner = if left { rect.right() } else { rect.left() };
        let stretch = 0.72 + 0.06 * motion.hover - 0.02 * motion.depth;
        let points: Vec<_> = (0..=12)
            .map(|index| {
                let progress = index as f32 / 12.0;
                Pos2::new(
                    inner + polarity * rect.width() * (0.13 + stretch * progress),
                    rect.top()
                        + rect.height() * (0.34 + 0.33 * progress * progress - 0.10 * motion.hover),
                )
            })
            .collect();
        let tip = points[12];
        let tangent = (tip - points[11]).normalized();
        let normal = Vec2::new(-tangent.y, tangent.x);
        let length = (rect.width() * 0.19).clamp(2.8, 9.0);
        let back = tip - tangent * length;
        let color = color_between(
            CORAL,
            Color32::WHITE,
            (0.5 * motion.hover + 0.8 * motion.depth + if motion.focused { 0.45 } else { 0.0 })
                .min(1.0),
        );
        let thickness = (rect.height() * 0.14).clamp(1.2, 3.5) + 0.45 * motion.hover;
        let stroke = Stroke::new(thickness, color);
        painter.add(egui::Shape::line(points, stroke));
        painter.add(egui::Shape::line(
            vec![
                back + normal * (length * 0.48),
                tip,
                back - normal * (length * 0.48),
            ],
            stroke,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_targets_are_separate_and_contained_at_every_supported_size() {
        for size in [18.0, 36.0, 60.0, 91.0] {
            let tile = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::splat(size));
            let hits = Dial::new(tile).targets();
            for (index, hit) in hits.iter().enumerate() {
                assert!(tile.contains_rect(*hit));
                assert!(hit.width() > 0.0 && hit.height() > 0.0);
                assert!(hits[index + 1..]
                    .iter()
                    .all(|other| !hit.intersects(*other)));
            }
            assert!(hits[0].center().x < hits[1].center().x);
            assert!(hits[2].center().x > hits[1].center().x);
        }
    }

    #[test]
    fn keycap_press_moves_the_face_down_within_the_raised_base() {
        let tile = Rect::from_min_size(Pos2::ZERO, Vec2::splat(60.0));
        let (_, raised) = key_shape(tile, Motion::default());
        let (base, pressed) = key_shape(
            tile,
            Motion {
                depth: 1.0,
                ..Motion::default()
            },
        );
        assert!(pressed.top() > raised.top());
        assert!((pressed.bottom() - base.bottom()).abs() < f32::EPSILON);
        assert!(tile.contains_rect(base) && tile.contains_rect(raised));
    }

    #[test]
    fn animated_dial_face_stays_inside_the_round_bezel() {
        let tile = Rect::from_min_size(Pos2::ZERO, Vec2::splat(84.0));
        for hover in [0.0, 1.0] {
            for depth in [0.0, 0.57, 1.0] {
                let (center, radius, face, face_radius) = Dial::new(tile).circles(Motion {
                    hover,
                    depth,
                    ..Motion::default()
                });
                assert!((face - center).length() + face_radius < radius);
            }
        }
    }
}
