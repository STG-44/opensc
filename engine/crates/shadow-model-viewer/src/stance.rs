//! Постоянная стойка отделена от временных действий (подбор, резание, стрельба).
use crate::{combat, fence, grenade, inventory, motion::Clip, pickup, player, tutorial};
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, BmfMotion, config_lines};
use std::error::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Posture {
    #[default]
    Stand,
    Crouch,
    Prone,
}
impl Posture {
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Stand => "Стоя",
            Self::Crouch => "В приседе",
            Self::Prone => "Лёжа",
        }
    }
}
pub struct Clips {
    pub idle: [usize; 3],
    pub walk: [usize; 3],
    pub run: [usize; 3],
    pub rifle_move: [usize; 2],
    pub transition: [[Option<usize>; 3]; 3],
    pub pistol: [(usize, usize); 3],
    pub rifle: [(usize, usize); 3],
    pub pickup: [(usize, f32); 3],
    pub source: serde_json::Value,
}
pub(crate) fn default_cog(
    lines: &[shadow_formats::ConfigLine],
    state: &str,
) -> Result<Vec3, Box<dyn Error>> {
    let matches: Vec<_> = lines
        .iter()
        .filter(|l| {
            l.words.len() == 5
                && l.words[0] == "DEFAULT_COG_POSITION"
                && l.words[1] == format!("MSEQ_STATE_{state}")
        })
        .collect();
    let [line] = matches.as_slice() else {
        return Err(format!("нужен один DEFAULT_COG_POSITION {state}").into());
    };
    let cog = Vec3::new(
        line.words[2].parse()?,
        line.words[3].parse()?,
        line.words[4].parse()?,
    );
    if !cog.is_finite() {
        return Err("нечисловой DEFAULT_COG_POSITION".into());
    }
    Ok(cog)
}
pub fn prepare(catalog: &AssetCatalog, p: &mut player::Player) -> Result<(), Box<dyn Error>> {
    let bones = &p
        .rig
        .as_ref()
        .ok_or("стойки готовятся до создания сцены")?
        .bones;
    let lines = config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
    let cogs = [
        default_cog(&lines, "STAND")?,
        default_cog(&lines, "CROUCH")?,
        default_cog(&lines, "PRONE")?,
    ];
    let mut loaded = Vec::new();
    let mut load = |name: &str, callback: Option<&str>| -> Result<(usize, f32), Box<dyn Error>> {
        let posture = if name.contains("CROUCH") {
            1
        } else if name.contains("PRONE") || name.contains("BELLY") {
            2
        } else {
            0
        };
        let (clip, frame) =
            combat::sequence_with_cog(catalog, name, callback, bones, Some(cogs[posture]))?;
        let index = p.clips.len();
        let seconds = frame.map(|f| f as f32 / clip.fps).unwrap_or(0.0);
        loaded.push(serde_json::json!({"sequence":name,"motion":clip.name,"fps":clip.fps,"duration_seconds":clip.duration(),"callback_seconds":frame.map(|_|seconds)}));
        p.clips.push(clip);
        Ok((index, seconds))
    };
    let mut idle = [0; 3];
    let mut walk = [1; 3];
    let mut run = [2; 3];
    let mut pistol = [(0, 0); 3];
    let mut rifle = [(0, 0); 3];
    let mut pick = [(4, 0.0); 3];
    for (i, name) in ["STAND", "CROUCH", "PRONE"].iter().enumerate() {
        idle[i] = load(&format!("MSEQ_{name}"), None)?.0;
        pistol[i] = (
            load(&format!("MSEQ_{name}_INTO_FIRE_PISTOL"), None)?.0,
            load(&format!("MSEQ_{name}_FIRE_PISTOL"), None)?.0,
        );
        rifle[i] = (
            load(&format!("MSEQ_{name}_INTO_FIRE_RIFLE"), None)?.0,
            load(&format!("MSEQ_{name}_FIRE_RIFLE"), None)?.0,
        );
        pick[i] = load(
            &format!("MSEQ_{name}_PICK_UP_ONE_HAND"),
            Some("MCB_PICK_UP_ITEM"),
        )?;
    }
    walk[1] = load("MSEQ_CROUCH_WALK", None)?.0;
    run[1] = load("MSEQ_CROUCH_RUN", None)?.0;
    walk[2] = load("MSEQ_BELLY_CRAWL", None)?.0;
    run[2] = load("MSEQ_BELLY_CRAWL_FAST", None)?.0;
    let rifle_move = [
        load("MSEQ_WALK_CARRYING_RIFLE", None)?.0,
        load("MSEQ_RUN_RIFLE", None)?.0,
    ];
    let lines = config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
    let mut transition = [[None; 3]; 3];
    for (from, a) in ["STAND", "CROUCH", "PRONE"].iter().enumerate() {
        for (to, b) in ["STAND", "CROUCH", "PRONE"].iter().enumerate() {
            if from == to {
                continue;
            }
            let hits: Vec<_> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    l.words.len() == 4
                        && l.words[0] == "BEGIN_TRANSITION_SEQ"
                        && l.words[1] == format!("MSEQ_STATE_{a}")
                        && l.words[2] == format!("MSEQ_STATE_{b}")
                })
                .map(|(i, _)| i)
                .collect();
            let [start] = hits.as_slice() else {
                return Err(format!("нужен один переход {a} → {b}").into());
            };
            let end = lines[start + 1..]
                .iter()
                .position(|l| l.words[0] == "END_SEQUENCE")
                .ok_or("переход не завершён")?
                + start
                + 1;
            let motions: Vec<_> = lines[start + 1..end]
                .iter()
                .filter(|l| l.words[0] == "MOTION")
                .collect();
            let [motion] = motions.as_slice() else {
                return Err("нужен один клип перехода стойки".into());
            };
            if motion.words.len() != 2 {
                return Err("неподдержанный режим перехода стойки".into());
            }
            let path = format!("motions/{}.bmf", motion.words[1].to_ascii_lowercase());
            let clip = Clip::new(&BmfMotion::from_bytes(&catalog.read(&path)?)?, bones)?;
            loaded.push(serde_json::json!({"transition":lines[*start].words,"motion":clip.name,"duration_seconds":clip.duration(),"origin":catalog.source_label(&path)?}));
            transition[from][to] = Some(p.clips.len());
            p.clips.push(clip);
        }
    }
    p.postures = Some(Clips {
        idle,
        walk,
        run,
        rifle_move,
        transition,
        pistol,
        rifle,
        pickup: pick,
        source: serde_json::json!({"motions":loaded,"movement":"source BMF root displacement at map scale 0.01","default_cog_positions_source_units":cogs.map(|v|v.to_array()),"missing_nodes":"skipped as in original 0x0049D629","body_height_collision":false,"transition_root_translation_applied":false}),
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<ResMut<inventory::Inventory>>,
    t: Option<Res<tutorial::Tutorial>>,
    c: Option<ResMut<combat::Combat>>,
    item: Option<ResMut<pickup::Pickup>>,
    g: Option<ResMut<grenade::Grenade>>,
    f: Option<ResMut<fence::Fence>>,
    u: Option<ResMut<crate::smg::Uzi>>,
    m: Option<ResMut<crate::medical::Medical>>,
    a: Option<ResMut<crate::armor::Armor>>,
    hud: Option<Res<crate::hud::Hud>>,
) {
    if t.is_some_and(|t| t.active.is_some() || t.consumed_input)
        || inv.as_ref().is_some_and(|i| i.open || i.consumed_input)
    {
        return;
    }
    let Some(mut p) = p else {
        return;
    };
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let desired = if let Some(posture) = hud.and_then(|h| h.command_posture) {
        Some(posture)
    } else if keys.just_pressed(KeyCode::KeyS) {
        Some(Posture::Stand)
    } else if keys.just_pressed(KeyCode::KeyC) {
        Some(Posture::Crouch)
    } else if keys.just_pressed(KeyCode::KeyP) && !ctrl {
        Some(Posture::Prone)
    } else {
        None
    };
    if !p.selected || p.playback.paused {
        return;
    }
    let Some(desired) = desired else {
        return;
    };
    if p.posture_transition.is_some() || desired == p.posture {
        return;
    }
    if let Some(mut c) = c {
        c.cancel_all(&mut p);
    }
    if let Some(mut item) = item {
        item.cancel();
    }
    if let Some(mut inv) = inv {
        inv.cancel();
    }
    if let Some(mut g) = g {
        g.cancel(&mut p);
    }
    if let Some(mut f) = f {
        f.cancel(&mut p);
    }
    if let Some(mut u) = u {
        u.cancel(&mut p);
    }
    if let Some(mut m) = m {
        m.cancel(&mut p);
    }
    if let Some(mut a) = a {
        a.ground.cancel(&mut p);
    }
    p.request_posture(desired);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn add(p: &mut player::Player) {
        let start = p.clips.len();
        for i in 0..24 {
            let mut clip = Clip {
                name: String::new(),
                fps: 1.0,
                frames: p.clips[0].frames.clone(),
            };
            clip.name = format!("stance-{i}");
            clip.fps = 10.0;
            clip.frames = vec![clip.frames[0].clone(); 21];
            if [2, 3, 4, 5].contains(&i) {
                let distance = match i {
                    2 => 100.0,
                    3 => 50.0,
                    4 => 200.0,
                    _ => 100.0,
                };
                for (frame, pose) in clip.frames.iter_mut().enumerate() {
                    pose.root = Vec3::Y * distance * frame as f32 / 20.0;
                }
            }
            p.clips.push(clip);
        }
        let mut transitions = [[None; 3]; 3];
        let mut next = start + 6;
        for (from, row) in transitions.iter_mut().enumerate() {
            for (to, index) in row.iter_mut().enumerate() {
                if from != to {
                    *index = Some(next);
                    next += 1;
                }
            }
        }
        p.postures = Some(Clips {
            idle: [0, start, start + 1],
            walk: [1, start + 2, start + 3],
            run: [2, start + 4, start + 5],
            rifle_move: [1, 2],
            transition: transitions,
            pistol: [
                (start + 12, start + 13),
                (start + 14, start + 15),
                (start + 16, start + 17),
            ],
            rifle: [
                (start + 18, start + 19),
                (start + 20, start + 21),
                (start + 20, start + 21),
            ],
            pickup: [(4, 0.6), (start + 22, 1.0), (start + 23, 0.2)],
            source: serde_json::Value::Null,
        });
    }
    #[test]
    fn every_pair_uses_its_transition_and_keeps_ground_position() {
        for from in [Posture::Stand, Posture::Crouch, Posture::Prone] {
            for to in [Posture::Stand, Posture::Crouch, Posture::Prone] {
                let mut p = player::tests::player();
                add(&mut p);
                p.posture = from;
                let before = p.position;
                assert_eq!(p.request_posture(to), from != to);
                if from != to {
                    let index =
                        p.postures.as_ref().unwrap().transition[from.index()][to.index()].unwrap();
                    p.step(&ButtonInput::default(), 1.0, &player::tests::grid());
                    assert_eq!(p.playback.selected, index);
                    assert_eq!(p.posture, from);
                    p.step(&ButtonInput::default(), 1.0, &player::tests::grid());
                }
                assert_eq!(p.position, before);
                assert_eq!(p.posture, to);
                assert!(!p.changing_posture());
            }
        }
    }
    #[test]
    fn queued_movement_waits_for_transition_then_uses_source_crouch_speed() {
        let mut p = player::tests::player();
        add(&mut p);
        let grid = player::tests::grid();
        let start = p.position;
        assert!(p.request_posture(Posture::Crouch));
        assert!(p.order(&grid, Vec3::new(0.5, 0.0, -2.0), false));
        p.step(&ButtonInput::default(), 2.0, &grid);
        assert_eq!(p.position, start);
        p.step(&ButtonInput::default(), 1.0, &grid);
        assert!((p.position.z - start.z + 0.5).abs() < 1e-5);
        assert_eq!(p.playback.selected, p.postures.as_ref().unwrap().walk[1]);
    }
    #[test]
    fn prone_fast_crawl_is_faster_and_arrival_does_not_overshoot() {
        let grid = player::tests::grid();
        let mut distances = Vec::new();
        for running in [false, true] {
            let mut p = player::tests::player();
            add(&mut p);
            p.posture = Posture::Prone;
            let start = p.position;
            p.order(&grid, Vec3::new(0.5, 0.0, -2.0), running);
            p.step(&ButtonInput::default(), 1.0, &grid);
            distances.push(start.z - p.position.z);
            p.step(&ButtonInput::default(), 20.0, &grid);
            assert_eq!(p.position.z, -2.0);
            assert!(p.destination.is_none());
        }
        assert!((distances[1] - distances[0] * 2.0).abs() < 1e-5);
    }
    #[test]
    fn pause_invalid_delta_selection_and_home_preserve_or_reset_posture() {
        let mut p = player::tests::player();
        add(&mut p);
        let grid = player::tests::grid();
        p.selected = false;
        assert!(!p.request_posture(Posture::Crouch));
        p.selected = true;
        p.request_posture(Posture::Crouch);
        p.playback.paused = true;
        p.step(&ButtonInput::default(), 1.0, &grid);
        assert_eq!(p.posture_transition.unwrap().1, 0.0);
        p.playback.paused = false;
        p.step(&ButtonInput::default(), f32::NAN, &grid);
        assert_eq!(p.posture_transition.unwrap().1, 0.0);
        p.step(&ButtonInput::default(), 2.0, &grid);
        assert_eq!(p.posture, Posture::Crouch);
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Home);
        p.step(&keys, 0.0, &grid);
        assert_eq!(p.posture, Posture::Stand);
        assert!(!p.changing_posture());
        assert!(!p.rifle_equipped);
    }
    #[test]
    fn s_c_p_input_respects_inventory_pause_and_ctrl_p() {
        let mut p = player::tests::player();
        add(&mut p);
        let mut app = App::new();
        app.insert_resource(p)
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ControlLeft);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();
        assert!(!app.world().resource::<player::Player>().changing_posture());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyC);
        app.update();
        assert_eq!(
            app.world()
                .resource::<player::Player>()
                .posture_transition
                .unwrap()
                .2,
            Posture::Crouch
        );
        app.world_mut()
            .resource_mut::<player::Player>()
            .posture_transition = None;
        let mut inv = inventory::tests::inventory();
        inv.open = true;
        app.insert_resource(inv);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyP);
        app.update();
        assert!(!app.world().resource::<player::Player>().changing_posture());
    }
    #[test]
    fn pickup_selects_posture_callback_and_returns_to_same_idle() {
        for (posture, callback) in [
            (Posture::Stand, 0.6),
            (Posture::Crouch, 1.0),
            (Posture::Prone, 0.2),
        ] {
            let (_, mut p, _, _) = pickup::tests::fixture();
            add(&mut p);
            p.posture = posture;
            p.begin_pickup();
            assert_eq!(p.playback.selected, p.pickup_clip());
            assert_eq!(p.pickup_callback(99.0), callback);
            p.finish_pickup();
            assert_eq!(p.playback.selected, p.idle_clip());
            assert_eq!(p.posture, posture);
        }
    }
}
