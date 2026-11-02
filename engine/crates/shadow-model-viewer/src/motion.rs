//! Поза, каналы BMF и движение по циклам. Здесь нет окна, ввода и GPU.
use crate::rig::BoneDefinition;
use bevy::prelude::*;
use shadow_formats::BmfMotion;
use std::{collections::BTreeMap, error::Error};

#[derive(Clone)]
pub struct Pose {
    pub bones: Vec<Transform>,
    pub root: Vec3,
}
impl Pose {
    pub fn blend(&self, other: &Self, weight: f32) -> Self {
        let weight = weight.clamp(0.0, 1.0);
        Self {
            bones: self
                .bones
                .iter()
                .zip(&other.bones)
                .map(|(a, b)| Transform {
                    translation: a.translation.lerp(b.translation, weight),
                    rotation: a.rotation.slerp(b.rotation, weight),
                    scale: a.scale.lerp(b.scale, weight),
                })
                .collect(),
            root: self.root.lerp(other.root, weight),
        }
    }
}
pub struct Clip {
    pub frames: Vec<Pose>,
    pub fps: f32,
    pub name: String,
}
impl Clip {
    pub fn new(motion: &BmfMotion, bones: &[BoneDefinition]) -> Result<Self, Box<dyn Error>> {
        Self::with_default_cog(motion, bones, None)
    }
    pub fn with_default_cog(
        motion: &BmfMotion,
        bones: &[BoneDefinition],
        default_cog: Option<Vec3>,
    ) -> Result<Self, Box<dyn Error>> {
        if default_cog.is_some_and(|v| !v.is_finite()) {
            return Err("неверный DEFAULT_COG_POSITION".into());
        }
        if motion.frames.is_empty() || motion.frames_per_second == 0 {
            return Err("клип без кадров или частоты".into());
        }
        let indices = bones
            .iter()
            .enumerate()
            .map(|(i, b)| (b.id, i))
            .collect::<BTreeMap<_, _>>();
        if indices.len() != bones.len() {
            return Err("повтор ID в скелете".into());
        }
        let mut frames = Vec::with_capacity(motion.frames.len());
        for frame in &motion.frames {
            let mut pose = Pose {
                bones: bones.iter().map(|b| b.bind).collect(),
                root: Vec3::from_array(frame.root_translation),
            };
            if let (Some(cog), Some(&index)) = (default_cog, indices.get(&1)) {
                pose.bones[index].translation = cog;
            }
            for key in &frame.keys {
                // Оригинальный 0x0049D520 пропускает Backpack (46).
                if key.bone_id == 46 {
                    continue;
                }
                // 0x0049D620–0x0049D629: отсутствующий узел пропускается.
                // Например, исходные crouch/prone-aim содержат ID 81–83, которых нет у Джона.
                let Some(&i) = indices.get(&key.bone_id) else {
                    continue;
                };
                // В 0x004A10A0 row-major R(q) применяется к векторам-строкам.
                // Bevy нужен R(q)^T: сопряжённый единичный quaternion XYZW.
                pose.bones[i].rotation = key
                    .rotation_wxyz
                    .map(|[w, x, y, z]| Quat::from_xyzw(-x, -y, -z, w).normalize())
                    .unwrap_or(Quat::IDENTITY);
                // Положение из ключа присваивается только тазу (ID 1).
                // Остальные смещения — из SMF: BMF хранит иные пропорции скелета.
                if key.bone_id == 1
                    && let Some(t) = key.translation
                {
                    pose.bones[i].translation = Vec3::from_array(t);
                }
            }
            frames.push(pose);
        }
        Ok(Self {
            frames,
            fps: motion.frames_per_second as f32,
            name: motion.name.clone(),
        })
    }
    pub fn duration(&self) -> f32 {
        (self.frames.len() - 1) as f32 / self.fps
    }
    pub fn sample(&self, seconds: f32) -> Pose {
        let frame =
            (seconds.clamp(0.0, self.duration()) * self.fps).min((self.frames.len() - 1) as f32);
        let a = frame.floor() as usize;
        let b = (a + 1).min(self.frames.len() - 1);
        self.frames[a].blend(&self.frames[b], frame - a as f32)
    }
    pub fn cycle_displacement(&self) -> Vec3 {
        self.frames.last().expect("nonempty clip").root - self.frames[0].root
    }
    /// Дельта включает целые пройденные циклы, поэтому на стыке нет шага назад.
    pub fn advance(&self, seconds: &mut f32, dt: f32) -> Vec3 {
        let duration = self.duration();
        if duration == 0.0 || dt <= 0.0 {
            return Vec3::ZERO;
        }
        let old = self.sample(*seconds).root;
        let total = *seconds + dt;
        let cycles = (total / duration).floor();
        *seconds = total.rem_euclid(duration);
        self.cycle_displacement() * cycles + self.sample(*seconds).root - old
    }
    pub fn displacement_at(&self, seconds: f32) -> Vec3 {
        self.sample(seconds).root - self.frames[0].root
    }
}
struct Transition {
    source: Pose,
    elapsed: f32,
}
#[derive(Resource)]
pub struct Playback {
    pub selected: usize,
    pub seconds: f32,
    pub paused: bool,
    pub speed: f32,
    pub root_motion: bool,
    /// Накопленное перемещение в координатах оригинала, до масштаба витрины.
    pub travel: Vec3,
    pub pose: Pose,
    transition: Option<Transition>,
}
const BLEND_SECONDS: f32 = 0.25;
impl Playback {
    pub fn new(
        clips: &[Clip],
        selected: usize,
        seconds: f32,
        paused: bool,
        root_motion: bool,
    ) -> Self {
        let clip = &clips[selected];
        Self {
            selected,
            seconds,
            paused,
            speed: 1.0,
            root_motion,
            travel: if root_motion {
                clip.displacement_at(seconds)
            } else {
                Vec3::ZERO
            },
            pose: clip.sample(seconds),
            transition: None,
        }
    }
    pub fn select(&mut self, clips: &[Clip], selected: usize) {
        if selected == self.selected {
            return;
        }
        self.transition = (!self.paused).then(|| Transition {
            source: self.pose.clone(),
            elapsed: 0.0,
        });
        self.selected = selected;
        self.seconds = 0.0;
        if self.paused {
            self.pose = clips[selected].sample(0.0);
        }
    }
    pub fn step_frame(&mut self, clips: &[Clip], direction: f32) {
        let clip = &clips[self.selected];
        self.paused = true;
        self.transition = None;
        let next = (self.seconds + direction / clip.fps).clamp(0.0, clip.duration());
        if self.root_motion {
            self.travel += clip.displacement_at(next) - clip.displacement_at(self.seconds);
        }
        self.seconds = next;
        self.pose = clip.sample(next);
    }
    pub fn reset(&mut self, clips: &[Clip]) {
        self.seconds = 0.0;
        self.travel = Vec3::ZERO;
        self.transition = None;
        self.pose = clips[self.selected].sample(0.0);
    }
    pub fn tick(&mut self, clips: &[Clip], dt: f32) {
        if self.paused {
            return;
        }
        let clip = &clips[self.selected];
        let delta = clip.advance(&mut self.seconds, dt * self.speed);
        let target = clip.sample(self.seconds);
        let weight = if let Some(t) = &mut self.transition {
            t.elapsed += dt;
            (t.elapsed / BLEND_SECONDS).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // При старте ходьбы скорость набирается вместе с переходом позы.
        if self.root_motion {
            self.travel += delta * weight;
        }
        self.pose = self
            .transition
            .as_ref()
            .map(|t| t.source.blend(&target, weight))
            .unwrap_or(target);
        if weight >= 1.0 {
            self.transition = None;
        }
    }
}
/// Размер и оси витрины одинаково применяются к геометрии и перемещению.
pub fn to_world_delta(basis: &Transform, source: Vec3) -> Vec3 {
    basis.rotation * (source * basis.scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_formats::{BmfFrame, BmfKey};
    fn bones() -> Vec<BoneDefinition> {
        [1, 51, 55, 46]
            .into_iter()
            .map(|id| BoneDefinition {
                id,
                name: format!("{id}"),
                parent: None,
                bind: Transform::from_xyz(id as f32, 2.0, 100.0),
            })
            .collect()
    }
    fn motion() -> BmfMotion {
        BmfMotion {
            name: "test".into(),
            frames_per_second: 10,
            ticks_per_frame: 480,
            stored_frame_millis: 100,
            flags: [1, 1],
            bone_ids: vec![1, 51],
            frames: (0..2)
                .map(|i| BmfFrame {
                    root_translation: [0.0, 20.0 + i as f32 * 10.0, 0.0],
                    index: i,
                    flags: [0, 0],
                    keys: vec![
                        BmfKey {
                            bone_id: 1,
                            rotation_wxyz: None,
                            translation: Some([0.0, 0.0, 90.0 + i as f32 * 10.0]),
                        },
                        BmfKey {
                            bone_id: 51,
                            rotation_wxyz: Some(if i == 0 {
                                [1.0, 0.0, 0.0, 0.0]
                            } else {
                                [0.70710677, 0.0, 0.0, 0.70710677]
                            }),
                            translation: Some([-999.0; 3]),
                        },
                    ],
                })
                .collect(),
        }
    }
    fn clip() -> Clip {
        Clip::new(&motion(), &bones()).unwrap()
    }
    fn near(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-4, "{a:?} != {b:?}");
    }
    #[test]
    fn interpolates_pelvis_but_keeps_other_bone_offsets() {
        let p = clip().sample(0.05);
        near(p.bones[0].translation, Vec3::new(0.0, 0.0, 95.0));
        assert_eq!(p.bones[1].translation, bones()[1].bind.translation);
        near(p.root, Vec3::new(0.0, 25.0, 0.0));
        assert_eq!(p.bones[2], bones()[2].bind);
    }
    #[test]
    fn conjugates_rotations_and_clamps_endpoints() {
        let c = clip();
        near(
            c.sample(0.05).bones[1].rotation * Vec3::X,
            Quat::from_rotation_z(-std::f32::consts::FRAC_PI_4) * Vec3::X,
        );
        assert_eq!(c.sample(-1.0).bones[1].rotation, Quat::IDENTITY);
        near(c.sample(1.0).bones[1].rotation * Vec3::X, -Vec3::Y);
        let mut m = motion();
        m.frames[1].keys[1].rotation_wxyz = Some([0.70710677, 0.70710677, 0.0, 0.0]);
        near(
            Clip::new(&m, &bones()).unwrap().sample(0.1).bones[1].rotation * Vec3::Y,
            -Vec3::Z,
        );
    }
    #[test]
    fn missing_rotation_is_identity_and_backpack_is_untouched() {
        let mut bs = bones();
        bs[1].bind.rotation = Quat::from_rotation_y(0.7);
        let mut m = motion();
        m.frames[0].keys[1].rotation_wxyz = None;
        m.frames[0].keys.push(BmfKey {
            bone_id: 46,
            rotation_wxyz: Some([0.0, 1.0, 0.0, 0.0]),
            translation: Some([999.0; 3]),
        });
        let p = Clip::new(&m, &bs).unwrap().sample(0.0);
        assert_eq!(p.bones[1].rotation, Quat::IDENTITY);
        assert_eq!(p.bones[3], bs[3].bind);
    }
    #[test]
    fn skips_absent_rig_nodes_like_original_and_rejects_empty_clips() {
        let mut m = motion();
        m.frames[0].keys[0].bone_id = 999;
        let clip = Clip::new(&m, &bones()).unwrap();
        assert_eq!(
            clip.frames[0].bones[0].translation,
            bones()[0].bind.translation
        );
        assert_eq!(clip.frames.len(), m.frames.len());
        m.frames.clear();
        assert!(Clip::new(&m, &bones()).is_err());
    }
    #[test]
    fn default_cog_supplies_static_pose_height_but_explicit_pelvis_key_wins() {
        let mut m = motion();
        m.frames[0].keys[0].translation = None;
        let cog = Vec3::new(0.0, 0.0, 44.0495);
        let clip = Clip::with_default_cog(&m, &bones(), Some(cog)).unwrap();
        assert_eq!(clip.frames[0].bones[0].translation, cog);
        assert_eq!(
            clip.frames[1].bones[0].translation,
            Vec3::new(0.0, 0.0, 100.0)
        );
        assert_eq!(
            clip.frames[0].bones[1].translation,
            bones()[1].bind.translation
        );
        assert_eq!(clip.frames[0].root, m.frames[0].root_translation.into());
    }
    #[test]
    fn invalid_default_cog_is_rejected_before_pose_construction() {
        assert!(Clip::with_default_cog(&motion(), &bones(), Some(Vec3::splat(f32::NAN))).is_err());
    }
    #[test]
    fn root_delta_is_continuous_across_one_and_multiple_cycles() {
        let c = clip();
        let mut t = 0.09;
        near(c.advance(&mut t, 0.03), Vec3::new(0.0, 3.0, 0.0));
        assert!((t - 0.02).abs() < 1e-5);
        near(c.advance(&mut t, 0.35), Vec3::new(0.0, 35.0, 0.0));
        assert!((t - 0.07).abs() < 1e-5);
        t = c.duration();
        near(c.advance(&mut t, 0.01), Vec3::Y);
        near(c.displacement_at(0.0), Vec3::ZERO);
    }
    #[test]
    fn single_frame_has_no_motion_or_division_by_zero() {
        let mut m = motion();
        m.frames.truncate(1);
        let c = Clip::new(&m, &bones()).unwrap();
        let mut t = 0.0;
        assert_eq!(c.advance(&mut t, 20.0), Vec3::ZERO);
        assert_eq!(c.sample(20.0).bones[0], c.frames[0].bones[0]);
    }
    #[test]
    fn pause_frame_step_and_in_place_mode_keep_travel_consistent() {
        let clips = [clip()];
        let mut p = Playback::new(&clips, 0, 0.05, true, true);
        let initial = p.travel;
        p.tick(&clips, 0.1);
        assert_eq!(p.travel, initial);
        p.step_frame(&clips, 1.0);
        near(p.travel, Vec3::Y * 10.0);
        p.step_frame(&clips, -1.0);
        assert_eq!(p.travel, Vec3::ZERO);
        p.paused = false;
        p.root_motion = false;
        p.tick(&clips, 0.03);
        assert_eq!(p.travel, Vec3::ZERO);
        p.root_motion = true;
        p.tick(&clips, 0.02);
        near(p.travel, Vec3::Y * 2.0);
        p.reset(&clips);
        assert_eq!(p.travel, Vec3::ZERO);
    }
    #[test]
    fn changing_clips_blends_pose_and_preserves_world_position() {
        let mut idle = motion();
        idle.frames.truncate(1);
        idle.frames[0].keys[0].translation = None;
        idle.frames[0].root_translation = [0.0; 3];
        let clips = [clip(), Clip::new(&idle, &bones()).unwrap()];
        let mut p = Playback::new(&clips, 0, 0.05, false, true);
        let initial = p.travel;
        p.select(&clips, 1);
        p.tick(&clips, 0.125);
        assert_eq!(p.travel, initial);
        assert!((p.pose.bones[0].translation.z - 97.5).abs() < 1e-4);
        p.tick(&clips, 0.125);
        assert_eq!(
            p.pose.bones[0].translation,
            clips[1].frames[0].bones[0].translation
        );
        assert_eq!(p.travel, initial);
    }
    #[test]
    fn converts_root_displacement_with_the_same_axes_and_scale_as_the_model() {
        let basis = Transform::from_xyz(5.0, 7.0, 9.0)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::splat(0.02));
        near(
            to_world_delta(&basis, Vec3::new(100.0, 200.0, 300.0)),
            Vec3::new(2.0, 6.0, -4.0),
        );
    }
}
