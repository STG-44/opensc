//! Джек: исходная внешность, разговор, присоединение и отдельное движение.
//! Снаряжение Джона остаётся в его ресурсах; у Джека пока нет предметных действий.
use crate::{
    gate::Gate,
    inventory::Inventory,
    map_scene::{MapScene, WORLD_SCALE},
    motion::Clip,
    pickup::Pickup,
    player,
    stance::Posture,
    tutorial::Tutorial,
    ui_controls::ButtonVisualState,
    vehicle::Vehicle,
};
use bevy::{
    audio::{AudioSink, AudioSinkPlayback},
    prelude::*,
};
use shadow_formats::{
    AssetCatalog, CampaignActionType as Action, CampaignEventType as Event, MissionDefinition,
    TerrainGrid, config_lines,
};
use std::{error::Error, io::Cursor};

#[derive(Resource)]
pub struct Party {
    pub jack: player::Player,
    pub recruited: bool,
    pub encountered: bool,
    pub speech_count: u32,
    pub pending_audio: bool,
    pub sound_handle: Option<Handle<AudioSource>>,
    pub test_cursor: Option<Vec2>,
    pub consumed_input: bool,
    pub source: serde_json::Value,
    range: f32,
    voice: Vec<u8>,
    voice_duration: f64,
    speech_started: Option<f64>,
    converse_clip: usize,
    history: crate::orders::ClickHistory,
}
#[derive(Component)]
pub struct Root;
#[derive(Component)]
pub struct Bone(usize);
#[derive(Component)]
pub struct Marker(bool);
#[derive(Component)]
pub struct Speech;
#[derive(Component)]
pub struct Select(pub u8);
#[derive(Component)]
pub struct JackStatus;
impl Party {
    pub fn prepare(
        catalog: &AssetCatalog,
        scene: &MapScene,
        mission: &MissionDefinition,
    ) -> Result<Self, Box<dyn Error>> {
        let responses: Vec<_> = mission
            .responses
            .iter()
            .filter(|r| crate::tutorial::rescue_response(r))
            .collect();
        let [response] = responses.as_slice() else {
            return Err("нужен один поддержанный обработчик спасения Jack".into());
        };
        let prisoners: Vec<_> = mission
            .players
            .iter()
            .filter(|p| !p.multiplayer && p.kind == "LOCAL_SCRIPTED" && p.name == "AI_Prisoner")
            .collect();
        let [prisoner] = prisoners.as_slice() else {
            return Err("нужен один AI_Prisoner обучения".into());
        };
        let actors: Vec<_> = scene
            .actors
            .iter()
            .filter(|a| a.name.eq_ignore_ascii_case("Jack") && a.id[0] == prisoner.id)
            .collect();
        let [actor] = actors.as_slice() else {
            return Err("нужен один исходный Jack в MTF".into());
        };
        let mut jack = player::prepare(catalog, &scene.grid, actor)?;
        crate::stance::prepare(catalog, &mut jack)?;
        jack.selected = false;
        let rig = jack.rig.as_ref().unwrap();
        let seq = config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
        let start = seq
            .iter()
            .position(|l| l.words == ["BEGIN_SEQUENCE", "MSEQ_STAND_CONVERSE_1"])
            .ok_or("нет последовательности разговора")?;
        let end = seq[start + 1..]
            .iter()
            .position(|l| l.words[0] == "END_SEQUENCE")
            .ok_or("нет конца разговора")?
            + start
            + 1;
        let [motion] = &seq[start + 1..end] else {
            return Err("неподдержанная последовательность разговора".into());
        };
        if motion.words.len() != 3 || motion.words[0] != "MOTION" || motion.words[2] != "LOOP" {
            return Err("неподдержанный режим разговора".into());
        }
        let motion_path = format!("motions/{}.bmf", motion.words[1].to_ascii_lowercase());
        let converse = Clip::new(
            &shadow_formats::BmfMotion::from_bytes(&catalog.read(&motion_path)?)?,
            &rig.bones,
        )?;
        let converse_clip = jack.clips.len();
        jack.clips.push(converse);
        let lines = config_lines(&catalog.read("config/perception_var_defs.txt")?)?;
        let ranges: Vec<_> = lines
            .iter()
            .filter(|l| {
                l.words.len() == 3 && l.words[0] == "float" && l.words[1] == "oli_converse_range"
            })
            .collect();
        let [range_line] = ranges.as_slice() else {
            return Err("нужен один oli_converse_range".into());
        };
        let source_range = range_line.words[2].parse::<f32>()?;
        if !source_range.is_finite() || !(1.0..=5000.0).contains(&source_range) {
            return Err("неверный радиус разговора".into());
        }
        let conversations: Vec<_> = mission
            .conversations
            .iter()
            .filter(|c| c.id.eq_ignore_ascii_case("beecher_reporting_for_duty"))
            .collect();
        let [conversation] = conversations.as_slice() else {
            return Err("нужен один разговор с Джеком".into());
        };
        let [element] = conversation.elements.as_slice() else {
            return Err("пока поддержана одна реплика Джека".into());
        };
        if !conversation.extra.is_empty()
            || element.words.len() != 3
            || !element.words[1].eq_ignore_ascii_case(&jack.name)
        {
            return Err("неподдержанный участник/режим разговора".into());
        }
        let (profile_path, profile) = player::profile_for(catalog, &actor.name)?;
        let directory = crate::inventory::field(&profile.properties, "SOUND_DIR")?;
        let speech = shadow_formats::normalize_path(&format!(
            "sounds/speech/{directory}/{}",
            element.words[2]
        ))?;
        let voice = catalog.read(&speech)?;
        crate::inventory::validate_sound(&voice)?;
        let reader = hound::WavReader::new(Cursor::new(&voice))?;
        let voice_duration = f64::from(reader.duration()) / f64::from(reader.spec().sample_rate);
        let rig = jack.rig.as_ref().unwrap();
        let source = serde_json::json!({"actor":jack.source,"profile":profile_path,"profile_origin":catalog.source_label(&profile_path)?,"manager":prisoner.name,"manager_id":prisoner.id,"parts":rig.parts.len(),"bones":rig.bones.len(),"body_height_metres":(jack.camera_target().y-jack.position.y)*2.0,"conversation":"beecher_reporting_for_duty","speech":speech,"speech_origin":catalog.source_label(&speech)?,"speech_duration_seconds":voice_duration,"conversation_motion":jack.clips[converse_clip].name,"perception_profile":"config/perception_var_defs.txt","converse_range_source_units":source_range,"response_line":response.line,"declared_delays_ms":response.actions.iter().map(|a|a.delay_ms).collect::<Vec<_>>(),"effective_delays_ms":response.actions.iter().map(|a|if a.kind==Action::ShowContextHelp {a.delay_ms.unwrap_or(0)}else{0}).collect::<Vec<_>>(),"delay_semantics":"verified EXE 0x4431e0, 0x449505, 0x449611, 0x449781, 0x449588: action-specific; no cumulative delay","entry_rule_prototype":"sixth objective enabled, John on foot, within converse radius and unobstructed walk segment","inventory_actions_supported":false,"passenger_supported":false});
        Ok(Self {
            jack,
            recruited: false,
            encountered: false,
            speech_count: 0,
            pending_audio: false,
            sound_handle: None,
            test_cursor: None,
            consumed_input: false,
            source,
            range: source_range * WORLD_SCALE,
            voice,
            voice_duration,
            speech_started: None,
            converse_clip,
            history: crate::orders::ClickHistory::default(),
        })
    }
    pub fn select(&mut self, which: u8, john: &mut player::Player) {
        if which != 1 && !self.recruited {
            return;
        }
        john.selected = which == 1 || which == 3;
        self.jack.selected = self.recruited && (which == 2 || which == 3);
        if !john.selected {
            john.follow_camera = false;
        }
        if !self.jack.selected {
            self.jack.follow_camera = false;
        }
        self.history.0 = None;
    }
    pub fn box_select(&mut self, mask: u8, john: &mut player::Player) {
        john.selected = mask & 1 != 0;
        self.jack.selected = self.recruited && mask & 2 != 0;
        if !john.selected {
            john.follow_camera = false;
        }
        if !self.jack.selected {
            self.jack.follow_camera = false;
        }
        self.history.0 = None;
    }
    pub fn toggle_member(&mut self, which: u8, john: &mut player::Player) {
        if !self.recruited {
            return;
        }
        if which == 1 {
            john.selected = !john.selected;
            if !john.selected {
                john.follow_camera = false;
            }
        }
        if which == 2 {
            self.jack.selected = !self.jack.selected;
            if !self.jack.selected {
                self.jack.follow_camera = false;
            }
        }
        self.history.0 = None;
    }
    pub fn order_group(
        &mut self,
        john: &player::Player,
        point: Vec3,
        running: bool,
        grid: &TerrainGrid,
    ) {
        if self.recruited && john.selected && self.jack.selected {
            let offset = (self.jack.position - john.position)
                .with_y(0.0)
                .normalize_or_zero()
                * 1.0;
            let point = if player::surface(grid, point + offset).is_some() {
                point + offset
            } else {
                point
            };
            self.jack.order(grid, point, running);
        }
    }
    pub fn encounter(
        &mut self,
        john: &mut player::Player,
        t: &mut Tutorial,
        on_foot: bool,
        clear: bool,
    ) -> bool {
        if self.encountered
            || john.playback.paused
            || t.pausing()
            || !on_foot
            || !clear
            || !t.objectives.contains("obj_rescue_beecher")
            || (john.position - self.jack.position).length() > self.range
        {
            return false;
        }
        self.encountered = true;
        john.destination = None;
        let direction = self.jack.position - john.position;
        john.heading = (-direction.x).atan2(-direction.z);
        self.jack.heading = direction.x.atan2(direction.z);
        t.fire(Event::ObjectInConverseRange, &self.jack.name);
        t.tick(0.0, false);
        true
    }
    pub fn receive(&mut self, t: &mut Tutorial) {
        while let Some(action) = t.external.pop_front() {
            match action.kind {
                Action::TriggerConversation
                    if action.id.eq_ignore_ascii_case("beecher_reporting_for_duty") =>
                {
                    self.speech_count += 1;
                    self.speech_started = Some(t.clock);
                    self.pending_audio = true;
                    self.jack.begin_action(self.converse_clip);
                }
                Action::GiveLocalControl if action.id.eq_ignore_ascii_case(&self.jack.name) => {
                    self.recruited = true
                }
                _ => unreachable!("only validated rescue actions enter the outbox"),
            }
        }
        if self
            .speech_started
            .is_some_and(|start| t.clock - start >= self.voice_duration)
        {
            self.speech_started = None;
            if self
                .jack
                .action
                .is_some_and(|(clip, _)| clip == self.converse_clip)
            {
                self.jack.finish_pickup();
            }
        }
    }
    pub fn restart(&mut self, grid: &TerrainGrid) {
        self.recruited = false;
        self.encountered = false;
        self.speech_count = 0;
        self.pending_audio = false;
        self.speech_started = None;
        self.history.0 = None;
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Home);
        self.jack.step(&keys, 0.0, grid);
        self.jack.selected = false;
        self.jack.follow_camera = false;
    }
    pub fn report(&self, john: &player::Player) -> serde_json::Value {
        serde_json::json!({"recruited":self.recruited,"encountered":self.encountered,"speech_count":self.speech_count,"speaking":self.speech_started.is_some(),"selection":{"John":john.selected,"Jack":self.jack.selected},"jack":{"name":self.jack.name,"position_metres":self.jack.position.to_array(),"heading_radians":self.jack.heading,"destination_metres":self.jack.destination.map(|p|p.to_array()),"posture":self.jack.posture.name(),"follow_camera":self.jack.follow_camera,"motion":self.jack.clips[self.jack.playback.selected].name,"motion_seconds":self.jack.playback.seconds},"source":self.source})
    }
}
pub fn john_controls(john: Option<Res<player::Player>>, keys: Res<ButtonInput<KeyCode>>) -> bool {
    john.is_none_or(|p| p.selected) || keys.just_pressed(KeyCode::Home)
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    party: Option<ResMut<Party>>,
    john: Option<ResMut<player::Player>>,
    t: Option<Res<Tutorial>>,
    inv: Option<Res<Inventory>>,
    preview: Res<crate::training::Preview>,
    vehicle: Option<Res<Vehicle>>,
    view: (
        Single<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
        Single<&Window>,
    ),
    buttons: Query<(&Select, &ButtonVisualState)>,
    mut orbit: ResMut<crate::training::Orbit>,
    geometry: Option<Res<Pickup>>,
    medical: Option<Res<crate::medical::Medical>>,
    pointer: Option<Res<crate::selection::Selection>>,
    hud: Option<Res<crate::hud::Hud>>,
) {
    let (Some(mut party), Some(mut john), Some(t), Some(inv), Some(grid), Some(geometry)) =
        (party, john, t, inv, &preview.grid, geometry)
    else {
        return;
    };
    party.consumed_input = false;
    if t.active.is_some()
        || t.consumed_input
        || inv.open
        || inv.consumed_input
        || preview.screenshot.is_some()
    {
        return;
    }
    if keys.just_pressed(KeyCode::Home) {
        party.restart(grid);
        john.selected = true;
        return;
    }
    if john.playback.paused {
        return;
    }
    if party.recruited && (keys.just_pressed(KeyCode::F9) || keys.just_pressed(KeyCode::F10)) {
        let which = if john.selected { 2 } else { 1 };
        party.select(which, &mut john);
    }
    let click = mouse.just_pressed(MouseButton::Left);
    let selection = if keys.just_pressed(KeyCode::Digit1) {
        Some(1)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(2)
    } else if keys.just_pressed(KeyCode::F11) {
        Some(3)
    } else if click {
        buttons
            .iter()
            .find(|(_, b)| **b == ButtonVisualState::Pressed)
            .map(|(s, _)| s.0)
    } else {
        None
    };
    let additive = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if let Some(which) = selection {
        if click && additive && which != 3 {
            party.toggle_member(which, &mut john);
        } else {
            party.select(which, &mut john);
        }
        party.consumed_input = click;
    }
    let (camera, window) = view;
    let (camera, tr) = *camera;
    let cursor = pointer
        .as_ref()
        .and_then(|s| s.click_cursor)
        .or(party.test_cursor)
        .or_else(|| window.cursor_position());
    if click
        && !party.consumed_input
        && party.recruited
        && cursor.is_some_and(|cursor| {
            cursor.x >= 0.0
                && !pointer.as_ref().is_some_and(|s| s.world_blocked)
                && crate::training::actor_clicked(camera, tr, cursor, &party.jack)
                && geometry.clear_line(tr.translation(), party.jack.camera_target())
        })
    {
        if additive {
            party.toggle_member(2, &mut john);
        } else {
            party.select(2, &mut john);
        }
        party.consumed_input = true;
    }
    if click
        && !party.consumed_input
        && party.recruited
        && !medical.as_ref().is_some_and(|m| m.armed)
        && !vehicle.as_ref().is_some_and(|v| v.occupied)
        && cursor.is_some_and(|cursor| {
            cursor.x >= 0.0
                && !pointer.as_ref().is_some_and(|s| s.world_blocked)
                && crate::training::actor_clicked(camera, tr, cursor, &john)
                && geometry.clear_line(tr.translation(), john.camera_target())
        })
    {
        if additive {
            party.toggle_member(1, &mut john);
        } else {
            party.select(1, &mut john);
        }
        party.consumed_input = true;
    }
    if mouse.just_pressed(MouseButton::Right)
        && !pointer.as_ref().is_some_and(|s| s.world_blocked)
        && party.jack.selected
    {
        let alone = !john.selected;
        party.jack.selected = false;
        party.jack.follow_camera = false;
        party.consumed_input = alone;
    }
    if party.jack.selected {
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
        if let Some(desired) = desired {
            party.jack.request_posture(desired);
        }
        if !john.selected && keys.just_pressed(KeyCode::KeyT) {
            party.jack.follow_camera = !party.jack.follow_camera;
            if party.jack.follow_camera {
                orbit.target = party.jack.camera_target();
            }
        }
    }
    if !john.selected
        && party.jack.selected
        && click
        && !party.consumed_input
        && let Some(cursor) = cursor
        && cursor.x >= 0.0
        && !pointer.as_ref().is_some_and(|s| s.world_blocked)
    {
        if !vehicle.as_ref().is_some_and(|v| v.occupied)
            && crate::training::actor_clicked(camera, tr, cursor, &john)
            && geometry.clear_line(tr.translation(), john.camera_target())
        {
            party.select(1, &mut john);
        } else if let Ok(ray) = camera.viewport_to_world(tr, cursor)
            && let Some(point) =
                crate::orders::terrain_hit(grid, ray.origin, ray.direction.as_vec3())
        {
            let running = party.history.register(time.elapsed_secs_f64(), cursor);
            party.jack.order(grid, point, running);
        }
        party.consumed_input = true;
    }
}
#[allow(clippy::too_many_arguments)]
pub fn tick(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    party: Option<ResMut<Party>>,
    john: Option<ResMut<player::Player>>,
    t: Option<ResMut<Tutorial>>,
    inv: Option<Res<Inventory>>,
    vehicle: Option<Res<Vehicle>>,
    preview: Res<crate::training::Preview>,
    geometry: Option<Res<Pickup>>,
    fence: Option<ResMut<crate::fence::Fence>>,
    gate: Option<ResMut<Gate>>,
    mut orbit: ResMut<crate::training::Orbit>,
) {
    let (
        Some(mut party),
        Some(mut john),
        Some(mut t),
        Some(inv),
        Some(vehicle),
        Some(grid),
        Some(geometry),
        Some(mut fence),
        Some(mut gate),
    ) = (
        party,
        john,
        t,
        inv,
        vehicle,
        &preview.grid,
        geometry,
        fence,
        gate,
    )
    else {
        return;
    };
    let previous = party.jack.position;
    party.jack.playback.paused = john.playback.paused || t.pausing() || inv.open;
    let mut input = keys.clone();
    for key in [
        KeyCode::Digit1,
        KeyCode::F11,
        KeyCode::KeyT,
        KeyCode::Escape,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::KeyP,
        KeyCode::Home,
    ] {
        input.reset(key);
    }
    party.jack.step(&input, time.delta_secs(), grid);
    fence.guard_movement(&mut party.jack, previous);
    gate.guard_movement(&mut party.jack, previous);
    if party.jack.follow_camera {
        orbit.target += party.jack.position - previous;
    }
    if !inv.open && !john.playback.paused && !t.pausing() {
        let clear = fence
            .movement_hit(john.position, party.jack.position)
            .is_none()
            && gate
                .movement_hit(john.position, party.jack.position)
                .is_none()
            && geometry.clear_line(john.camera_target(), party.jack.camera_target());
        party.encounter(&mut john, &mut t, !vehicle.occupied, clear);
        party.receive(&mut t);
    }
}
#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut commands: Commands,
    party: Option<ResMut<Party>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sounds: ResMut<Assets<AudioSource>>,
    mut fonts: ResMut<Assets<Font>>,
    camera: Single<Entity, With<crate::training::MapCamera>>,
) {
    let Some(mut party) = party else { return };
    let jack = &mut party.jack;
    let rig = jack.rig.take().expect("Jack setup once");
    let root = commands
        .spawn((
            player::SceneEntity,
            Root,
            Name::new(jack.display_name.clone()),
            jack.transform(),
            Visibility::default(),
        ))
        .id();
    let bones: Vec<_> = rig
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| {
            commands
                .spawn((
                    Bone(i),
                    Name::new(b.name.clone()),
                    jack.playback.pose.bones[i],
                    Visibility::default(),
                ))
                .id()
        })
        .collect();
    for (i, b) in rig.bones.iter().enumerate() {
        commands
            .entity(bones[i])
            .insert(ChildOf(b.parent.map(|p| bones[p]).unwrap_or(root)));
    }
    let textures: std::collections::BTreeMap<_, _> = rig
        .images
        .into_iter()
        .map(|(n, t)| (n, (images.add(t.image), t.alpha_mode)))
        .collect();
    for part in rig.parts {
        commands.spawn((
            Name::new(part.name),
            Mesh3d(meshes.add(part.mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: Some(textures[&part.texture].0.clone()),
                alpha_mode: textures[&part.texture].1,
                cull_mode: None,
                perceptual_roughness: 0.95,
                ..default()
            })),
            Transform::default(),
            ChildOf(bones[part.bone]),
        ));
    }
    let ring = meshes.add(Torus::new(0.48, 0.52));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.75, 1.0),
        unlit: true,
        ..default()
    });
    for selected in [true, false] {
        commands.spawn((
            player::SceneEntity,
            Marker(selected),
            Mesh3d(ring.clone()),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
    party.sound_handle = Some(sounds.add(AudioSource {
        bytes: party.voice.clone().into(),
    }));
    let font: Handle<Font> = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    commands.spawn((
        player::SceneEntity,
        JackStatus,
        crate::hud::Diagnostic,
        UiTargetCamera(*camera),
        GlobalZIndex(12),
        Text::default(),
        TextFont {
            font: font.into(),
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.65, 0.85, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            left: px(25),
            bottom: px(25),
            width: px(285),
            display: Display::None,
            ..default()
        },
    ));
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn visuals(
    party: Option<Res<Party>>,
    john: Option<Res<player::Player>>,
    t: Option<Res<Tutorial>>,
    inv: Option<Res<Inventory>>,
    v: Option<Res<Vehicle>>,
    mut roots: Query<&mut Transform, With<Root>>,
    mut bones: Query<(&Bone, &mut Transform), Without<Root>>,
    mut markers: Query<(&Marker, &mut Transform, &mut Visibility), (Without<Root>, Without<Bone>)>,
    mut jack_status: Query<(&mut Node, &mut Text), (With<JackStatus>, Without<player::Status>)>,
    mut john_status: Query<&mut Node, (With<player::Status>, Without<JackStatus>)>,
) {
    let (Some(party), Some(john), Some(_t), Some(_inv), Some(_v)) = (party, john, t, inv, v) else {
        return;
    };
    for mut root in &mut roots {
        *root = party.jack.transform();
    }
    for (b, mut tr) in &mut bones {
        *tr = party.jack.playback.pose.bones[b.0];
    }
    for (m, mut tr, mut visible) in &mut markers {
        let point = if m.0 {
            party.jack.selected.then_some(party.jack.position)
        } else {
            party.jack.destination
        };
        if let Some(point) = point {
            tr.translation = point + Vec3::Y * 0.03;
            *visible = Visibility::Visible;
        } else {
            *visible = Visibility::Hidden;
        }
    }
    let only_jack = party.jack.selected && !john.selected;
    for mut node in &mut john_status {
        node.display = if only_jack {
            Display::None
        } else {
            Display::Flex
        };
    }
    for (mut node, mut text) in &mut jack_status {
        node.display = if only_jack {
            Display::Flex
        } else {
            Display::None
        };
        **text = format!(
            "JACK BEECHER\nБез оружия\nСтойка: {} · S/C/P\nПозиция: {:.2}, {:.2}, {:.2} м\nT — следить · H — остановить\n1 — вернуться к Джону",
            party.jack.posture.name(),
            party.jack.position.x,
            party.jack.position.y,
            party.jack.position.z
        );
    }
}
pub fn audio(
    mut commands: Commands,
    party: Option<ResMut<Party>>,
    john: Option<Res<player::Player>>,
    t: Option<Res<Tutorial>>,
    inv: Option<Res<Inventory>>,
    sinks: Query<&AudioSink, With<Speech>>,
    entities: Query<Entity, With<Speech>>,
) {
    let (Some(mut party), Some(john), Some(t), Some(inv)) = (party, john, t, inv) else {
        return;
    };
    let paused = john.playback.paused || t.pausing() || inv.open;
    if !party.encountered {
        for e in &entities {
            commands.entity(e).despawn();
        }
    }
    for sink in &sinks {
        if paused {
            sink.pause();
        } else {
            sink.play();
        }
    }
    if party.pending_audio && !paused {
        party.pending_audio = false;
        if let Some(sound) = &party.sound_handle {
            commands.spawn((
                player::SceneEntity,
                Speech,
                AudioPlayer::new(sound.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}
#[allow(dead_code)] // Главное приложение с меню; витрина использует audio().
pub fn pause_audio(sinks: Query<&AudioSink, With<Speech>>) {
    for sink in &sinks {
        sink.pause();
    }
}

#[derive(Resource)]
pub struct NativeSmoke {
    path: std::path::PathBuf,
    screenshot: Option<std::path::PathBuf>,
    stage: u32,
    frames: u32,
    total: u32,
    last: Option<u32>,
    john: Vec3,
    jack: Vec3,
    goal: Vec3,
    clock: f64,
    checks: Vec<serde_json::Value>,
    before_reset: Option<serde_json::Value>,
}
impl NativeSmoke {
    pub fn new(path: std::path::PathBuf, screenshot: Option<std::path::PathBuf>) -> Self {
        Self {
            path,
            screenshot,
            stage: 0,
            frames: 0,
            total: 0,
            last: None,
            john: Vec3::ZERO,
            jack: Vec3::ZERO,
            goal: Vec3::ZERO,
            clock: 0.0,
            checks: Vec::new(),
            before_reset: None,
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn native_smoke(
    mut commands: Commands,
    s: Option<ResMut<NativeSmoke>>,
    party: Option<ResMut<Party>>,
    john: Option<ResMut<player::Player>>,
    t: Option<Res<Tutorial>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    resources: (
        Option<Res<Inventory>>,
        Option<Res<crate::armor::Armor>>,
        Option<Res<crate::medical::Medical>>,
        Option<ResMut<Vehicle>>,
        Option<Res<Gate>>,
        Option<Res<crate::fence::Fence>>,
    ),
    view: (
        Single<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
        Single<&Window>,
        Res<crate::training::Preview>,
        Option<Res<Pickup>>,
    ),
    mut orbit: ResMut<crate::training::Orbit>,
    mut buttons: Query<(&Select, &mut ButtonVisualState)>,
    roots: Query<&Transform, With<Root>>,
    speech: Query<Entity, With<Speech>>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut s), Some(mut party), Some(mut john), Some(t)) = (s, party, john, t) else {
        return;
    };
    let (Some(inv), Some(armor), Some(medical), Some(mut vehicle), Some(gate), Some(fence)) =
        resources
    else {
        return;
    };
    let (camera, window, preview, Some(geometry)) = view else {
        return;
    };
    let (camera, tr) = *camera;
    let grid = preview.grid.as_ref().unwrap();
    s.total += 1;
    s.frames += 1;
    if s.last != Some(s.stage) {
        eprintln!(
            "jack smoke stage {}: recruited={} John={:?} Jack={:?} help={:?}",
            s.stage, party.recruited, john.position, party.jack.position, t.active
        );
        s.last = Some(s.stage);
        s.frames = 0;
    }
    assert!(
        s.total < 1800,
        "jack native timeout stage {}: {}",
        s.stage,
        party.report(&john)
    );
    keys.reset_all();
    mouse.reset_all();
    party.test_cursor = None;
    for (_, mut state) in &mut buttons {
        *state = ButtonVisualState::None;
    }
    if s.total < 20 {
        return;
    }
    if t.active.is_some() {
        keys.press(KeyCode::Enter);
        return;
    }
    let click_world =
        |point: Vec3, party: &mut Party, mouse: &mut ButtonInput<MouseButton>| -> bool {
            if let Ok(cursor) = camera.world_to_viewport(tr, point)
                && cursor.x > 315.0
            {
                party.test_cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                true
            } else {
                false
            }
        };
    let equipment_ok = |john: &player::Player| {
        assert!(armor.worn);
        assert_eq!(inv.weapon(john), Some(crate::inventory::Weapon::Uzi));
        assert_eq!(inv.uzi_rounds, 14);
        assert_eq!(inv.rounds, 9);
        assert_eq!(medical.uses, 1);
    };
    match s.stage {
        0 => {
            john.playback.paused = false;
            assert_eq!(
                t.completed.len(),
                5,
                "native Jack checkpoint must include Hummer boarding"
            );
            if vehicle.occupied {
                vehicle.arm_exit();
                for i in 0..16 {
                    let angle = i as f32 * std::f32::consts::TAU / 16.0;
                    let point =
                        vehicle.position + Vec3::new(angle.cos() * 3.8, 0.0, angle.sin() * 3.8);
                    if vehicle.exit(point, &mut john, grid, Some(fence.id)) {
                        break;
                    }
                }
            }
            assert!(!vehicle.occupied, "cannot leave Hummer at checkpoint");
            john.position = gate.sides[0] + (gate.sides[0] - gate.sides[1]).normalize() * 1.0;
            john.position.y = player::surface(grid, john.position).unwrap();
            john.finish_pickup();
            john.selected = true;
            john.follow_camera = false;
            orbit.target = gate.center.lerp(party.jack.camera_target(), 0.45);
            orbit.yaw = std::f32::consts::PI;
            orbit.pitch = 0.8;
            orbit.distance = 17.0;
            assert!(!party.recruited && !party.encountered);
            assert_eq!(roots.iter().count(), 1);
            equipment_ok(&john);
            s.checks.push(serde_json::json!({"original_jack_spawn_and_model":true,"fifth_objective_preserved":true,"setup":"after real numeric Hummer boarding and validated exit, John placed outside original gate; road driving not exercised"}));
            keys.press(KeyCode::KeyS);
            s.stage = 1;
        }
        1 if s.frames > 4 && !john.changing_posture() => {
            if camera
                .world_to_viewport(tr, gate.center)
                .is_ok_and(|c| gate.clicked(camera, tr, c, &geometry))
                && click_world(gate.center, &mut party, &mut mouse)
            {
                s.stage = 2;
            }
        }
        2 if gate.open && !gate.moving() => {
            let point = party.jack.camera_target();
            if click_world(point, &mut party, &mut mouse) {
                s.stage = 3;
            }
        }
        3 if party.recruited => {
            assert!(t.completed.contains("obj_rescue_beecher"));
            assert_eq!(t.completed.len(), 6);
            assert_eq!(party.speech_count, 1);
            assert_eq!(
                t.events
                    .iter()
                    .filter(|e| e.as_str() == "CET_OBJECT_ENCOUNTERED_WITHIN_CONVERSE_RANGE:Jack")
                    .count(),
                1
            );
            assert!(
                party.pending_audio || !speech.is_empty(),
                "speech player was not created"
            );
            equipment_ok(&john);
            s.checks.push(serde_json::json!({"mouse_approach_rescue":true,"sixth_objective":true,"original_speech_player":true,"control_transferred":true}));
            s.john = john.position;
            s.jack = party.jack.position;
            s.clock = t.clock;
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::KeyP);
            s.stage = 4;
        }
        4 if s.frames >= 12 => {
            assert!(john.playback.paused);
            assert!(
                (john.position - s.john).length() < 0.001
                    && (party.jack.position - s.jack).length() < 0.001
            );
            assert!((t.clock - s.clock).abs() < 1e-6);
            s.checks
                .push(serde_json::json!({"pause_freezes_both_and_tutorial":true}));
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::KeyP);
            s.stage = 5;
        }
        5 if t.shown.iter().any(|h| h == "help_combat_18") && !t.consumed_input => {
            keys.press(KeyCode::Digit2);
            s.stage = 6;
        }
        6 if s.frames > 2 => {
            assert!(!john.selected && party.jack.selected);
            s.john = john.position;
            s.jack = party.jack.position;
            s.goal = party.jack.position + Vec3::new(1.3, 0.0, 0.5);
            s.goal.y = player::surface(grid, s.goal).unwrap();
            keys.press(KeyCode::Space);
            s.stage = 7;
        }
        7 if s.frames > 2 => {
            assert!(!inv.open, "Jack opened John's inventory");
            let goal = s.goal;
            if click_world(goal, &mut party, &mut mouse) {
                s.stage = 8;
            }
        }
        8 if party.jack.destination.is_none() && s.frames > 8 => {
            assert!(
                (party.jack.position - s.goal).with_y(0.0).length() < 0.25,
                "Jack movement failed: {:?}",
                party.jack.position
            );
            assert!((john.position - s.john).length() < 0.001);
            equipment_ok(&john);
            s.checks.push(serde_json::json!({"independent_jack_mouse_movement":true,"john_equipment_preserved":true,"jack_cannot_open_john_inventory":true}));
            for (button, mut state) in &mut buttons {
                if button.0 == 1 {
                    *state = ButtonVisualState::Pressed;
                }
            }
            mouse.press(MouseButton::Left);
            s.stage = 9;
        }
        9 if s.frames > 2 => {
            assert!(john.selected && !party.jack.selected);
            s.checks
                .push(serde_json::json!({"john_selection_button":true}));
            keys.press(KeyCode::F11);
            s.stage = 10;
        }
        10 if s.frames > 2 => {
            assert!(john.selected && party.jack.selected);
            s.john = john.position;
            s.jack = party.jack.position;
            let point = Vec3::new(gate.center.x - 1.6, 0.0, party.jack.position.z - 1.4);
            if click_world(
                Vec3::new(point.x, player::surface(grid, point).unwrap(), point.z),
                &mut party,
                &mut mouse,
            ) {
                s.stage = 11;
            }
        }
        11 if s.frames > 12 && john.destination.is_none() && party.jack.destination.is_none() => {
            assert!(
                john.position.distance(s.john) > 0.2 && party.jack.position.distance(s.jack) > 0.2,
                "group did not move"
            );
            assert_eq!(party.speech_count, 1);
            equipment_ok(&john);
            s.checks
                .push(serde_json::json!({"group_mouse_order":true,"conversation_once":true}));
            s.before_reset = Some(party.report(&john));
            if let Some(path) = &s.screenshot {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
            }
            s.stage = 12;
        }
        12 if s.frames > 12 => {
            keys.press(KeyCode::Home);
            s.stage = 13;
        }
        13 if s.frames > 8 => {
            assert!(
                !party.recruited
                    && !party.encountered
                    && party.speech_count == 0
                    && !party.jack.selected
                    && party.jack.destination.is_none()
            );
            assert!(john.selected);
            assert!(!gate.open);
            assert_eq!(t.completed.len(), 0);
            assert!(speech.is_empty());
            let src = party.source["actor"]["source_position"].as_array().unwrap();
            let expected = Vec3::new(
                src[0].as_f64().unwrap() as f32,
                src[2].as_f64().unwrap() as f32,
                -src[1].as_f64().unwrap() as f32,
            ) * WORLD_SCALE;
            assert!((party.jack.position - expected).with_y(0.0).length() < 0.001);
            s.checks.push(serde_json::json!({"home_restores_both_and_gate":true,"speech_entities_cleared":true}));
            let report = serde_json::json!({"status":"passed","window":[window.width(),window.height()],"checks":s.checks,"before_reset":s.before_reset,"after_reset":party.report(&john),"actual_os_mouse_input":false,"input":"Bevy ButtonInput, viewport rays and project button state; physical pointer not exercised"});
            std::fs::write(
                &s.path,
                serde_json::to_string_pretty(&report).unwrap() + "\n",
            )
            .unwrap();
            s.stage = 14;
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn tutorial() -> Tutorial {
        let mission=MissionDefinition::from_bytes(b"SPECIFY_MTF test.mtf\nDEFINE_OBJECTIVE true obj_rescue_beecher \"Jack\"\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_ENCOUNTERED_WITHIN_CONVERSE_RANGE\nCER_FOR Jack\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_TRIGGER_CONVERSE\nCER_BODY_IDENTIFIER beecher_reporting_for_duty\nCER_BODY_MS_DELAY 200\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_NOTIFY_OBJECTIVE_COMPLETE_NO_AUDIO\nCER_BODY_IDENTIFIER obj_rescue_beecher\nCER_BODY_MS_DELAY 10000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_GIVE_LNS_PLAYER_OBJECT_CONTROL\nCER_BODY_IDENTIFIER Jack\nCER_BODY_MS_DELAY 2000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER help_combat_18\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\nCONVERSE_SEQUENCE_DEFINITION\nCONVERSE_IDENTIFIER beecher_reporting_for_duty\nCONVERSE_ELEMENT Jack Atg_02.wav\n").unwrap();
        let help = shadow_formats::HelpDefinitions::from_bytes(
            b"HELP_DEF\nHELP_ID help_combat_18\nHELP_CONTEXT\nHELP_BODY \"Two mercenaries\"\n",
        )
        .unwrap();
        let mut t = Tutorial::new(mission, help).unwrap();
        t.events.clear();
        t.objectives.insert("obj_rescue_beecher".into());
        t
    }
    pub(crate) fn fixture() -> (Party, player::Player, Tutorial, TerrainGrid) {
        let john = player::tests::player();
        let mut jack = player::tests::player();
        jack.name = "Jack".into();
        jack.position += Vec3::X * 0.5;
        jack.selected = false;
        let party = Party {
            jack,
            recruited: false,
            encountered: false,
            speech_count: 0,
            pending_audio: false,
            sound_handle: None,
            test_cursor: None,
            consumed_input: false,
            source: serde_json::Value::Null,
            range: 4.0,
            voice: vec![],
            voice_duration: 1.2,
            speech_started: None,
            converse_clip: 3,
            history: crate::orders::ClickHistory::default(),
        };
        (party, john, tutorial(), player::tests::grid())
    }
    #[test]
    fn rescue_requires_enabled_objective_foot_range_and_clear_reach() {
        let (mut p, mut john, mut t, _) = fixture();
        assert!(!p.encounter(&mut john, &mut t, false, true));
        assert!(!p.encounter(&mut john, &mut t, true, false));
        john.playback.paused = true;
        assert!(!p.encounter(&mut john, &mut t, true, true));
        john.playback.paused = false;
        t.active = Some("help_combat_18".into());
        assert!(!p.encounter(&mut john, &mut t, true, true));
        t.active = None;
        t.objectives.clear();
        assert!(!p.encounter(&mut john, &mut t, true, true));
        t.objectives.insert("obj_rescue_beecher".into());
        p.jack.position += Vec3::X * 8.0;
        assert!(!p.encounter(&mut john, &mut t, true, true));
        assert!(!p.encountered && t.events.is_empty());
    }
    #[test]
    fn original_dispatch_recruits_and_completes_immediately_then_shows_help() {
        let (mut p, mut john, mut t, _) = fixture();
        assert!(p.encounter(&mut john, &mut t, true, true));
        assert!(t.completed.contains("obj_rescue_beecher"));
        assert!(t.active.is_none());
        assert_eq!(t.external.len(), 2);
        p.receive(&mut t);
        assert!(p.recruited && p.pending_audio);
        assert_eq!(p.speech_count, 1);
        t.tick(0.999, false);
        assert!(t.active.is_none());
        t.tick(0.001001, false);
        assert_eq!(t.active.as_deref(), Some("help_combat_18"));
    }
    #[test]
    fn repeated_encounter_and_removed_response_do_not_repeat_speech() {
        let (mut p, mut john, mut t, _) = fixture();
        assert!(p.encounter(&mut john, &mut t, true, true));
        p.receive(&mut t);
        assert!(!p.encounter(&mut john, &mut t, true, true));
        t.fire(Event::ObjectInConverseRange, "Jack");
        t.tick(0.0, false);
        p.receive(&mut t);
        assert_eq!(p.speech_count, 1);
        assert!(t.external.is_empty());
    }
    #[test]
    fn prisoner_cannot_be_selected_before_transfer() {
        let (mut p, mut john, _, _) = fixture();
        for which in [2, 3] {
            p.select(which, &mut john);
            assert!(john.selected && !p.jack.selected);
        }
    }
    #[test]
    fn switching_selection_preserves_existing_john_orders() {
        let (mut p, mut john, _, _) = fixture();
        p.recruited = true;
        let target = Vec3::new(1.0, 2.0, -1.0);
        john.destination = Some(target);
        john.follow_camera = true;
        p.select(2, &mut john);
        assert!(!john.selected && p.jack.selected && !john.follow_camera);
        assert_eq!(john.destination, Some(target));
        p.select(3, &mut john);
        assert!(john.selected && p.jack.selected);
        p.select(1, &mut john);
        assert!(john.selected && !p.jack.selected);
    }
    #[test]
    fn jack_root_motion_moves_only_jack() {
        let (mut p, mut john, _, grid) = fixture();
        p.recruited = true;
        p.select(2, &mut john);
        let before = john.position;
        let jack_before = p.jack.position;
        assert!(p.jack.order(&grid, Vec3::new(1.0, 0.0, -1.5), false));
        p.jack.step(&ButtonInput::default(), 0.3, &grid);
        assert!((p.jack.position - jack_before).with_y(0.0).length() > 0.1);
        assert_eq!(john.position, before);
    }
    #[test]
    fn group_order_uses_distinct_targets_on_the_same_terrain() {
        let (mut p, mut john, _, grid) = fixture();
        p.recruited = true;
        p.select(3, &mut john);
        let target = Vec3::new(0.8, 0.0, -0.8);
        assert!(john.order(&grid, target, false));
        p.order_group(&john, target, false, &grid);
        let a = john.destination.unwrap();
        let b = p.jack.destination.unwrap();
        assert!((a - b).with_y(0.0).length() > 0.5);
        assert_eq!(b.y, player::surface(&grid, b).unwrap());
    }
    #[test]
    fn group_order_does_not_include_an_unselected_jack() {
        let (mut p, john, _, grid) = fixture();
        p.recruited = true;
        p.order_group(&john, Vec3::new(1.0, 0.0, -1.0), false, &grid);
        assert!(p.jack.destination.is_none());
    }
    #[test]
    fn speech_ends_without_erasing_a_later_movement_order() {
        let (mut p, mut john, mut t, grid) = fixture();
        p.encounter(&mut john, &mut t, true, true);
        p.receive(&mut t);
        assert!(p.jack.action.is_some());
        p.jack.order(&grid, Vec3::new(1.0, 0.0, -1.0), false);
        let target = p.jack.destination;
        t.clock += 2.0;
        p.receive(&mut t);
        assert!(p.speech_started.is_none());
        assert_eq!(p.jack.destination, target);
    }
    #[test]
    fn home_clears_recruitment_voice_selection_and_motion() {
        let (mut p, mut john, mut t, grid) = fixture();
        p.encounter(&mut john, &mut t, true, true);
        p.receive(&mut t);
        p.select(2, &mut john);
        p.jack.position = Vec3::ZERO;
        p.restart(&grid);
        assert!(!p.recruited && !p.encountered && !p.jack.selected && !p.pending_audio);
        assert_eq!(p.speech_count, 0);
        assert!(p.jack.action.is_none() && p.jack.destination.is_none());
        assert_eq!(p.jack.position, player::tests::player().position);
    }
    #[test]
    fn tutorial_restart_drops_unconsumed_external_actions() {
        let mut t = tutorial();
        t.fire(Event::ObjectInConverseRange, "Jack");
        t.tick(0.0, false);
        assert_eq!(t.external.len(), 2);
        t.restart();
        assert!(t.external.is_empty() && t.completed.is_empty());
    }
    #[test]
    fn unsupported_conversation_identifier_keeps_entire_response_disabled() {
        // Неподдержанный ID должен отключить весь обработчик.
        let bytes=b"SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_ENCOUNTERED_WITHIN_CONVERSE_RANGE\nCER_FOR Jack\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_TRIGGER_CONVERSE\nCER_BODY_IDENTIFIER unknown_conversation\nCONVERSE_SEQUENCE_DEFINITION\nCONVERSE_IDENTIFIER unknown_conversation\nCONVERSE_ELEMENT Jack Atg_02.wav\n";
        let mut t = Tutorial::new(
            MissionDefinition::from_bytes(bytes).unwrap(),
            shadow_formats::HelpDefinitions::from_bytes(
                b"HELP_DEF\nHELP_ID dummy\nHELP_CONTEXT\nHELP_BODY \"x\"\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(t.unsupported.len(), 1);
        t.fire(Event::ObjectInConverseRange, "Jack");
        t.tick(0.0, false);
        assert!(t.external.is_empty());
    }
    #[test]
    fn additive_selection_toggles_only_the_clicked_member() {
        let (mut party, mut john, _, _) = fixture();
        party.recruited = true;
        party.toggle_member(2, &mut john);
        assert!(john.selected && party.jack.selected);
        party.toggle_member(1, &mut john);
        assert!(!john.selected && party.jack.selected);
        party.toggle_member(2, &mut john);
        assert!(!john.selected && !party.jack.selected);
    }
}
