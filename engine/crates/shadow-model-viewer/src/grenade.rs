//! Дополнительный слот, бросок по callback, траектория и запал.
//! Формула взрыва и физика пока приближённые; параметры ресурсов сохраняются в отчёте.
use crate::{
    combat,
    inventory::{self, Inventory},
    map_scene::WORLD_SCALE,
    orders,
    pickup::Pickup,
    player, rig, scene,
    tutorial::Tutorial,
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use shadow_formats::{AssetCatalog, CampaignEventType as Event, TerrainGrid, config_lines};
use std::error::Error;

pub struct Definition {
    pub model: String,
    pub range: f32,
    pub fuse: f32,
    pub radius: f32,
    pub damage: f32,
    pub sound: Vec<u8>,
    pub source: serde_json::Value,
}
fn positive(value: &str, max: f32) -> Result<f32, Box<dyn Error>> {
    let n = value.parse::<f32>()?;
    if !n.is_finite() || n <= 0.0 || n > max {
        return Err("неверный положительный параметр гранаты".into());
    }
    Ok(n)
}
pub fn definition(catalog: &AssetCatalog) -> Result<Definition, Box<dyn Error>> {
    let path = "config/inventory_object_profiles/grenades.txt";
    let lines = config_lines(&catalog.read(path)?)?;
    let profile = inventory::block(&lines, "OBJECT_NAME", "Frag Grenade", "OBJECT_DEF")?;
    if inventory::field(profile, "GRENADE_TYPE")? != "grenade_frag"
        || inventory::field(profile, "OBJECT_EQUIP_LOC")? != "EQL_RIGHT_HAND"
        || inventory::field(profile, "OBJECT_CARRY_STYLE")? != "CSTYLE_ONE_HAND"
        || inventory::field(profile, "OBJECT_AUTO_EQUIP")? != "FALSE"
    {
        return Err("профиль Frag Grenade пока не поддерживается".into());
    }
    let model = player::model_path(catalog, inventory::field(profile, "OBJECT_MODEL")?)?;
    let fuse_ms = positive(inventory::field(profile, "FUSE_DURATION")?, 60000.0)?;
    let range = positive(inventory::field(profile, "MAX_RANGE")?, 1_000_000.0)?;
    let shell_path = "config/shell_damage_models.txt";
    let shells = config_lines(&catalog.read(shell_path)?)?;
    let shell = inventory::block(
        &shells,
        "SHELL_DAMAGE_MODEL",
        "GRENADE_FRAG",
        "SHELL_DAMAGE_MODEL",
    )?;
    let radius = positive(inventory::field(shell, "BLAST_RADIUS")?, 1_000_000.0)?;
    let force = positive(inventory::field(shell, "BLAST_FORCE")?, 1_000_000.0)?;
    let audio = config_lines(&catalog.read("config/audio_defs.txt")?)?;
    let sample = inventory::block(&audio, "AUDIO_DEF", "AUDIO_EXP_GRN_FRAG", "AUDIO_DEF")?;
    let filename = format!(
        "{}.wav",
        inventory::field(sample, "FILE")?.to_ascii_lowercase()
    );
    let paths: Vec<_> = catalog
        .paths()
        .filter(|p| p.rsplit('/').next() == Some(filename.as_str()))
        .collect();
    let [sound_path] = paths.as_slice() else {
        return Err("не найден один звук взрыва".into());
    };
    let sound = catalog.read(sound_path)?;
    inventory::validate_sound(&sound)?;
    let damage = force * WORLD_SCALE; // Временное преобразование, не формула старого EXE.
    let source = serde_json::json!({"profile":path,"profile_origin":catalog.source_label(path)?,"model":model,"fuse_ms":fuse_ms,"max_range_source_units":range,
        "shell_profile":shell_path,"base_damage":inventory::field(shell,"BASE_DAMAGE")?.parse::<f32>()?,"rand_damage":inventory::field(shell,"RAND_DAMAGE")?.parse::<f32>()?,"blast_force":force,"blast_radius_source_units":radius,
        "prototype_center_damage":damage,"damage_formula_original_verified":false,"ballistics_original_verified":false,"drop_placement_original_verified":false,
        "profile_auto_equip_primary":false,"secondary_auto_assignment_source":"help_combat_6","sound":sound_path,"sound_origin":catalog.source_label(sound_path)?,"explosion_visual_original_verified":false,"player_damage_implemented":false});
    Ok(Definition {
        model,
        range: range * WORLD_SCALE,
        fuse: fuse_ms / 1000.0,
        radius: radius * WORLD_SCALE,
        damage,
        sound,
        source,
    })
}
#[derive(Clone, Debug)]
pub struct Flight {
    pub position: Vec3,
    pub velocity: Vec3,
    pub elapsed: f32,
    pub collisions: u32,
    pub resting: bool,
}
#[derive(Clone)]
struct ThrowOrder {
    target: Vec3,
    guard: Option<usize>,
    forced: bool,
    aiming: bool,
    released: bool,
}
#[derive(Resource)]
pub struct Grenade {
    pub definition: Definition,
    pub count: u32,
    pub secondary: bool,
    pub available: bool,
    pub collected: bool,
    pub pickup_ordered: bool,
    pub position: Option<Vec3>,
    pub center: Option<Vec3>,
    pub drop_transform: Transform,
    pub flight: Option<Flight>,
    pub throws: u32,
    pub explosions: u32,
    pub collisions: u32,
    pub last_blast: Option<Vec3>,
    pub message: String,
    order: Option<ThrowOrder>,
    clip: usize,
    callback: f32,
    model: Option<scene::PreparedModel>,
    triangles: Vec<[Vec3; 3]>,
    local_center: Vec3,
    lower_y: f32,
    flash: Option<(Vec3, f32)>,
    sound_pending: bool,
    pub sound_handle: Option<Handle<AudioSource>>,
}
impl Grenade {
    pub fn prepare(catalog: &AssetCatalog, p: &mut player::Player) -> Result<Self, Box<dyn Error>> {
        let definition = definition(catalog)?;
        let rig = p
            .rig
            .as_mut()
            .ok_or("граната готовится до создания сцены")?;
        let first = rig.parts.len();
        rig::prepare_weapon(catalog, rig, &definition.model, "Frag Grenade")?;
        p.equipment_parts
            .push((definition.model.clone(), first..rig.parts.len()));
        let (clip, frame) = combat::sequence(
            catalog,
            "MSEQ_STAND_THROW_GRENADE",
            Some("MCB_FIRE_WEAPON"),
            &rig.bones,
        )?;
        let callback = frame.unwrap() as f32 / clip.fps;
        let index = p.clips.len();
        p.clips.push(clip);
        let model = scene::prepare(catalog, &definition.model, false)?;
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        let mut triangles = Vec::new();
        for part in &model.parts {
            let Some(VertexAttributeValues::Float32x3(points)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                return Err("нет геометрии гранаты".into());
            };
            let points: Vec<Vec3> = points.iter().map(|p| Vec3::from_array(*p)).collect();
            for &v in &points {
                low = low.min(v);
                high = high.max(v);
            }
            let indices: Vec<_> = part
                .mesh
                .indices()
                .ok_or("нет индексов гранаты")?
                .iter()
                .collect();
            for t in indices.as_chunks::<3>().0 {
                triangles.push([points[t[0]], points[t[1]], points[t[2]]]);
            }
        }
        if !low.is_finite() || !high.is_finite() || triangles.is_empty() {
            return Err("пустая модель гранаты".into());
        }
        let mut result = Self::new(definition, index, callback);
        result.local_center = (low + high) * 0.5;
        result.lower_y = low.y;
        result.triangles = triangles;
        result.model = Some(model);
        result.definition.source["sequence"] = serde_json::json!("MSEQ_STAND_THROW_GRENADE");
        result.definition.source["callback_frame"] = serde_json::json!(frame.unwrap());
        result.definition.source["fps"] = serde_json::json!(p.clips[index].fps);
        Ok(result)
    }
    fn new(definition: Definition, clip: usize, callback: f32) -> Self {
        Self {
            definition,
            count: 0,
            secondary: false,
            available: false,
            collected: false,
            pickup_ordered: false,
            position: None,
            center: None,
            drop_transform: Transform::default(),
            flight: None,
            throws: 0,
            explosions: 0,
            collisions: 0,
            last_blast: None,
            message: String::new(),
            order: None,
            clip,
            callback,
            model: None,
            triangles: Vec::new(),
            local_center: Vec3::ZERO,
            lower_y: 0.0,
            flash: None,
            sound_pending: false,
            sound_handle: None,
        }
    }
    pub fn observe(&mut self, c: &combat::Combat) {
        if c.second.is_some() && !c.guard_at(1).alive() && !self.available {
            self.available = true;
            let guard = &c.guard_at(1).guard;
            let p = guard.position + Vec3::new(0.55, 0.03 - self.lower_y * WORLD_SCALE, 0.2);
            self.position = Some(Vec3::new(p.x, guard.position.y, p.z));
            self.drop_transform = Transform::from_translation(p)
                .with_rotation(Quat::from_rotation_y(guard.heading))
                .with_scale(Vec3::splat(WORLD_SCALE));
            self.center = Some(self.drop_transform.transform_point(self.local_center));
        }
    }
    pub fn restart(&mut self, p: &mut player::Player) {
        self.cancel(p);
        self.count = 0;
        self.secondary = false;
        self.available = false;
        self.collected = false;
        self.position = None;
        self.center = None;
        self.flight = None;
        self.throws = 0;
        self.explosions = 0;
        self.collisions = 0;
        self.last_blast = None;
        self.flash = None;
        self.sound_pending = false;
        self.message.clear();
    }
    pub fn cancel(&mut self, p: &mut player::Player) {
        self.pickup_ordered = false;
        self.order = None;
        if p.action.is_some_and(|(i, _)| i == self.clip) {
            p.action = None;
            p.playback.select(&p.clips, 0);
        }
    }
    pub fn suppress_pending_audio(&mut self) {
        self.sound_pending = false;
    }
    pub fn has_order(&self) -> bool {
        self.pickup_ordered || self.order.is_some()
    }
    pub fn order_pickup(&mut self, p: &mut player::Player, grid: &TerrainGrid) -> bool {
        if !self.available || self.collected || p.playback.paused {
            return false;
        }
        if p.order(grid, self.position.unwrap(), false) {
            self.pickup_ordered = true;
            self.order = None;
            true
        } else {
            false
        }
    }
    pub fn order_throw(
        &mut self,
        target: Vec3,
        guard: Option<usize>,
        forced: bool,
        p: &mut player::Player,
        grid: &TerrainGrid,
    ) -> bool {
        if self.count == 0
            || !self.secondary
            || self.flight.is_some()
            || p.playback.paused
            || !target.is_finite()
        {
            self.message = "Нет гранаты в дополнительном слоте".into();
            return false;
        }
        self.cancel(p);
        if !p.order(grid, target, false) {
            return false;
        }
        self.message.clear();
        self.order = Some(ThrowOrder {
            target,
            guard,
            forced,
            aiming: false,
            released: false,
        });
        true
    }
    pub fn clicked(
        &self,
        camera: &Camera,
        transform: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> bool {
        if !self.available || self.collected {
            return false;
        }
        let center = self.center.unwrap();
        if !geometry.clear_line(transform.translation(), center) {
            return false;
        }
        let Ok(ray) = camera.viewport_to_world(transform, cursor) else {
            return false;
        };
        let world = self.drop_transform.to_matrix();
        self.triangles.iter().any(|t| {
            crate::pickup::ray_triangle(
                ray.origin,
                ray.direction.as_vec3(),
                t.map(|v| world.transform_point3(v)),
            )
            .is_some()
        }) || camera
            .world_to_viewport(transform, center)
            .is_ok_and(|v| v.distance(cursor) <= 14.0)
    }
    fn holding(&self, p: &player::Player) -> bool {
        self.order.as_ref().is_some_and(|o| o.aiming && !o.released)
            && p.action.is_some_and(|(i, _)| i == self.clip)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        p: &mut player::Player,
        c: &mut combat::Combat,
        t: &mut Tutorial,
        item: &Pickup,
        inv: &Inventory,
        grid: &TerrainGrid,
        dt: f32,
        pickup_callback: f32,
    ) {
        self.observe(c);
        if inv.open
            || t.pausing()
            || p.playback.paused
            || p.changing_posture()
            || !dt.is_finite()
            || dt < 0.0
        {
            return;
        }
        if self.pickup_ordered {
            let point = self.position.unwrap();
            let distance = Vec2::new(point.x - p.position.x, point.z - p.position.z).length();
            if !self.collected
                && p.pickup_seconds.is_none()
                && p.destination.is_none()
                && distance > 0.5
            {
                self.pickup_ordered = false;
            }
            if !self.collected && distance <= 0.5 && p.pickup_seconds.is_none() {
                p.destination = None;
                p.begin_pickup();
            }
            if let Some(seconds) = p.pickup_seconds {
                if !self.collected
                    && seconds >= p.pickup_callback(pickup_callback)
                    && distance <= 0.5
                {
                    self.collected = true;
                    self.count = 1;
                    self.secondary = true;
                    t.fire(Event::ObjectPickedUp, "Frag Grenade");
                    t.tick(0.0, false);
                }
                if seconds >= p.pickup_duration() {
                    p.finish_pickup();
                    self.pickup_ordered = false;
                }
            }
        }
        let mut launched = false;
        if let Some(mut order) = self.order.take() {
            if order
                .guard
                .is_some_and(|i| i >= c.guard_count() || !c.guard_at(i).alive())
                && !order.released
            {
                self.cancel(p);
            } else if !order.aiming {
                let delta = order.target - p.position;
                if delta.length() <= self.definition.range
                    && (order.forced
                        || item.clear_line(p.camera_target(), order.target + Vec3::Y * 0.9))
                {
                    p.heading = (-delta.x).atan2(-delta.z);
                    p.begin_action(self.clip);
                    order.aiming = true;
                    self.order = Some(order);
                } else if p.destination.is_none() {
                    self.message = "Нет доступной точки броска".into();
                } else {
                    self.order = Some(order);
                }
            } else if let Some((index, seconds)) = p.action
                && index == self.clip
            {
                if !order.released && seconds >= self.callback && self.count > 0 {
                    let from = p.camera_target() + Vec3::Y * 0.35;
                    let to = order.target + Vec3::Y * 0.04;
                    self.count -= 1;
                    self.secondary = self.count > 0;
                    self.throws += 1;
                    order.released = true;
                    launched = true;
                    self.flight = Some(Flight {
                        position: from,
                        velocity: launch_velocity(from, to, 1.2),
                        elapsed: 0.0,
                        collisions: 0,
                        resting: false,
                    });
                }
                if seconds >= p.clips[self.clip].duration() {
                    p.action = None;
                    p.playback.select(&p.clips, 0);
                } else {
                    self.order = Some(order);
                }
            }
        }
        if let Some((_, age)) = &mut self.flash {
            *age += dt;
            if *age >= 0.6 {
                self.flash = None;
            }
        }
        if !launched && let Some(mut flight) = self.flight.take() {
            let step = dt.min(self.definition.fuse - flight.elapsed).max(0.0);
            advance_flight(&mut flight, step, grid, item);
            if flight.elapsed + 1e-5 >= self.definition.fuse {
                self.collisions += flight.collisions;
                self.explosions += 1;
                self.last_blast = Some(flight.position);
                self.flash = Some((flight.position, 0.0));
                self.sound_pending = true;
                for index in 0..c.guard_count() {
                    let guard = c.guard_at(index);
                    if !guard.alive() {
                        continue;
                    }
                    let point = guard.guard.position + Vec3::Y * 0.7;
                    let distance = point.distance(flight.position);
                    let damage = blast_damage(
                        self.definition.damage,
                        self.definition.radius,
                        distance,
                        item.clear_line(flight.position + Vec3::Y * 0.08, point),
                    );
                    c.blast_damage(index, damage, t);
                }
            } else {
                self.flight = Some(flight);
            }
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"available":self.available,"collected":self.collected,"count":self.count,"secondary_equipped":self.secondary,"pickup_ordered":self.pickup_ordered,"throw_ordered":self.order.is_some(),"throws":self.throws,"explosions":self.explosions,"collisions":self.collisions,"position_metres":self.position.map(|p|p.to_array()),"center_metres":self.center.map(|p|p.to_array()),"last_blast_metres":self.last_blast.map(|p|p.to_array()),"flight":self.flight.as_ref().map(|f|serde_json::json!({"position_metres":f.position.to_array(),"elapsed_seconds":f.elapsed,"collisions":f.collisions})),"message":self.message,"definition":self.definition.source})
    }
}
fn launch_velocity(from: Vec3, to: Vec3, seconds: f32) -> Vec3 {
    (to - from) / seconds + Vec3::Y * (9.81 * seconds * 0.5)
}
fn blast_damage(max: f32, radius: f32, distance: f32, visible: bool) -> f32 {
    if !visible || !distance.is_finite() || radius <= 0.0 || distance >= radius {
        0.0
    } else {
        max * (1.0 - distance / radius).clamp(0.0, 1.0)
    }
}
fn advance_flight(f: &mut Flight, dt: f32, grid: &TerrainGrid, item: &Pickup) {
    let mut remaining = dt;
    while remaining > 1e-6 {
        let step = remaining.min(1.0 / 60.0);
        remaining -= step;
        f.elapsed += step;
        if f.resting {
            continue;
        }
        let next = f.position + f.velocity * step - Vec3::Y * (9.81 * step * step * 0.5);
        f.velocity -= Vec3::Y * (9.81 * step);
        let delta = next - f.position;
        let distance = delta.length();
        let wall = item.segment_hit(f.position, next);
        let ground = if distance > 1e-6 {
            orders::terrain_hit(grid, f.position, delta / distance)
                .filter(|p| p.distance(f.position) <= distance + 1e-5)
                .map(|p| (p.distance(f.position), Vec3::Y))
        } else {
            None
        };
        let hit = match (wall, ground) {
            (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
            (a, b) => a.or(b),
        };
        if let Some((d, normal)) = hit {
            f.collisions += 1;
            f.position += delta.normalize_or_zero() * d + normal * 0.035;
            f.velocity = (f.velocity - 2.0 * f.velocity.dot(normal) * normal) * 0.35;
            if normal.y > 0.7 && f.velocity.length() < 2.0 {
                f.resting = true;
                f.velocity = Vec3::ZERO;
            }
        } else {
            f.position = next;
        }
    }
}
#[derive(Component)]
pub struct Drop;
#[derive(Component)]
pub struct Flying;
#[derive(Component)]
pub struct Flash;
pub fn setup(
    mut commands: Commands,
    g: Option<ResMut<Grenade>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sounds: ResMut<Assets<AudioSource>>,
) {
    let Some(mut g) = g else {
        return;
    };
    let Some(model) = g.model.take() else {
        return;
    };
    g.sound_handle = Some(sounds.add(AudioSource {
        bytes: g.definition.sound.clone().into(),
    }));
    let textures = model
        .images
        .into_iter()
        .map(|(p, t)| (p, (images.add(t.image), t.alpha_mode)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let drop = commands
        .spawn((
            player::SceneEntity,
            Drop,
            Name::new("Dropped Frag Grenade"),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    commands.spawn((
        Mesh3d(meshes.add(Torus::new(23.0, 26.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.8, 0.12),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(Vec3::new(
            g.local_center.x,
            g.lower_y + 3.0,
            g.local_center.z,
        )),
        ChildOf(drop),
    ));
    let flying = commands
        .spawn((
            player::SceneEntity,
            Flying,
            Name::new("Thrown Frag Grenade"),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    for part in model.parts {
        let mesh = meshes.add(part.mesh);
        let material = materials.add(StandardMaterial {
            base_color_texture: part.texture.as_ref().map(|p| textures[p].0.clone()),
            alpha_mode: part.alpha_mode,
            cull_mode: None,
            perceptual_roughness: 0.95,
            ..default()
        });
        for parent in [drop, flying] {
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                ChildOf(parent),
            ));
        }
    }
    commands.spawn((
        player::SceneEntity,
        Flash,
        Mesh3d(meshes.add(Sphere::new(1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.5, 0.05, 0.65),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
}
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    g: Option<ResMut<Grenade>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<Res<Inventory>>,
    t: Option<Res<Tutorial>>,
) {
    if inv.is_some_and(|inv| inv.open || inv.consumed_input)
        || t.is_some_and(|t| t.active.is_some() || t.consumed_input)
    {
        return;
    }
    if let (Some(mut g), Some(mut p)) = (g, p) {
        if keys.just_pressed(KeyCode::Home) {
            g.restart(&mut p);
        } else if keys.just_pressed(KeyCode::KeyH) && p.selected {
            g.cancel(&mut p);
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync(
    mut commands: Commands,
    g: Option<ResMut<Grenade>>,
    p: Option<ResMut<player::Player>>,
    c: Option<ResMut<combat::Combat>>,
    t: Option<ResMut<Tutorial>>,
    item: Option<Res<Pickup>>,
    inv: Option<Res<Inventory>>,
    preview: Res<crate::training::Preview>,
    time: Res<Time>,
    mut drops: Query<
        (&mut Transform, &mut Visibility),
        (With<Drop>, Without<Flying>, Without<Flash>),
    >,
    mut flying: Query<
        (&mut Transform, &mut Visibility),
        (With<Flying>, Without<Drop>, Without<Flash>),
    >,
    mut flashes: Query<
        (&mut Transform, &mut Visibility),
        (With<Flash>, Without<Drop>, Without<Flying>),
    >,
    mut equipment: Query<
        (&crate::pickup::EquippedPart, &mut Visibility),
        (Without<Drop>, Without<Flying>, Without<Flash>),
    >,
) {
    let (Some(mut g), Some(mut p), Some(mut c), Some(mut t), Some(item), Some(inv), Some(grid)) =
        (g, p, c, t, item, inv, preview.grid.as_ref())
    else {
        return;
    };
    g.update(
        &mut p,
        &mut c,
        &mut t,
        &item,
        &inv,
        grid,
        time.delta_secs(),
        item.callback_seconds,
    );
    for (mut tr, mut v) in &mut drops {
        *tr = g.drop_transform;
        *v = if g.available && !g.collected {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (mut tr, mut v) in &mut flying {
        if let Some(f) = &g.flight {
            let rotation = Quat::from_rotation_y(f.elapsed * 8.0);
            *tr =
                Transform::from_translation(f.position - rotation * (g.local_center * WORLD_SCALE))
                    .with_rotation(rotation)
                    .with_scale(Vec3::splat(WORLD_SCALE));
            *v = Visibility::Inherited;
        } else {
            *v = Visibility::Hidden;
        }
    }
    for (mut tr, mut v) in &mut flashes {
        if let Some((p, age)) = g.flash {
            *tr = Transform::from_translation(p).with_scale(Vec3::splat(0.2 + age * 4.0));
            *v = Visibility::Inherited;
        } else {
            *v = Visibility::Hidden;
        }
    }
    let holding = g.holding(&p);
    for (part, mut v) in &mut equipment {
        *v = if (holding && part.0 == g.definition.model)
            || (!holding && p.weapon.as_ref() == Some(&part.0))
        {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if g.sound_pending {
        g.sound_pending = false;
        if let Some(sound) = &g.sound_handle {
            commands.spawn((
                player::SceneEntity,
                AudioPlayer(sound.clone()),
                PlaybackSettings::DESPAWN,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::type_complexity)]
    fn fixture() -> (
        Grenade,
        combat::Combat,
        player::Player,
        Pickup,
        Tutorial,
        TerrainGrid,
        Inventory,
    ) {
        let (c, mut p, item, t, grid) = combat::tests::fixture();
        let mut inv = inventory::tests::inventory();
        inv.collected = true;
        inv.equip(inventory::Weapon::Bda, &mut p);
        let clip = p.clips.len();
        p.clips.push(crate::motion::Clip {
            name: "throw".into(),
            fps: p.clips[5].fps,
            frames: p.clips[5].frames.clone(),
        });
        let g = Grenade::new(
            Definition {
                model: "grenade".into(),
                range: 25.0,
                fuse: 3.5,
                radius: 11.0,
                damage: 150.0,
                sound: Vec::new(),
                source: serde_json::Value::Null,
            },
            clip,
            0.7,
        );
        (g, c, p, item, t, grid, inv)
    }
    #[test]
    fn pickup_callback_assigns_secondary_once_and_keeps_primary_ammo() {
        let (mut g, mut c, mut p, item, mut t, grid, mut inv) = fixture();
        inv.rounds = 9;
        g.available = true;
        g.position = Some(p.position);
        g.pickup_ordered = true;
        p.pickup_seconds = Some(0.29);
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
        assert_eq!(g.count, 0);
        p.pickup_seconds = Some(0.3);
        for _ in 0..3 {
            g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
        }
        assert_eq!(g.count, 1);
        assert!(g.secondary && g.collected);
        assert_eq!(p.weapon.as_deref(), Some("bda"));
        assert_eq!(inv.rounds, 9);
        assert_eq!(
            t.events
                .iter()
                .filter(|e| e.as_str() == "CET_OBJECT_PICKED_UP:Frag Grenade")
                .count(),
            1
        );
    }
    #[test]
    fn cancellation_before_release_preserves_item_and_after_release_preserves_flight() {
        let (mut g, mut c, mut p, item, mut t, grid, inv) = fixture();
        g.count = 1;
        g.secondary = true;
        for cancel_before in [true, false] {
            assert!(g.order_throw(c.guard.position, Some(0), true, &mut p, &grid));
            g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
            p.step(
                &ButtonInput::default(),
                if cancel_before { 0.69 } else { 0.7 },
                &grid,
            );
            g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
            g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
            g.cancel(&mut p);
            assert_eq!(g.count, if cancel_before { 1 } else { 0 });
            assert_eq!(g.throws, u32::from(!cancel_before));
            assert_eq!(g.flight.is_some(), !cancel_before);
        }
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.5, 0.3);
        assert!(g.flight.as_ref().unwrap().elapsed >= 0.49);
        assert_eq!(p.weapon.as_deref(), Some("bda"));
    }
    #[test]
    fn fuse_completes_objective_only_on_detonation_and_only_once() {
        let (mut g, mut c, mut p, item, mut t, grid, inv) = fixture();
        g.flight = Some(Flight {
            position: c.guard.position + Vec3::Y * 0.03,
            velocity: Vec3::ZERO,
            elapsed: 0.0,
            collisions: 0,
            resting: true,
        });
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 3.49, 0.3);
        assert_eq!(g.explosions, 0);
        assert_eq!(c.health, 75.0);
        assert!(t.completed.is_empty());
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.02, 0.3);
        assert_eq!(g.explosions, 1);
        assert_eq!(c.health, 0.0);
        assert!(t.completed.contains("first"));
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 1.0, 0.3);
        assert_eq!(g.explosions, 1);
        assert_eq!(c.hits, 1);
    }
    #[test]
    fn inventory_pause_actor_pause_help_and_invalid_delta_stop_fuse() {
        let (mut g, mut c, mut p, item, mut t, grid, mut inv) = fixture();
        g.flight = Some(Flight {
            position: p.position,
            velocity: Vec3::ZERO,
            elapsed: 0.0,
            collisions: 0,
            resting: true,
        });
        inv.open = true;
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 10.0, 0.3);
        inv.open = false;
        p.playback.paused = true;
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 10.0, 0.3);
        p.playback.paused = false;
        t.active = Some("after".into());
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 10.0, 0.3);
        t.active = None;
        for dt in [f32::NAN, -1.0] {
            g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, dt, 0.3);
        }
        assert_eq!(g.flight.unwrap().elapsed, 0.0);
        assert_eq!(g.explosions, 0);
    }
    #[test]
    fn wall_reflects_projectile_and_shields_guard_from_blast() {
        let (mut g, mut c, mut p, mut item, mut t, mut grid, inv) = fixture();
        // Полёт выше земли: здесь проверяется стена, а не склон из общей фикстуры.
        grid.heights.fill(0.0);
        crate::pickup::tests::add_wall(&mut item);
        let mut f = Flight {
            position: Vec3::new(0.5, 1.5, -0.5),
            velocity: Vec3::X * 40.0,
            elapsed: 0.0,
            collisions: 0,
            resting: false,
        };
        advance_flight(&mut f, 0.02, &grid, &item);
        assert!(
            f.position.x < 0.9 && f.velocity.x < 0.0 && f.collisions > 0 && f.position.is_finite(),
            "{f:?}"
        );
        g.flight = Some(Flight {
            position: Vec3::new(0.5, 1.5, -0.5),
            velocity: Vec3::ZERO,
            elapsed: 3.49,
            collisions: 0,
            resting: true,
        });
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.02, 0.3);
        assert_eq!(g.explosions, 1);
        assert_eq!(c.health, 75.0);
    }
    #[test]
    fn ground_collision_settles_without_tunneling_through_surface() {
        let (_, _, _, item, _, mut grid, _) = fixture();
        grid.heights.fill(0.0);
        let mut f = Flight {
            position: Vec3::new(0.5, 2.0, -0.5),
            velocity: Vec3::NEG_Y,
            elapsed: 0.0,
            collisions: 0,
            resting: false,
        };
        advance_flight(&mut f, 2.0, &grid, &item);
        assert!(f.resting && f.collisions > 0);
        assert!(f.position.y >= 0.0 && f.position.is_finite());
        assert_eq!(f.velocity, Vec3::ZERO);
    }
    #[test]
    fn empty_secondary_range_and_dead_target_prevent_release() {
        let (mut g, mut c, mut p, item, mut t, grid, inv) = fixture();
        assert!(!g.order_throw(c.guard.position, Some(0), true, &mut p, &grid));
        g.count = 1;
        g.secondary = true;
        g.definition.range = 0.1;
        assert!(g.order_throw(c.guard.position, Some(0), true, &mut p, &grid));
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
        assert!(p.action.is_none());
        assert_eq!(g.count, 1);
        g.cancel(&mut p);
        g.definition.range = 25.0;
        g.order_throw(c.guard.position, Some(0), true, &mut p, &grid);
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
        p.step(&ButtonInput::default(), 0.7, &grid);
        c.health = 0.0;
        g.update(&mut p, &mut c, &mut t, &item, &inv, &grid, 0.0, 0.3);
        assert_eq!(g.count, 1);
        assert!(g.flight.is_none() && !g.has_order());
    }
    #[test]
    fn restart_removes_live_projectile_and_restores_drop_state() {
        let (mut g, _, mut p, _, _, _, _) = fixture();
        g.count = 1;
        g.secondary = true;
        g.collected = true;
        g.available = true;
        g.throws = 1;
        g.explosions = 2;
        g.flash = Some((Vec3::ZERO, 0.1));
        g.flight = Some(Flight {
            position: p.position,
            velocity: Vec3::Y,
            elapsed: 1.0,
            collisions: 2,
            resting: false,
        });
        g.restart(&mut p);
        assert_eq!(g.count, 0);
        assert_eq!(g.throws, 0);
        assert_eq!(g.explosions, 0);
        assert!(!g.secondary && !g.available && !g.collected);
        assert!(g.flight.is_none() && g.flash.is_none());
    }
    #[test]
    fn actual_right_click_targets_secondary_and_shift_right_click_targets_ground() {
        use bevy::camera::{CameraProjection, RenderTargetInfo};
        let (mut g, mut c, mut p, item, _, mut grid, inv) = fixture();
        grid.config.edge = 1000.0;
        grid.heights.fill(0.0);
        p.position = Vec3::new(8.0, 0.0, -5.0);
        c.guard.position = Vec3::new(12.0, 0.0, -12.0);
        g.count = 1;
        g.secondary = true;
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(1280, 720),
            ..default()
        });
        camera.computed.clip_from_view = PerspectiveProjection {
            aspect_ratio: 1280.0 / 720.0,
            ..default()
        }
        .get_clip_from_view();
        let transform = GlobalTransform::from(
            Transform::from_xyz(10.0, 10.0, 5.0).looking_at(Vec3::new(10.0, 0.0, -10.0), Vec3::Y),
        );
        let cursor = camera.world_to_viewport(&transform, c.target()).unwrap();
        assert!(cursor.x > 315.0);
        let ground = Vec3::new(11.0, 0.0, -10.0);
        let ground_cursor = camera.world_to_viewport(&transform, ground).unwrap();
        let mut window = Window {
            resolution: (1280, 720).into(),
            ..default()
        };
        window.set_cursor_position(Some(cursor));
        let mut app = App::new();
        app.insert_resource(p)
            .insert_resource(c)
            .insert_resource(g)
            .insert_resource(inv)
            .insert_resource(item)
            .insert_resource(Time::<()>::default())
            .init_resource::<orders::ClickHistory>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(crate::training::Preview {
                scene: None,
                screenshot: None,
                focus: Vec3::ZERO,
                span: 20.0,
                grid: Some(grid),
                playable: true,
            })
            .add_systems(Update, crate::training::issue_orders);
        let window = app.world_mut().spawn(window).id();
        app.world_mut()
            .spawn((camera, transform, crate::training::MapCamera));
        let mut mouse = ButtonInput::default();
        mouse.press(MouseButton::Right);
        app.insert_resource(mouse);
        app.update();
        assert_eq!(
            app.world()
                .resource::<Grenade>()
                .order
                .as_ref()
                .unwrap()
                .guard,
            Some(0)
        );
        assert!(app.world().resource::<player::Player>().selected);
        assert_eq!(
            app.world().resource::<player::Player>().weapon.as_deref(),
            Some("bda")
        );
        app.world_mut()
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(ground_cursor));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        let mut mouse = ButtonInput::default();
        mouse.press(MouseButton::Right);
        app.insert_resource(mouse);
        app.update();
        let order = app.world().resource::<Grenade>().order.as_ref().unwrap();
        assert!(order.forced && order.guard.is_none());
        assert!(order.target.distance(ground) < 0.001);
    }
}
