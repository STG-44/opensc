//! Учебная аптечка: предмет MTF, здоровье Джона, выбор пациента и конечное действие.
//! Числа ловушки и лечения временные: точная формула оригинала ещё не восстановлена.
use crate::{
    inventory::{self, Inventory, Slot},
    map_scene::MapScene,
    motion::Clip,
    pickup::{MapObject, Pickup},
    player,
    smg::Uzi,
    stance::Posture,
    tutorial::Tutorial,
    ui_controls::ButtonVisualState,
};
use bevy::prelude::*;
use shadow_formats::{AssetCatalog, BmfMotion, CampaignEventType as Event, config_lines};
use std::error::Error;

#[derive(Resource)]
pub struct Medical {
    pub ground: Uzi,
    pub maximum: f32,
    pub health: f32,
    pub skill: u32,
    pub uses: u32,
    pub initial_uses: u32,
    pub armed: bool,
    pub healing: Option<usize>,
    pub treatments: u32,
    pub trap_triggered: bool,
    pub source: serde_json::Value,
    pub(crate) test_cursor: Option<Vec2>,
    clips: [usize; 3],
    sound: Vec<u8>,
    pub(crate) sound_handle: Option<Handle<AudioSource>>,
    pending_audio: bool,
}
impl Medical {
    pub fn prepare(
        catalog: &AssetCatalog,
        scene: &MapScene,
        p: &mut player::Player,
    ) -> Result<Self, Box<dyn Error>> {
        let path = "config/inventory_object_profiles/tools.txt";
        let lines = config_lines(&catalog.read(path)?)?;
        let kit = inventory::block(&lines, "OBJECT_DEF", "med_kit", "OBJECT_DEF")?;
        if inventory::field(kit, "OBJECT_NAME")? != "Medical Kit"
            || inventory::field(kit, "OBJECT_EQUIP_LOC")? != "EQL_NONE"
            || inventory::field(kit, "OBJECT_AUTO_EQUIP")? != "FALSE"
        {
            return Err("неподдержанный вариант аптечки".into());
        }
        let uses = inventory::field(kit, "TOOL_USES")?.parse::<u32>()?;
        if !(1..=10000).contains(&uses) {
            return Err("неверное TOOL_USES аптечки".into());
        }
        let model = player::model_path(catalog, inventory::field(kit, "OBJECT_MODEL")?)?;
        let ground = Uzi::ground_item(scene, "Medical Kit", &model, false)?;
        let (profile_path, profile) = player::profile_for(catalog, &p.name)?;
        let maximum = inventory::field(&profile.properties, "HEALTH_MAX")?.parse::<f32>()?;
        let skills: Vec<_> = profile
            .properties
            .iter()
            .filter(|l| l.words.len() == 3 && l.words[0] == "SKILL" && l.words[1] == "Medic")
            .collect();
        let [skill] = skills.as_slice() else {
            return Err("нужен один навык Medic".into());
        };
        let skill = skill.words[2].parse::<u32>()?;
        if !maximum.is_finite() || !(1.0..=100000.0).contains(&maximum) || skill > 100 {
            return Err("неверное здоровье или Medic".into());
        }
        let seqs = config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
        let bones = &p
            .rig
            .as_ref()
            .ok_or("лечение готовится до создания сцены")?
            .bones;
        let mut clips = [0; 3];
        let mut motions = Vec::new();
        for (index, posture) in ["STAND", "CROUCH", "PRONE"].iter().enumerate() {
            let name = format!("MSEQ_{posture}_USE_MEDPAC");
            let body = inventory::block(&seqs, "BEGIN_SEQUENCE", &name, "END_SEQUENCE")?;
            // Поддерживаем именно исходную пару: лечебный клип, NOTIFY_END, затем idle.
            if body.len() != 4
                || body[1].words.len() != 2
                || body[1].words[0] != "MOTION"
                || body[2].words != ["CALLBACK", "NOTIFY_END"]
                || body[3].words.len() != 2
                || body[3].words[0] != "MOTION"
            {
                return Err(format!("неподдержанная последовательность {name}").into());
            }
            let finish = if index == 0 {
                "bipedal_stand"
            } else {
                "bipedal_crouch"
            };
            if body[3].words[1] != finish {
                return Err("неподдержанная завершающая стойка лечения".into());
            }
            let motion = format!("motions/{}.bmf", body[1].words[1].to_ascii_lowercase());
            let cog_state = if index == 0 {
                "MSEQ_STATE_STAND"
            } else {
                "MSEQ_STATE_CROUCH"
            };
            let cogs: Vec<_> = seqs
                .iter()
                .filter(|l| {
                    l.words.len() == 5
                        && l.words[0] == "DEFAULT_COG_POSITION"
                        && l.words[1] == cog_state
                })
                .collect();
            let [cog] = cogs.as_slice() else {
                return Err("нет COG лечения".into());
            };
            let cog = Vec3::new(
                cog.words[2].parse()?,
                cog.words[3].parse()?,
                cog.words[4].parse()?,
            );
            if !cog.is_finite() {
                return Err("нечисловой COG лечения".into());
            }
            let clip = Clip::with_default_cog(
                &BmfMotion::from_bytes(&catalog.read(&motion)?)?,
                bones,
                Some(cog),
            )?;
            motions.push(serde_json::json!({"sequence":name,"motion":motion,"duration_seconds":clip.duration(),"completion":"NOTIFY_END","finish_posture":finish,"origin":catalog.source_label(&motion)?}));
            clips[index] = p.clips.len();
            p.clips.push(clip);
        }
        let audio = config_lines(&catalog.read("config/audio_defs.txt")?)?;
        let sample = inventory::block(
            &audio,
            "AUDIO_DEF",
            inventory::field(kit, "OBJECT_USE_SAMPLE")?,
            "AUDIO_DEF",
        )?;
        let filename = format!(
            "{}.wav",
            inventory::field(sample, "FILE")?.to_ascii_lowercase()
        );
        let matches: Vec<_> = catalog
            .paths()
            .filter(|p| p.rsplit('/').next() == Some(filename.as_str()))
            .collect();
        let [sound_path] = matches.as_slice() else {
            return Err("нужен один звук аптечки".into());
        };
        let sound = catalog.read(sound_path)?;
        inventory::validate_sound(&sound)?;
        let source = serde_json::json!({"profile":path,"profile_origin":catalog.source_label(path)?,"model":model,"tool_uses":uses,"equip_location":"EQL_NONE","auto_equip":false,
            "character_profile":profile_path,"health_max":maximum,"medic_skill":skill,"motions":motions,"sound":sound_path,
            "trap_evidence":"help_combat_13: The Med Kit was booby trapped","trap_damage_prototype":maximum*0.5,"trap_damage_original_verified":false,
            "healing_per_use_prototype":skill.max(1),"healing_formula_original_verified":false,"prone_sequence_returns_crouch":true,
            "other_patients":false,"enemy_damage":false});
        Ok(Self {
            ground,
            maximum,
            health: maximum,
            skill,
            uses,
            initial_uses: uses,
            armed: false,
            healing: None,
            treatments: 0,
            trap_triggered: false,
            source,
            test_cursor: None,
            clips,
            sound,
            sound_handle: None,
            pending_audio: false,
        })
    }
    pub fn cancel(&mut self, p: &mut player::Player) {
        self.ground.cancel(p);
        self.armed = false;
        self.pending_audio = false;
        if let Some(clip) = self.healing.take()
            && p.action.is_some_and(|a| a.0 == clip)
        {
            p.finish_pickup();
        }
    }
    pub fn restart(&mut self, p: &mut player::Player) {
        self.cancel(p);
        self.ground.collected = false;
        self.health = self.maximum;
        self.uses = self.initial_uses;
        self.treatments = 0;
        self.trap_triggered = false;
        self.pending_audio = false;
    }
    pub fn arm(&mut self) -> bool {
        self.armed = self.ground.collected
            && self.uses > 0
            && self.health < self.maximum
            && self.health > 0.0
            && self.healing.is_none();
        self.armed
    }
    pub fn treat(&mut self, p: &mut player::Player) -> bool {
        if !self.armed
            || p.playback.paused
            || p.changing_posture()
            || p.pickup_seconds.is_some()
            || p.action.is_some()
            || self.uses == 0
            || self.health <= 0.0
            || self.health >= self.maximum
        {
            return false;
        }
        self.armed = false;
        self.ground.ordered = false;
        let clip = self.clips[p.posture.index()];
        // В исходной PRONE-последовательности используется crouch_heal и завершающий crouch.
        if p.posture == Posture::Prone {
            p.posture = Posture::Crouch;
        }
        p.destination = None;
        p.begin_action(clip);
        self.healing = Some(clip);
        self.pending_audio = true;
        true
    }
    pub fn update(
        &mut self,
        p: &mut player::Player,
        inv: &Inventory,
        t: &mut Tutorial,
        callback: f32,
    ) {
        if inv.open || p.playback.paused || p.changing_posture() || t.pausing() {
            return;
        }
        if let Some(clip) = self.healing {
            if let Some((active, seconds)) = p.action {
                if active != clip {
                    self.healing = None;
                    return;
                }
                if seconds >= p.clips[clip].duration() {
                    self.health = (self.health + self.skill.max(1) as f32).min(self.maximum);
                    self.uses -= 1;
                    self.treatments += 1;
                    self.healing = None;
                    p.finish_pickup();
                }
            } else {
                self.healing = None;
            }
            return;
        }
        if !self.ground.ordered {
            return;
        }
        let distance = Vec2::new(
            p.position.x - self.ground.position.x,
            p.position.z - self.ground.position.z,
        )
        .length();
        if !self.ground.collected && p.pickup_seconds.is_none() && distance <= 0.5 {
            p.destination = None;
            p.begin_pickup();
        }
        if !self.ground.collected
            && p.pickup_seconds.is_none()
            && p.destination.is_none()
            && distance > 0.5
        {
            self.ground.cancel(p);
            return;
        }
        if let Some(seconds) = p.pickup_seconds {
            if !self.ground.collected && seconds >= p.pickup_callback(callback) && distance <= 0.5 {
                self.ground.collected = true;
                self.health = (self.health - self.maximum * 0.5).max(1.0);
                self.trap_triggered = true;
                t.fire(Event::ObjectPickedUp, "Medical Kit");
                t.tick(0.0, false);
            }
            if seconds >= p.pickup_duration() {
                p.finish_pickup();
                self.ground.ordered = false;
            }
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"pickup":self.ground.report(),"health":self.health,"maximum":self.maximum,"uses":self.uses,
            "armed":self.armed,"healing":self.healing,"treatments":self.treatments,"trap_triggered":self.trap_triggered,"source":self.source})
    }
    pub fn suppress_audio(&mut self) {
        self.pending_audio = false;
    }
}
#[derive(Component)]
pub struct Ring;
#[derive(Component)]
pub struct HealthLabel;
#[allow(clippy::too_many_arguments)]
pub fn hud(
    mut commands: Commands,
    medical: Option<Res<Medical>>,
    mut labels: Query<&mut Text, With<HealthLabel>>,
    camera: Single<Entity, With<crate::training::MapCamera>>,
    mut fonts: ResMut<Assets<Font>>,
    windows: Query<(Entity, Option<&bevy::window::CursorIcon>), With<Window>>,
) {
    let Some(m) = medical else {
        return;
    };
    let desired = bevy::window::CursorIcon::System(if m.armed {
        bevy::window::SystemCursorIcon::Crosshair
    } else {
        bevy::window::SystemCursorIcon::Default
    });
    for (window, cursor) in &windows {
        if cursor != Some(&desired) {
            commands.entity(window).insert(desired.clone());
        }
    }
    let text = format!(
        "Джон: {:.0}/{:.0} HP · Аптечка: {}{}",
        m.health,
        m.maximum,
        if m.ground.collected {
            m.uses.to_string()
        } else {
            "—".into()
        },
        if m.armed {
            " · + ЛКМ по Джону, ПКМ отмена"
        } else if m.healing.is_some() {
            " · Лечение…"
        } else {
            ""
        }
    );
    if let Ok(mut label) = labels.single_mut() {
        label.0 = text;
    } else {
        let font = fonts.add(Font::from_bytes(
            include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
        ));
        commands.spawn((
            player::SceneEntity,
            HealthLabel,
            crate::hud::Diagnostic,
            UiTargetCamera(*camera),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                right: px(20),
                bottom: px(20),
                ..default()
            },
            Text::new(text),
            TextFont {
                font: font.into(),
                font_size: FontSize::Px(18.0),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.94, 0.8)),
        ));
    }
}
pub fn setup(
    mut commands: Commands,
    medical: Option<ResMut<Medical>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut sounds: ResMut<Assets<AudioSource>>,
) {
    let Some(mut m) = medical else {
        return;
    };
    m.sound_handle = Some(sounds.add(AudioSource {
        bytes: m.sound.clone().into(),
    }));
    commands.spawn((
        player::SceneEntity,
        Ring,
        Mesh3d(meshes.add(Torus::new(0.22, 0.25))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.25, 0.25),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(m.ground.position + Vec3::Y * 0.04),
        Visibility::default(),
    ));
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    medical: Option<ResMut<Medical>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<ResMut<Inventory>>,
    t: Option<Res<Tutorial>>,
    slots: Query<(&Slot, &ButtonVisualState)>,
    actions: (
        Option<ResMut<crate::combat::Combat>>,
        Option<ResMut<crate::grenade::Grenade>>,
        Option<ResMut<crate::fence::Fence>>,
        Option<ResMut<Uzi>>,
        Option<ResMut<Pickup>>,
        Option<ResMut<crate::armor::Armor>>,
    ),
) {
    let (Some(mut m), Some(mut p), Some(mut inv)) = (medical, p, inv) else {
        return;
    };
    if p.carry_movement.is_some() {
        return;
    }
    if t.is_some_and(|t| t.active.is_some() || t.consumed_input) {
        return;
    }
    if keys.just_pressed(KeyCode::Home) && !inv.open {
        m.restart(&mut p);
        return;
    }
    if keys.just_pressed(KeyCode::KeyH) && p.selected && !inv.open {
        m.cancel(&mut p);
        return;
    }
    if !inv.open {
        return;
    }
    if mouse.just_pressed(MouseButton::Right)
        && slots
            .iter()
            .any(|(s, v)| *s == Slot::Medical && *v != ButtonVisualState::None)
    {
        let (c, g, f, u, item, a) = actions;
        if let Some(mut a) = a {
            a.ground.cancel(&mut p);
        }
        if let Some(mut c) = c {
            c.cancel_all(&mut p);
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
        if let Some(mut item) = item {
            item.cancel();
        }
        m.cancel(&mut p);
        inv.cancel();
        m.arm();
        inv.drag = None;
        inv.consumed_input = true;
    } else if mouse.just_pressed(MouseButton::Right) {
        m.armed = false;
    }
    if mouse.just_pressed(MouseButton::Left) && m.armed {
        inv.consumed_input = true;
        inv.drag = None;
        if slots
            .iter()
            .any(|(s, v)| *s == Slot::Patient && *v != ButtonVisualState::None)
            && m.treat(&mut p)
        {
            inv.open = false;
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync(
    mut commands: Commands,
    medical: Option<ResMut<Medical>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<Res<Inventory>>,
    t: Option<ResMut<Tutorial>>,
    item: Option<Res<Pickup>>,
    mut objects: Query<(&MapObject, &mut Visibility)>,
    mut rings: Query<&mut Visibility, (With<Ring>, Without<MapObject>)>,
) {
    let (Some(mut m), Some(mut p), Some(inv), Some(mut t), Some(item)) = (medical, p, inv, t, item)
    else {
        return;
    };
    m.update(&mut p, &inv, &mut t, item.callback_seconds);
    for (id, mut visible) in &mut objects {
        if id.0 == m.ground.id {
            *visible = if m.ground.collected {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
    for mut visible in &mut rings {
        *visible = if m.ground.collected {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    if m.pending_audio && !p.playback.paused && !inv.open && !t.pausing() {
        m.pending_audio = false;
        if let Some(handle) = &m.sound_handle {
            commands.spawn((
                player::SceneEntity,
                AudioPlayer::new(handle.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        Medical,
        player::Player,
        Inventory,
        Tutorial,
        shadow_formats::TerrainGrid,
    ) {
        let (ground, mut p, inv, _, grid) = crate::smg::tests::fixture();
        let index = p.clips.len();
        p.clips.push(Clip {
            name: "heal".into(),
            fps: 10.0,
            frames: vec![p.clips[0].frames[0].clone(); 21],
        });
        let mission=shadow_formats::MissionDefinition::from_bytes(b"SPECIFY_MTF training.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_PICKED_UP\nCER_FOR \"Medical Kit\"\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER medical\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\n").unwrap();
        let helps = shadow_formats::HelpDefinitions::from_bytes(
            b"HELP_DEF\nHELP_ID medical\nHELP_CONTEXT\nHELP_BODY \"Medical kit\"\n",
        )
        .unwrap();
        let m = Medical {
            ground,
            maximum: 327.0,
            health: 327.0,
            skill: 43,
            uses: 5,
            initial_uses: 5,
            armed: false,
            healing: None,
            treatments: 0,
            trap_triggered: false,
            source: serde_json::Value::Null,
            test_cursor: None,
            clips: [index; 3],
            sound: Vec::new(),
            sound_handle: None,
            pending_audio: false,
        };
        (m, p, inv, Tutorial::new(mission, helps).unwrap(), grid)
    }
    fn injured(m: &mut Medical) {
        m.ground.collected = true;
        m.health = 163.5;
        m.trap_triggered = true;
    }
    #[test]
    fn pickup_triggers_one_wound_and_one_cer_preserving_primary() {
        let (mut m, mut p, inv, mut t, grid) = fixture();
        m.ground.order(&mut p, &grid);
        m.update(&mut p, &inv, &mut t, 0.6);
        p.step(&ButtonInput::default(), 0.59, &grid);
        m.update(&mut p, &inv, &mut t, 0.6);
        assert!(!m.ground.collected);
        assert_eq!(m.health, 327.0);
        p.step(&ButtonInput::default(), 0.02, &grid);
        m.update(&mut p, &inv, &mut t, 0.6);
        for _ in 0..10 {
            m.update(&mut p, &inv, &mut t, 0.6);
        }
        assert!(m.ground.collected && m.trap_triggered);
        assert_eq!(m.health, 163.5);
        assert_eq!(m.uses, 5);
        assert_eq!(inv.weapon(&p), Some(inventory::Weapon::Bda));
        assert_eq!(
            t.report()["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e.as_str() == Some("CET_OBJECT_PICKED_UP:Medical Kit"))
                .count(),
            1
        );
    }
    #[test]
    fn charges_change_only_at_end_and_final_heal_clamps_to_maximum() {
        let (mut m, mut p, inv, mut t, grid) = fixture();
        injured(&mut m);
        for n in 1..=4 {
            assert!(m.arm() && m.treat(&mut p));
            let before = m.health;
            p.step(&ButtonInput::default(), 1.99, &grid);
            m.update(&mut p, &inv, &mut t, 0.6);
            assert_eq!(m.health, before);
            assert_eq!(m.uses, 6 - n);
            p.step(&ButtonInput::default(), 0.02, &grid);
            m.update(&mut p, &inv, &mut t, 0.6);
            m.update(&mut p, &inv, &mut t, 0.6);
            assert_eq!(m.uses, 5 - n);
            assert_eq!(m.treatments, n);
            assert!(p.action.is_none());
        }
        assert_eq!(m.health, 327.0);
        assert!(!m.arm());
        assert_eq!(m.uses, 1);
    }
    #[test]
    fn absent_exhausted_dead_and_full_patients_cannot_arm() {
        let (mut m, _, _, _, _) = fixture();
        m.health = 100.0;
        assert!(!m.arm());
        m.ground.collected = true;
        m.uses = 0;
        assert!(!m.arm());
        m.uses = 5;
        m.health = 0.0;
        assert!(!m.arm());
        m.health = 327.0;
        assert!(!m.arm());
    }
    #[test]
    fn pause_inventory_and_context_help_preserve_unfinished_treatment() {
        let (mut m, mut p, mut inv, mut t, grid) = fixture();
        injured(&mut m);
        m.arm();
        m.treat(&mut p);
        p.playback.paused = true;
        p.step(&ButtonInput::default(), 5.0, &grid);
        m.update(&mut p, &inv, &mut t, 0.6);
        assert_eq!(p.action.unwrap().1, 0.0);
        assert_eq!(m.uses, 5);
        p.playback.paused = false;
        p.step(&ButtonInput::default(), 3.0, &grid);
        inv.open = true;
        m.update(&mut p, &inv, &mut t, 0.6);
        assert_eq!(m.uses, 5);
        inv.open = false;
        t.fire(Event::ObjectPickedUp, "Medical Kit");
        t.tick(1.0, false);
        m.update(&mut p, &inv, &mut t, 0.6);
        assert_eq!(m.uses, 5);
        t.close();
        m.update(&mut p, &inv, &mut t, 0.6);
        assert_eq!(m.uses, 4);
    }
    #[test]
    fn cancellation_and_replaced_action_do_not_consume_charge() {
        let (mut m, mut p, inv, mut t, grid) = fixture();
        injured(&mut m);
        m.arm();
        m.treat(&mut p);
        p.step(&ButtonInput::default(), 1.0, &grid);
        m.cancel(&mut p);
        m.update(&mut p, &inv, &mut t, 0.6);
        assert_eq!((m.health, m.uses, m.treatments), (163.5, 5, 0));
        assert!(p.action.is_none());
        m.arm();
        m.treat(&mut p);
        p.begin_action(0);
        m.update(&mut p, &inv, &mut t, 0.6);
        assert!(m.healing.is_none());
        assert_eq!(m.uses, 5);
        assert!(p.action.is_some());
    }
    #[test]
    fn healing_keeps_primary_and_source_prone_returns_crouch() {
        let (mut m, mut p, inv, mut t, grid) = fixture();
        injured(&mut m);
        p.posture = Posture::Prone;
        m.arm();
        m.treat(&mut p);
        assert_eq!(p.posture, Posture::Crouch);
        p.step(&ButtonInput::default(), 3.0, &grid);
        m.update(&mut p, &inv, &mut t, 0.6);
        assert_eq!(p.posture, Posture::Crouch);
        assert_eq!(inv.weapon(&p), Some(inventory::Weapon::Bda));
    }
    #[test]
    fn reset_restores_ground_health_charges_and_pending_audio() {
        let (mut m, mut p, _, _, _) = fixture();
        injured(&mut m);
        m.arm();
        m.treat(&mut p);
        m.restart(&mut p);
        assert!(
            !m.ground.collected
                && !m.ground.ordered
                && !m.armed
                && m.healing.is_none()
                && !m.pending_audio
                && !m.trap_triggered
        );
        assert_eq!((m.health, m.uses, m.treatments), (327.0, 5, 0));
    }
    #[test]
    fn right_click_kit_then_left_click_patient_routes_inventory_input() {
        let (mut m, p, mut inv, t, _) = fixture();
        injured(&mut m);
        inv.open = true;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(m)
            .insert_resource(p)
            .insert_resource(inv)
            .insert_resource(t)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, input);
        let kit = app
            .world_mut()
            .spawn((Slot::Medical, ButtonVisualState::Hovered))
            .id();
        let patient = app
            .world_mut()
            .spawn((Slot::Patient, ButtonVisualState::None))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();
        assert!(app.world().resource::<Medical>().armed);
        assert!(app.world().resource::<Inventory>().open);
        *app.world_mut().get_mut::<ButtonVisualState>(kit).unwrap() = ButtonVisualState::None;
        *app.world_mut()
            .get_mut::<ButtonVisualState>(patient)
            .unwrap() = ButtonVisualState::Pressed;
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .reset_all();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert!(!app.world().resource::<Inventory>().open);
        assert!(app.world().resource::<Medical>().healing.is_some());
        assert_eq!(app.world().resource::<Medical>().uses, 5);
    }
}

#[derive(Resource)]
pub struct NativeSmoke {
    pub path: std::path::PathBuf,
    pub screenshot: Option<std::path::PathBuf>,
    stage: u32,
    frames: u32,
    checks: Vec<serde_json::Value>,
}
impl NativeSmoke {
    pub fn new(path: std::path::PathBuf, screenshot: Option<std::path::PathBuf>) -> Self {
        Self {
            path,
            screenshot,
            stage: 0,
            frames: 0,
            checks: Vec::new(),
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub fn native_smoke(
    mut commands: Commands,
    smoke: Option<ResMut<NativeSmoke>>,
    mut m: Option<ResMut<Medical>>,
    mut p: Option<ResMut<player::Player>>,
    mut inv: Option<ResMut<Inventory>>,
    t: Option<Res<Tutorial>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    camera: Single<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
    mut orbit: ResMut<crate::training::Orbit>,
    mut slots: Query<(&Slot, &mut ButtonVisualState)>,
    window: Single<&Window>,
    objects: Query<(&MapObject, &Visibility)>,
    mut exit: MessageWriter<AppExit>,
    geometry: Option<Res<Pickup>>,
    mut last_stage: Local<Option<u32>>,
) {
    let (Some(mut s), Some(ref mut m), Some(ref mut p), Some(ref mut inv), Some(t)) =
        (smoke, m.as_mut(), p.as_mut(), inv.as_mut(), t)
    else {
        return;
    };
    s.frames += 1;
    if *last_stage != Some(s.stage) {
        eprintln!(
            "medical smoke stage {} frame {}: collected={} ordered={} health={} uses={} action={:?} destination={:?} inv={}",
            s.stage,
            s.frames,
            m.ground.collected,
            m.ground.ordered,
            m.health,
            m.uses,
            p.action,
            p.destination,
            inv.open
        );
        *last_stage = Some(s.stage);
    }
    assert!(
        s.frames < 1500,
        "medical native smoke timeout at {}",
        s.stage
    );
    keys.reset_all();
    mouse.reset_all();
    if s.frames < 20 {
        return;
    }
    if t.active.is_some() {
        keys.press(KeyCode::Enter);
        return;
    }
    let (camera, transform) = *camera;
    match s.stage {
        0 => {
            p.playback.paused = false;
            p.follow_camera = false;
            orbit.target = m.ground.center;
            s.stage = 1;
        }
        1 => {
            let cursor = camera
                .world_to_viewport(transform, m.ground.center)
                .expect("medical viewport");
            assert!(cursor.x > 315.0, "medical behind sidebar");
            if !geometry
                .as_ref()
                .is_some_and(|item| m.ground.clicked(camera, transform, cursor, item))
            {
                if s.frames.is_multiple_of(20) {
                    eprintln!("medical ground occluded at yaw {}", orbit.yaw);
                }
                orbit.yaw += 0.12;
                return;
            }
            m.test_cursor = Some(cursor);
            mouse.press(MouseButton::Left);
            s.checks
                .push(serde_json::json!({"pickup_by_viewport_mouse":cursor.to_array()}));
            s.stage = 2;
        }
        2 if !m.ground.collected && !m.ground.ordered && p.destination.is_none() => {
            eprintln!("medical click not accepted; retrying after UI consumption");
            s.stage = 1;
        }
        2 if m.ground.collected && !m.ground.ordered && p.pickup_seconds.is_none() => {
            assert_eq!(m.health, m.maximum * 0.5);
            assert_eq!(m.uses, 5);
            assert!(
                objects
                    .iter()
                    .any(|(id, v)| id.0 == m.ground.id && *v == Visibility::Hidden)
            );
            s.checks.push(serde_json::json!({"trap_once":true,"ground_hidden":true,"health_after_pickup":m.health}));
            p.follow_camera = true;
            keys.press(KeyCode::Space);
            s.stage = 3;
        }
        3 if inv.open && slots.iter().any(|(slot, _)| *slot == Slot::Medical) => {
            for (slot, mut visual) in &mut slots {
                *visual = if *slot == Slot::Medical {
                    ButtonVisualState::Hovered
                } else {
                    ButtonVisualState::None
                };
            }
            mouse.press(MouseButton::Right);
            s.stage = 4;
        }
        4 => {
            assert!(m.armed && inv.open, "right click arms treatment");
            if let Some(path) = &s.screenshot {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(
                        path.with_file_name(format!(
                            "{}-inventory.png",
                            path.file_stem().unwrap().to_string_lossy()
                        )),
                    ));
            }
            s.stage = 5;
        }
        5 => {
            for (slot, mut visual) in &mut slots {
                *visual = if *slot == Slot::Patient {
                    ButtonVisualState::Pressed
                } else {
                    ButtonVisualState::None
                };
            }
            mouse.press(MouseButton::Left);
            s.stage = 6;
        }
        6 if m.healing.is_none() && p.action.is_none() => {
            assert!(!inv.open && !m.armed);
            s.checks.push(
                serde_json::json!({"treatment":m.treatments,"health":m.health,"uses":m.uses}),
            );
            if m.health < m.maximum {
                keys.press(KeyCode::Space);
                s.stage = 3;
            } else {
                s.stage = 7;
            }
        }
        7 => {
            assert_eq!(m.treatments, 4);
            assert_eq!(m.uses, 1);
            assert_eq!(inv.weapon(p), Some(inventory::Weapon::Uzi));
            assert_eq!(inv.uzi_rounds, 14);
            assert_eq!(inv.rounds, 9);
            assert_eq!(
                t.report()["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|e| e.as_str() == Some("CET_OBJECT_PICKED_UP:Medical Kit"))
                    .count(),
                1
            );
            keys.press(KeyCode::Space);
            s.stage = 8;
        }
        8 if inv.open && slots.iter().any(|(slot, _)| *slot == Slot::Medical) => {
            for (slot, mut visual) in &mut slots {
                *visual = if *slot == Slot::Medical {
                    ButtonVisualState::Hovered
                } else {
                    ButtonVisualState::None
                };
            }
            mouse.press(MouseButton::Right);
            s.stage = 9;
        }
        9 => {
            assert!(!m.armed);
            assert_eq!(m.uses, 1);
            s.checks.push(
                serde_json::json!({"full_health_preserves_charge":true,"primary_preserved":"UZI"}),
            );
            if let Some(path) = &s.screenshot {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
            }
            s.stage = 10;
        }
        10 => {
            let report = serde_json::json!({"status":"passed","window":[window.resolution.width(),window.resolution.height()],"medical":m.report(),"inventory":inv.report(p),"tutorial":t.report(),"checks":s.checks,"native_input":"viewport mouse for ground pickup; project ButtonVisualState and mouse buttons for inventory","physical_pointer_tested":false});
            std::fs::write(
                &s.path,
                serde_json::to_string_pretty(&report).unwrap() + "\n",
            )
            .unwrap();
            s.stage = 11;
            s.frames = 0;
        }
        11 if s.frames >= 8 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

#[allow(dead_code)] // Используется переходами экранов shadow-game.
pub fn reset_cursor(mut commands: Commands, windows: Query<Entity, With<Window>>) {
    for window in &windows {
        commands
            .entity(window)
            .insert(bevy::window::CursorIcon::System(
                bevy::window::SystemCursorIcon::Default,
            ));
    }
}
