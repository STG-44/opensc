//! Время и движение восстановлены по 0x4CEEB0 / 0x4D02B0 старого EXE.
//! Рисуем Unicode-текст, поэтому растровое размытие старого атласа не копируем.
use bevy::prelude::*;

pub(super) const BACKGROUND_STEP: f64 = 2.5;
pub(super) const HOVER_ENTER: f32 = 0.25;
pub(super) const HOVER_LEAVE: f32 = 0.75;
pub(super) const BULLET_PATH: &str = "textures/object/titlebullets.bmp";

#[derive(Debug, PartialEq)]
pub(super) struct Blend {
    pub from: usize,
    pub to: usize,
    pub alpha: f32,
}

/// WINDOW_BASE перечисляет геометрию снизу вверх: frame5, ..., frame1.
/// Верхние слои исчезают по очереди, затем снова появляются в обратном порядке.
pub(super) fn background(seconds: f64, count: usize) -> Blend {
    assert!(count > 0);
    if count == 1 {
        return Blend {
            from: 0,
            to: 0,
            alpha: 0.0,
        };
    }
    let last = count - 1;
    let position = (seconds.max(0.0) / BACKGROUND_STEP) % (last * 2) as f64;
    let step = position.floor() as usize;
    let index = |n: usize| last.abs_diff(n);
    Blend {
        from: index(step),
        to: index(step + 1),
        alpha: position.fract() as f32,
    }
}

#[derive(Component, Default)]
pub(super) struct Hover {
    pub progress: f32,
}
impl Hover {
    /// Возврат втрое медленнее наведения; быстрый повторный вход не сбрасывает тень.
    pub fn advance(&mut self, selected: bool, seconds: f32) {
        self.progress = if selected {
            (self.progress + seconds / HOVER_ENTER).min(1.0)
        } else {
            (self.progress - seconds / HOVER_LEAVE).max(0.0)
        };
    }
    pub fn offset(&self, text: Vec2, shadow: Vec2) -> Vec2 {
        text + (shadow - text) * (1.0 - self.progress)
    }
}

/// SPRITEFRAME из interface_elements_16: 0 0 63 63 / 0 64 63 127.
/// Обычный и нажатый значки, с относительными границами для HD-замены атласа.
pub(super) fn bullet_rect(size: UVec2, pressed: bool) -> Rect {
    let source = size.as_vec2();
    let top = if pressed { 0.5 } else { 0.0 };
    Rect::from_corners(
        source * Vec2::new(0.0, top),
        source * Vec2::new(63.0 / 128.0, top + 63.0 / 128.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backgrounds_follow_original_order_and_turn_without_a_cut() {
        let expected = [4, 3, 2, 1, 0, 1, 2, 3, 4];
        for (step, &frame) in expected.iter().enumerate() {
            let b = background(step as f64 * 2.5, 5);
            assert_eq!(b.from, frame);
            assert_eq!(b.alpha, 0.0);
            let mid = background(step as f64 * 2.5 + 1.25, 5);
            assert_eq!(mid.from, frame);
            assert_eq!(mid.alpha, 0.5);
            if step < 8 {
                assert_eq!(mid.to, expected[step + 1]);
            }
            if step > 0 {
                assert_eq!(background(step as f64 * 2.5 - 0.0001, 5).to, frame);
            }
        }
        assert_eq!(background(86400.0, 5), background(0.0, 5));
        assert_eq!(
            background(10.0, 1),
            Blend {
                from: 0,
                to: 0,
                alpha: 0.0
            }
        );
    }
    #[test]
    fn hover_returns_slowly_and_reverses_from_its_current_position() {
        let mut h = Hover::default();
        let text = Vec2::new(30.0, 5.0);
        let shadow = Vec2::new(43.0, 13.0);
        h.advance(true, 0.125);
        assert_eq!(h.offset(text, shadow), Vec2::new(36.5, 9.0));
        h.advance(false, 0.1875);
        assert_eq!(h.progress, 0.25);
        h.advance(true, 0.1875);
        assert_eq!(h.offset(text, shadow), text);
        h.advance(false, 0.75);
        assert_eq!(h.offset(text, shadow), shadow);
        h.advance(false, 5.0);
        assert_eq!(h.progress, 0.0);
        h.advance(true, 5.0);
        assert_eq!(h.progress, 1.0);
    }
    #[test]
    fn hover_timing_does_not_depend_on_the_frame_rate() {
        for fps in [15, 30, 60, 144] {
            let mut h = Hover::default();
            for _ in 0..fps {
                h.advance(true, 0.25 / fps as f32);
            }
            assert!((h.progress - 1.0).abs() < 0.00001);
            for _ in 0..fps {
                h.advance(false, 0.75 / fps as f32);
            }
            assert!(h.progress < 0.00001);
        }
    }
    #[test]
    fn pressed_bullet_keeps_alpha_gutters_and_scales_with_override() {
        assert_eq!(
            bullet_rect(UVec2::splat(128), false),
            Rect::new(0.0, 0.0, 63.0, 63.0)
        );
        assert_eq!(
            bullet_rect(UVec2::splat(512), true),
            Rect::new(0.0, 256.0, 252.0, 508.0)
        );
    }
}
