//! Hummer обучения: исходная геометрия/места, посадка, управление и безопасный выход.
//! Динамика пока кинематическая; формула физики оригинального EXE не восстановлена.
use crate::{
    inventory::Inventory,
    map_scene::{MapScene, WORLD_SCALE},
    pickup::{MapObject, Pickup},
    player,
    tutorial::Tutorial,
    ui_controls::{ButtonVisualState, UiButton},
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use shadow_formats::{
    AssetCatalog, CampaignEventType as Event, SmfModel, TerrainGrid, config_lines,
};
use std::error::Error;

const EXIT_RADIUS: f32 = 4.0;
#[derive(Clone, Debug)]
pub struct Bounds {
    pub low: Vec3,
    pub high: Vec3,
}
impl Bounds {
    fn points(points: impl Iterator<Item = Vec3>) -> Result<Self, Box<dyn Error>> {
        let mut b = Self {
            low: Vec3::splat(f32::INFINITY),
            high: Vec3::splat(f32::NEG_INFINITY),
        };
        for p in points {
            b.low = b.low.min(p);
            b.high = b.high.max(p);
        }
        if !b.low.is_finite() || !b.high.is_finite() {
            return Err("пустые границы техники".into());
        }
        Ok(b)
    }
    fn center(&self) -> Vec3 {
        (self.low + self.high) * 0.5
    }
    fn contains(&self, p: Vec3, margin: f32) -> bool {
        p.x >= self.low.x - margin
            && p.x <= self.high.x + margin
            && p.z >= self.low.z - margin
            && p.z <= self.high.z + margin
    }
}
#[derive(Clone)]
struct Obstacle {
    id: [i32; 2],
    bounds: Bounds,
}
#[derive(Clone)]
pub struct Wheel {
    pub name: String,
    pub pivot: Vec3,
    pub radius: f32,
    pub front: bool,
}
#[derive(Component)]
pub struct Tire(pub Wheel);
#[derive(Component)]
pub struct ExitCircle;
#[derive(Component)]
pub struct ExitButton;
#[derive(Component)]
pub struct Hud;
#[derive(Resource)]
pub struct Vehicle {
    pub id: [i32; 2],
    pub position: Vec3,
    pub heading: f32,
    pub occupied: bool,
    pub approaching: bool,
    pub exit_armed: bool,
    pub speed: f32,
    pub steering: f32,
    pub distance: f32,
    pub entries: u32,
    pub exits: u32,
    pub blocked: u32,
    pub message: String,
    pub wheels: Vec<Wheel>,
    pub source: serde_json::Value,
    pub(crate) test_cursor: Option<Vec2>,
    pub consumed_input: bool,
    spawn: Vec3,
    spawn_heading: f32,
    body: Bounds,
    seat: Vec3,
    wheelbase: f32,
    mass: f32,
    forward_power: f32,
    reverse_power: f32,
    approach: Option<Vec3>,
    obstacles: Vec<Obstacle>,
    triangles: Vec<[Vec3; 3]>,
}
impl Vehicle {
    pub fn prepare(catalog: &AssetCatalog, scene: &MapScene) -> Result<Self, Box<dyn Error>> {
        let path = "models/alvhqd-hummer/alvhqd-hummer.smf";
        let found: Vec<_> = scene
            .instances
            .iter()
            .filter(|i| {
                i.name.eq_ignore_ascii_case("AlVhQd-Hummer") && i.path == path && !i.inventory
            })
            .collect();
        let [object] = found.as_slice() else {
            return Err("нужен один исходный Hummer обучения".into());
        };
        let map = shadow_formats::MtfMap::from_bytes(&catalog.read("maps/training_final.mtf")?)?;
        if !map
            .objects
            .iter()
            .any(|o| o.id == object.id && o.class == "4x4")
        {
            return Err("неподдержанный класс Hummer".into());
        }
        let profile = "config/vehicle_var_defs.txt";
        let lines = config_lines(&catalog.read(profile)?)?;
        let value = |suffix: &str| -> Result<f32, Box<dyn Error>> {
            let key = format!("alvhqd-hummer_{suffix}");
            let found: Vec<_> = lines
                .iter()
                .filter(|l| {
                    l.words.len() == 3
                        && l.words[1].eq_ignore_ascii_case(&key)
                        && l.words[0] == "float"
                })
                .collect();
            let [l] = found.as_slice() else {
                return Err(format!("нужен один параметр {key}").into());
            };
            let n = l.words[2].parse::<f32>()?;
            if !n.is_finite() || !(0.01..=1000000.0).contains(&n) {
                return Err(format!("неверный параметр {key}").into());
            }
            Ok(n)
        };
        let (mass, forward_power, reverse_power) =
            (value("mass")?, value("fwd_power")?, value("rvs_power")?);
        let model = SmfModel::from_bytes(&catalog.read(path)?)?;
        let basis = Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
        let mut transforms = vec![Mat4::IDENTITY; model.nodes.len()];
        for &i in &model.node_order {
            transforms[i] = model.parents[i].map(|p| transforms[p]).unwrap_or(basis)
                * crate::smf_transform::rest(&model.nodes[i], model.parents[i].is_none())
                    .to_matrix();
        }
        let helper = |name: &str| -> Result<Vec3, Box<dyn Error>> {
            let hits: Vec<_> = model
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.name.eq_ignore_ascii_case(name))
                .collect();
            let [(i, _)] = hits.as_slice() else {
                return Err(format!("нет однозначного узла {name}").into());
            };
            Ok(transforms[*i].transform_point3(Vec3::ZERO) * WORLD_SCALE)
        };
        let seat = helper("seat1")?;
        let wheelbase = (helper("Gcp1")? - helper("Gcp3")?).length();
        if !(0.5..=10.0).contains(&wheelbase) {
            return Err("неверная база колёс Hummer".into());
        }
        let mut triangles = Vec::new();
        let mut points = Vec::new();
        let mut wheels = Vec::new();
        for part in &scene.models[path].parts {
            let Some(VertexAttributeValues::Float32x3(v)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                return Err("нет геометрии Hummer".into());
            };
            let vertices: Vec<_> = v
                .iter()
                .map(|p| Vec3::from_array(*p) * WORLD_SCALE)
                .collect();
            points.extend(vertices.iter().copied());
            let indices: Vec<_> = part
                .mesh
                .indices()
                .ok_or("нет индексов Hummer")?
                .iter()
                .collect();
            triangles.extend(
                indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|i| [vertices[i[0]], vertices[i[1]], vertices[i[2]]]),
            );
            if let Some(node) = model.nodes.iter().find(|n| {
                n.name.to_ascii_lowercase().starts_with("tire-")
                    && n.meshes.iter().any(|m| m.name == part.name)
            }) {
                let bounds = Bounds::points(vertices.iter().copied())?;
                let radius = (bounds.high.y - bounds.low.y) * 0.5;
                if !(0.1..=2.0).contains(&radius) {
                    return Err("неверный радиус колеса".into());
                }
                wheels.push(Wheel {
                    name: part.name.clone(),
                    pivot: bounds.center() / WORLD_SCALE,
                    radius,
                    front: node.name.to_ascii_lowercase().ends_with('f'),
                });
            }
        }
        if wheels.len() != 4 {
            return Err("нужны четыре колеса Hummer".into());
        }
        let body = Bounds::points(points.into_iter())?;
        let mut obstacles = Vec::new();
        for i in scene
            .instances
            .iter()
            .filter(|i| i.id != object.id && !i.inventory)
        {
            let matrix = Mat4::from_scale(Vec3::splat(WORLD_SCALE)) * i.transform.to_matrix();
            for part in scene.models[&i.path]
                .parts
                .iter()
                .filter(|p| p.alpha_mode == AlphaMode::Opaque)
            {
                let Some(VertexAttributeValues::Float32x3(vertices)) =
                    part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                else {
                    continue;
                };
                let b = Bounds::points(
                    vertices
                        .iter()
                        .map(|p| matrix.transform_point3(Vec3::from_array(*p))),
                )?;
                if b.high.y - b.low.y >= 0.25 {
                    obstacles.push(Obstacle {
                        id: i.id,
                        bounds: b,
                    });
                }
            }
        }
        let (_, rotation, translation) =
            object.transform.to_matrix().to_scale_rotation_translation();
        let forward = rotation * Vec3::NEG_Z;
        let heading = (-forward.x).atan2(-forward.z);
        let spawn = translation * WORLD_SCALE;
        let source = serde_json::json!({"map":"maps/training_final.mtf","id":object.id,"position_source_units":translation.to_array(),"rotation_xyzw":rotation.to_array(),"model":path,"profile":profile,"profile_origin":catalog.source_label(profile)?,"mass":mass,"forward_power":forward_power,"reverse_power":reverse_power,"driver_seat":"seat1","driver_seat_local_metres":seat.to_array(),"wheelbase_metres":wheelbase,"wheels":wheels.len(),"prototype_acceleration":"power / mass in m/s²; original unit conversion unverified","maximum_speed_prototype_metres_per_second":[12.0,4.0],"exit_radius_prototype_metres":EXIT_RADIUS,"original_physics_verified":false,"collision":"oriented vehicle footprint versus opaque static mesh AABBs; 1/120 s substeps","suspension":false,"vehicle_weapons":false,"engine_audio_connected":false});
        Ok(Self {
            id: object.id,
            position: spawn,
            heading,
            occupied: false,
            approaching: false,
            exit_armed: false,
            speed: 0.0,
            steering: 0.0,
            distance: 0.0,
            entries: 0,
            exits: 0,
            blocked: 0,
            message: "ЛКМ по Hummer — сесть".into(),
            wheels,
            source,
            test_cursor: None,
            consumed_input: false,
            spawn,
            spawn_heading: heading,
            body,
            seat,
            wheelbase,
            mass,
            forward_power,
            reverse_power,
            approach: None,
            obstacles,
            triangles,
        })
    }
    fn matrix_at(position: Vec3, heading: f32) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_rotation_y(heading), position)
    }
    pub fn matrix(&self) -> Mat4 {
        Self::matrix_at(self.position, self.heading)
    }
    pub fn center(&self) -> Vec3 {
        self.matrix().transform_point3(self.body.center())
    }
    pub fn seat_position(&self) -> Vec3 {
        self.matrix().transform_point3(self.seat)
    }
    pub fn clicked(
        &self,
        camera: &Camera,
        transform: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> bool {
        if self.occupied
            || !geometry.clear_line_ignoring(transform.translation(), self.center(), Some(self.id))
        {
            return false;
        }
        let Ok(ray) = camera.viewport_to_world(transform, cursor) else {
            return false;
        };
        let matrix = self.matrix();
        self.triangles.iter().any(|t| {
            crate::pickup::ray_triangle(
                ray.origin,
                ray.direction.as_vec3(),
                t.map(|p| matrix.transform_point3(p)),
            )
            .is_some()
        })
    }
    pub fn order(&mut self, p: &mut player::Player, grid: &TerrainGrid) -> bool {
        if self.occupied || p.playback.paused || !p.selected {
            return false;
        }
        let left = Vec3::new(self.body.low.x - 0.65, 0.0, self.seat.z);
        let right = Vec3::new(self.body.high.x + 0.65, 0.0, self.seat.z);
        let point = [left, right]
            .map(|p| self.matrix().transform_point3(p))
            .into_iter()
            .min_by(|a, b| {
                a.distance_squared(p.position)
                    .total_cmp(&b.distance_squared(p.position))
            })
            .unwrap();
        self.approaching = p.order(grid, point, false);
        self.approach = self.approaching.then_some(point);
        self.approaching
    }
    pub fn cancel_approach(&mut self, p: &mut player::Player) {
        if self.approaching {
            p.destination = None;
        }
        self.approaching = false;
        self.approach = None;
    }
    pub fn board(&mut self, p: &mut player::Player, t: &mut Tutorial, grid: &TerrainGrid) {
        if !self.approaching || p.playback.paused || t.pausing() {
            return;
        }
        if p.changing_posture() {
            self.cancel_approach(p);
            return;
        }
        let Some(point) = self.approach else { return };
        let distance = Vec2::new(p.position.x - point.x, p.position.z - point.z).length();
        if distance <= 0.6 {
            self.approaching = false;
            self.approach = None;
            self.occupied = true;
            self.entries += 1;
            p.destination = None;
            p.finish_pickup();
            p.position = self.seat_position();
            p.heading = self.heading;
            p.follow_camera = true;
            self.message = "Hummer: стрелки — ехать · H — стоп · E — выход".into();
            t.fire(Event::VehicleOccupied, "AlVhQd-Hummer");
            t.tick(0.0, false);
        } else if p.destination.is_none() || player::surface(grid, point).is_none() {
            self.cancel_approach(p)
        }
    }
    pub fn arm_exit(&mut self) -> bool {
        if !self.occupied {
            return false;
        }
        if self.speed.abs() > 0.2 {
            self.message = "Сначала остановись: H".into();
            return false;
        }
        self.speed = 0.0;
        self.exit_armed = !self.exit_armed;
        self.message = if self.exit_armed {
            "ЛКМ по земле внутри круга · ПКМ отменяет"
        } else {
            "Стрелки — ехать · H — стоп · E — выход"
        }
        .into();
        true
    }
    fn footprint(&self, position: Vec3, heading: f32) -> [Vec3; 4] {
        let b = &self.body;
        let m = Self::matrix_at(position, heading);
        [
            (b.low.x, b.low.z),
            (b.high.x, b.low.z),
            (b.high.x, b.high.z),
            (b.low.x, b.high.z),
        ]
        .map(|(x, z)| m.transform_point3(Vec3::new(x, 0.0, z)))
    }
    fn overlaps(foot: [Vec3; 4], b: &Bounds) -> bool {
        let rect = [
            Vec3::new(b.low.x, 0.0, b.low.z),
            Vec3::new(b.high.x, 0.0, b.low.z),
            Vec3::new(b.high.x, 0.0, b.high.z),
            Vec3::new(b.low.x, 0.0, b.high.z),
        ];
        [
            Vec3::X,
            Vec3::Z,
            (foot[1] - foot[0]).normalize_or_zero(),
            (foot[3] - foot[0]).normalize_or_zero(),
        ]
        .into_iter()
        .all(|a| {
            let range = |points: &[Vec3; 4]| {
                points
                    .iter()
                    .map(|p| p.dot(a))
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), n| {
                        (l.min(n), h.max(n))
                    })
            };
            let (l, h) = range(&foot);
            let (r, s) = range(&rect);
            h >= r && s >= l
        })
    }
    pub fn exit(
        &mut self,
        point: Vec3,
        p: &mut player::Player,
        grid: &TerrainGrid,
        cut: Option<[i32; 2]>,
    ) -> bool {
        if !self.occupied || !self.exit_armed || !point.is_finite() || self.speed.abs() > 0.2 {
            return false;
        }
        let Some(height) = self.exit_height(point, grid, cut) else {
            self.message = "Выбери свободную точку внутри круга".into();
            return false;
        };
        self.occupied = false;
        self.exit_armed = false;
        self.exits += 1;
        p.position = Vec3::new(point.x, height, point.z);
        p.destination = None;
        p.finish_pickup();
        self.message = "ЛКМ по Hummer — сесть".into();
        true
    }
    fn exit_height(&self, point: Vec3, grid: &TerrainGrid, cut: Option<[i32; 2]>) -> Option<f32> {
        if !point.is_finite() {
            return None;
        }
        let center = self.center();
        let d = Vec2::new(point.x - center.x, point.z - center.z).length();
        let local = self.matrix().inverse().transform_point3(point);
        let height = player::surface(grid, point)?;
        if d > EXIT_RADIUS
            || self.body.contains(local, 0.3)
            || self.obstacles.iter().any(|o| {
                Some(o.id) != cut
                    && o.bounds.contains(point, 0.3)
                    && o.bounds.high.y > height + 0.2
                    && o.bounds.low.y < height + 1.8
            })
        {
            return None;
        }
        Some(height)
    }
    pub fn drive(
        &mut self,
        throttle: f32,
        steer: f32,
        dt: f32,
        grid: &TerrainGrid,
        cut: Option<[i32; 2]>,
    ) {
        if !self.occupied
            || self.exit_armed
            || !dt.is_finite()
            || dt <= 0.0
            || !throttle.is_finite()
            || !steer.is_finite()
        {
            return;
        }
        let mut remaining = dt.min(0.25);
        while remaining > 0.00001 {
            let dt = remaining.min(1.0 / 120.0);
            remaining -= dt;
            let throttle = throttle.clamp(-1.0, 1.0);
            self.steering = steer.clamp(-1.0, 1.0) * 0.55;
            let acceleration = if throttle > 0.0 {
                self.forward_power / self.mass
            } else {
                self.reverse_power / self.mass
            };
            let delta = if throttle == 0.0 {
                -self.speed.signum() * (self.speed.abs().min(1.6 * dt))
            } else {
                throttle * acceleration * dt
            };
            self.speed = (self.speed + delta).clamp(-4.0, 12.0);
            let angle = self.heading + self.speed * self.steering.tan() / self.wheelbase * dt;
            let mut next =
                self.position + Quat::from_rotation_y(angle) * Vec3::NEG_Z * self.speed * dt;
            let foot = self.footprint(next, angle);
            let valid = foot.iter().all(|p| player::surface(grid, *p).is_some());
            let height = player::surface(grid, next);
            let hit = self.obstacles.iter().any(|o| {
                Some(o.id) != cut
                    && o.bounds.high.y > next.y + 0.2
                    && o.bounds.low.y < next.y + self.body.high.y
                    && Self::overlaps(foot, &o.bounds)
            });
            if !valid || height.is_none() || hit {
                self.speed = 0.0;
                self.blocked += 1;
                self.message = "Проезд перекрыт".into();
                break;
            }
            next.y = height.unwrap();
            self.position = next;
            self.heading = angle;
            self.distance += self.speed * dt;
        }
    }
    pub fn restart(&mut self) {
        self.position = self.spawn;
        self.heading = self.spawn_heading;
        self.occupied = false;
        self.approaching = false;
        self.approach = None;
        self.exit_armed = false;
        self.speed = 0.0;
        self.steering = 0.0;
        self.distance = 0.0;
        self.entries = 0;
        self.exits = 0;
        self.blocked = 0;
        self.message = "ЛКМ по Hummer — сесть".into();
    }
    /// Динамические препятствия ворот, включая маскированную сетку.
    pub fn replace_obstacles(&mut self, id: [i32; 2], bounds: Vec<Bounds>) {
        self.obstacles.retain(|o| o.id != id);
        self.obstacles
            .extend(bounds.into_iter().map(|bounds| Obstacle { id, bounds }));
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id,"position_metres":self.position.to_array(),"heading_radians":self.heading,"occupied":self.occupied,"approaching":self.approaching,"exit_armed":self.exit_armed,"speed_metres_per_second":self.speed,"distance_signed_metres":self.distance,"entries":self.entries,"exits":self.exits,"blocked_steps":self.blocked,"message":self.message,"source":self.source})
    }
}

pub fn on_foot_or_reset(v: Option<Res<Vehicle>>, keys: Res<ButtonInput<KeyCode>>) -> bool {
    v.is_none_or(|v| !v.occupied) || keys.just_pressed(KeyCode::Home)
}
#[allow(clippy::too_many_arguments)]
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    v: Option<ResMut<Vehicle>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<Res<Inventory>>,
    t: Option<Res<Tutorial>>,
    buttons: Query<&ButtonVisualState, With<ExitButton>>,
    geometry: Option<ResMut<Pickup>>,
) {
    let (Some(mut v), Some(mut p)) = (v, p) else {
        return;
    };
    v.consumed_input = false;
    if t.is_some_and(|t| t.active.is_some() || t.consumed_input) || inv.is_some_and(|i| i.open) {
        return;
    }
    if keys.just_pressed(KeyCode::Home) {
        let before = v.matrix();
        v.restart();
        if let Some(mut g) = geometry {
            g.move_object(v.id, v.matrix() * before.inverse())
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyH) && p.selected {
        v.cancel_approach(&mut p);
        v.speed = 0.0;
    }
    if v.occupied && p.selected && !p.playback.paused {
        let click = mouse.just_pressed(MouseButton::Left)
            && buttons.iter().any(|b| *b == ButtonVisualState::Pressed);
        if keys.just_pressed(KeyCode::KeyE) || click {
            v.arm_exit();
            v.consumed_input = click;
        }
        if mouse.just_pressed(MouseButton::Right) {
            v.exit_armed = false;
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub fn tick(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    v: Option<ResMut<Vehicle>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<Res<Inventory>>,
    t: Option<ResMut<Tutorial>>,
    preview: Res<crate::training::Preview>,
    geometry: Option<ResMut<Pickup>>,
    f: Option<Res<crate::fence::Fence>>,
    mut orbit: ResMut<crate::training::Orbit>,
) {
    let (Some(mut v), Some(mut p), Some(inv), Some(mut t), Some(grid)) =
        (v, p, inv, t, &preview.grid)
    else {
        return;
    };
    if inv.open || t.pausing() || p.playback.paused {
        // Player::step привязывает пехоту к земле даже при паузе. Водитель
        // остаётся на исходном месте в машине; компенсируем и камеру слежения.
        if v.occupied {
            let previous = p.position;
            p.position = v.seat_position();
            p.heading = v.heading;
            if p.follow_camera {
                orbit.target += p.position - previous;
            }
        }
        return;
    }
    let was = v.occupied;
    let previous = p.position;
    let before = v.matrix();
    v.board(&mut p, &mut t, grid);
    if v.occupied && !t.pausing() {
        let (throttle, steer) = if preview.screenshot.is_some() || !p.selected {
            (0.0, 0.0)
        } else {
            (
                f32::from(keys.pressed(KeyCode::ArrowUp))
                    - f32::from(keys.pressed(KeyCode::ArrowDown)),
                f32::from(keys.pressed(KeyCode::ArrowLeft))
                    - f32::from(keys.pressed(KeyCode::ArrowRight)),
            )
        };
        let cut = f.filter(|f| f.cut).map(|f| f.id);
        v.drive(throttle, steer, time.delta_secs(), grid, cut);
        p.position = v.seat_position();
        p.heading = v.heading;
        p.destination = None;
        if p.follow_camera {
            if was {
                orbit.target += p.position - previous
            } else {
                orbit.target = v.center();
                orbit.distance = 18.0;
            }
        }
    }
    if v.matrix() != before
        && let Some(mut g) = geometry
    {
        g.move_object(v.id, v.matrix() * before.inverse());
    }
}
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn visuals(
    v: Option<Res<Vehicle>>,
    mut objects: Query<(&MapObject, &mut Transform)>,
    mut tires: Query<(&Tire, &mut Transform), Without<MapObject>>,
    mut roots: Query<&mut Visibility, With<player::Root>>,
    mut markers: Query<&mut Visibility, (With<player::Marker>, Without<player::Root>)>,
    mut circles: Query<
        (&mut Transform, &mut Visibility),
        (
            With<ExitCircle>,
            Without<MapObject>,
            Without<Tire>,
            Without<player::Root>,
            Without<player::Marker>,
        ),
    >,
    hud: Option<Res<crate::hud::Hud>>,
    mut widgets: Query<(&mut Node, Has<crate::selection::UiBlock>), With<Hud>>,
    mut texts: Query<&mut Text, With<Hud>>,
) {
    let Some(v) = v else { return };
    for (id, mut transform) in &mut objects {
        if id.0 == v.id {
            transform.translation = v.position / WORLD_SCALE;
            transform.rotation = Quat::from_rotation_y(v.heading);
        }
    }
    for (tire, mut transform) in &mut tires {
        let w = &tire.0;
        let rotation = Quat::from_rotation_y(if w.front { v.steering } else { 0.0 })
            * Quat::from_rotation_x(-v.distance / w.radius);
        transform.rotation = rotation;
        transform.translation = w.pivot - rotation * w.pivot;
    }
    for mut visibility in &mut roots {
        *visibility = if v.occupied {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    if v.occupied {
        for mut visibility in &mut markers {
            *visibility = Visibility::Hidden;
        }
    }
    for (mut transform, mut visibility) in &mut circles {
        transform.translation = Vec3::new(v.center().x, v.position.y + 0.05, v.center().z);
        *visibility = if v.occupied && v.exit_armed {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for (mut node, panel) in &mut widgets {
        if panel && let Some(h) = &hud {
            node.left = px(310.0 * h.fit.scale);
            node.right = Val::Auto;
            node.bottom = px(25.0 * h.fit.scale);
            node.width = px(h.fit.screen.x - 530.0 * h.fit.scale);
        }

        node.display = if v.occupied {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut text in &mut texts {
        **text = format!("Hummer · {:.1} км/ч\n{}", v.speed * 3.6, v.message);
    }
}
pub fn setup(
    mut commands: Commands,
    v: Option<Res<Vehicle>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut fonts: ResMut<Assets<Font>>,
    camera: Single<Entity, With<crate::training::MapCamera>>,
) {
    if v.is_none() {
        return;
    }
    commands.spawn((
        player::SceneEntity,
        ExitCircle,
        Mesh3d(meshes.add(Torus::new(EXIT_RADIUS - 0.025, EXIT_RADIUS + 0.025))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.9, 0.95),
            unlit: true,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
    ));
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../../shadow-viewer/assets/fonts/DejaVuSans.ttf").to_vec(),
    ));
    let panel = commands
        .spawn((
            player::SceneEntity,
            Hud,
            crate::selection::UiBlock,
            UiTargetCamera(*camera),
            GlobalZIndex(16),
            Node {
                position_type: PositionType::Absolute,
                right: px(25),
                bottom: px(68),
                width: px(540),
                padding: UiRect::all(px(4)),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .id();
    commands.spawn((
        Hud,
        Text::new("Hummer"),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(17.0),
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.92, 0.8)),
        ChildOf(panel),
    ));
    let button = commands
        .spawn((
            UiButton,
            ExitButton,
            Node {
                padding: UiRect::all(px(4)),
                align_self: AlignSelf::Start,
                ..default()
            },
            BackgroundColor(Color::srgb(0.15, 0.3, 0.2)),
            ChildOf(panel),
        ))
        .id();
    commands.spawn((
        Text::new("Выйти · E → выбрать точку"),
        TextFont {
            font: font.into(),
            font_size: FontSize::Px(17.0),
            ..default()
        },
        ChildOf(button),
    ));
}

#[derive(Resource)]
pub struct NativeSmoke {
    path: std::path::PathBuf,
    screenshot: Option<std::path::PathBuf>,
    stage: u32,
    frames: u32,
    total: u32,
    last: Option<u32>,
    start: Vec3,
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
            start: Vec3::ZERO,
            checks: Vec::new(),
            before_reset: None,
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn native_smoke(
    mut commands: Commands,
    s: Option<ResMut<NativeSmoke>>,
    v: Option<ResMut<Vehicle>>,
    p: Option<ResMut<player::Player>>,
    equipment: (
        Option<Res<Inventory>>,
        Option<Res<crate::armor::Armor>>,
        Option<Res<crate::medical::Medical>>,
    ),
    t: Option<Res<Tutorial>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    view: (
        Single<(&Camera, &GlobalTransform), With<crate::training::MapCamera>>,
        Single<&Window>,
        Res<Pickup>,
        Res<crate::training::Preview>,
    ),
    mut orbit: ResMut<crate::training::Orbit>,
    render: (
        Query<&Visibility, With<player::Root>>,
        Query<(&MapObject, &Transform)>,
        Query<&Visibility, With<ExitCircle>>,
        Query<&mut ButtonVisualState, With<ExitButton>>,
    ),
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut s), Some(mut v), Some(mut p), (Some(inv), Some(armor), Some(medical)), Some(t)) =
        (s, v, p, equipment, t)
    else {
        return;
    };
    let (camera, window, geometry, preview) = view;
    let (roots, objects, circles, mut buttons) = render;
    let (camera, transform) = *camera;
    s.total += 1;
    s.frames += 1;
    if s.last != Some(s.stage) {
        eprintln!(
            "vehicle smoke stage {}: occupied={} approach={} speed={:.2} position={:?}",
            s.stage, v.occupied, v.approaching, v.speed, p.position
        );
        s.last = Some(s.stage);
        s.frames = 0;
    }
    assert!(
        s.total < 2400,
        "vehicle smoke timeout stage {}: {} player={}",
        s.stage,
        v.report(),
        p.report()
    );
    keys.reset_all();
    mouse.reset_all();
    if s.total < 20 {
        return;
    }
    if t.active.is_some() {
        keys.press(KeyCode::Enter);
        return;
    }
    let click_vehicle = |v: &mut Vehicle,
                         mouse: &mut ButtonInput<MouseButton>,
                         orbit: &mut crate::training::Orbit|
     -> bool {
        let Ok(cursor) = camera.world_to_viewport(transform, v.center()) else {
            return false;
        };
        if cursor.x < 0.0 || !v.clicked(camera, transform, cursor, &geometry) {
            orbit.yaw += 0.12;
            return false;
        }
        v.test_cursor = Some(cursor);
        mouse.press(MouseButton::Left);
        true
    };
    let screenshot = |commands: &mut Commands, path: std::path::PathBuf| {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    };
    match s.stage {
        0 => {
            p.playback.paused = false;
            p.selected = true;
            p.follow_camera = false;
            orbit.target = v.center();
            orbit.distance = 18.0;
            keys.press(KeyCode::KeyS);
            s.stage = 1;
        }
        1 if !p.changing_posture() => {
            if click_vehicle(&mut v, &mut mouse, &mut orbit) {
                s.stage = 2;
            }
        }
        2 if !v.occupied && !v.approaching && p.destination.is_none() => {
            s.stage = 1;
        }
        2 if v.occupied && t.shown.iter().any(|h| h == "help_combat_16") => {
            assert!(
                roots
                    .iter()
                    .all(|visibility| *visibility == Visibility::Hidden)
            );
            assert_eq!(t.completed.len(), 5);
            assert!(t.objectives.contains("obj_rescue_beecher"));
            assert!(objects.iter().any(|(id, tr)| id.0 == v.id
                && (tr.translation * WORLD_SCALE - v.position).length() < 0.001));
            s.checks.push(serde_json::json!({"mouse_boarding":true,"fifth_objective":true,"driver_hidden":true,"source_helper":"seat1"}));
            s.start = v.position;
            s.stage = 3;
        }
        3 => {
            if s.frames < 12 {
                keys.press(KeyCode::ArrowUp);
            } else {
                assert!(
                    v.position.distance(s.start) > 0.05,
                    "vehicle did not move: {}",
                    v.report()
                );
                assert!((p.position - v.seat_position()).length() < 0.01);
                let travelled = v.position.distance(s.start);
                s.checks.push(serde_json::json!({"arrow_driving":true,"travelled_metres":travelled,"driver_tracks_seat":true}));
                keys.press(KeyCode::KeyH);
                s.stage = 4;
            }
        }
        4 if s.frames >= 3 => {
            assert_eq!(v.speed, 0.0);
            s.start = v.position;
            if let Some(path) = &s.screenshot {
                screenshot(&mut commands, path.clone());
            }
            s.stage = 5;
        }
        5 if s.frames >= 4 => {
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::KeyP);
            s.stage = 6;
        }
        6 => {
            assert!(p.playback.paused);
            assert!((p.position - v.seat_position()).length() < 0.001);
            assert_eq!(v.position, s.start);
            if s.frames < 5 {
                keys.press(KeyCode::ArrowUp);
            } else {
                s.checks
                    .push(serde_json::json!({"pause_freezes_driving":true}));
                keys.press(KeyCode::ControlLeft);
                keys.press(KeyCode::KeyP);
                s.stage = 7;
            }
        }
        7 => {
            assert!(!p.playback.paused);
            for mut button in &mut buttons {
                *button = ButtonVisualState::Pressed;
            }
            mouse.press(MouseButton::Left);
            s.stage = 8;
        }
        8 if s.frames >= 3 => {
            assert!(v.exit_armed && circles.iter().any(|vis| *vis == Visibility::Visible));
            s.checks
                .push(serde_json::json!({"exit_button":true,"exit_circle_visible":true}));
            if let Some(path) = &s.screenshot {
                screenshot(
                    &mut commands,
                    path.with_file_name(format!(
                        "{}-exit.png",
                        path.file_stem().unwrap().to_string_lossy()
                    )),
                );
            }
            s.stage = 9;
        }
        9 if s.frames >= 4 => {
            let grid = preview.grid.as_ref().unwrap();
            let point = (0..32)
                .map(|i| {
                    let angle = i as f32 * std::f32::consts::TAU / 32.0;
                    v.center() + Vec3::new(angle.cos(), 0.0, angle.sin()) * 3.6
                })
                .find_map(|mut point| {
                    point.y = v.exit_height(point, grid, None)?;
                    let cursor = camera.world_to_viewport(transform, point).ok()?;
                    (cursor.x > 315.0 && cursor.y > 10.0 && cursor.y < window.height() - 30.0)
                        .then_some((point, cursor))
                })
                .expect("visible free exit point");
            v.test_cursor = Some(point.1);
            mouse.press(MouseButton::Left);
            s.stage = 10;
        }
        10 if !v.occupied => {
            assert!(
                roots
                    .iter()
                    .all(|visibility| *visibility != Visibility::Hidden)
            );
            assert_eq!((inv.rounds, inv.uzi_rounds), (9, 14));
            assert!(armor.worn);
            assert_eq!((medical.health, medical.uses), (327.0, 1));
            s.checks.push(serde_json::json!({"mouse_exit":true,"driver_visible":true,"weapon_armor_health_preserved":true}));
            orbit.target = v.center();
            if click_vehicle(&mut v, &mut mouse, &mut orbit) {
                s.stage = 11;
            }
        }
        11 if v.occupied => {
            assert_eq!(v.entries, 2);
            assert_eq!(t.shown.iter().filter(|h| *h == "help_combat_16").count(), 1);
            assert_eq!(t.completed.len(), 5);
            keys.press(KeyCode::KeyE);
            s.stage = 12;
        }
        12 => {
            assert!(v.exit_armed);
            s.checks
                .push(serde_json::json!({"reentry":true,"help_once":true,"exit_key_e":true}));
            s.before_reset = Some(
                serde_json::json!({"vehicle":v.report(),"inventory":inv.report(&p),"armor":armor.report(),"medical":medical.report(),"tutorial":t.report()}),
            );
            keys.press(KeyCode::Home);
            s.stage = 13;
        }
        13 => {
            assert!(!v.occupied && !v.approaching && !v.exit_armed);
            assert_eq!(v.position, v.spawn);
            assert_eq!((v.speed, v.entries, v.exits), (0.0, 0, 0));
            assert!(!armor.worn && !armor.ground.collected);
            assert_eq!((medical.health, medical.uses), (327.0, 5));
            assert!(!inv.owns_uzi && !inv.collected);
            assert!(t.completed.is_empty());
            s.checks
                .push(serde_json::json!({"home_restores_vehicle_and_equipment":true}));
            let report = serde_json::json!({"status":"passed","window":[window.width(),window.height()],"checks":s.checks,"before_reset":s.before_reset,"after_reset":v.report(),"physical_pointer_tested":false,"starting_checkpoint":"armor-420","native_input":"viewport mouse and ButtonVisualState, ArrowUp/H/Ctrl+P/E/Home"});
            std::fs::write(
                &s.path,
                serde_json::to_string_pretty(&report).unwrap() + "\n",
            )
            .unwrap();
            s.stage = 14;
        }
        14 if s.frames >= 20 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Vehicle, player::Player, Inventory, Tutorial, TerrainGrid) {
        let (_, mut p, inv, _, _) = crate::smg::tests::fixture();
        p.position = Vec3::new(51.65, 0.0, -51.0);
        let mission=shadow_formats::MissionDefinition::from_bytes(b"SPECIFY_MTF training.mtf\nDEFINE_OBJECTIVE true vehicle \"Hummer\"\nCER_DEFINITION\nCER_FOR_TYPE CET_VEHICLE_OCCUPIED\nCER_FOR AlVhQd-Hummer\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_NOTIFY_OBJECTIVE_COMPLETE\nCER_BODY_IDENTIFIER vehicle\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER board\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\n").unwrap();
        let helps = shadow_formats::HelpDefinitions::from_bytes(
            b"HELP_DEF\nHELP_ID board\nHELP_CONTEXT\nHELP_BODY \"ARROW keys\"\n",
        )
        .unwrap();
        let mut t = Tutorial::new(mission, helps).unwrap();
        t.objectives.insert("vehicle".into());
        let grid = TerrainGrid {
            config: shadow_formats::TerrainConfig {
                name: "test".into(),
                width: 61,
                height: 61,
                edge: 500.0,
                height_step: 1.0,
                water_index: 0,
            },
            heights: vec![0.0; 61 * 61],
        };
        let v = Vehicle {
            id: [1, -767],
            position: Vec3::new(50.0, 0.0, -50.0),
            heading: 0.0,
            occupied: false,
            approaching: false,
            exit_armed: false,
            speed: 0.0,
            steering: 0.0,
            distance: 0.0,
            entries: 0,
            exits: 0,
            blocked: 0,
            message: String::new(),
            wheels: Vec::new(),
            source: serde_json::Value::Null,
            test_cursor: None,
            consumed_input: false,
            spawn: Vec3::new(50.0, 0.0, -50.0),
            spawn_heading: 0.0,
            body: Bounds {
                low: Vec3::new(-1.0, 0.0, -2.0),
                high: Vec3::new(1.0, 2.0, 1.0),
            },
            seat: Vec3::new(0.4, 1.2, -1.0),
            wheelbase: 3.75,
            mass: 15000.0,
            forward_power: 35000.0,
            reverse_power: 15000.0,
            approach: None,
            obstacles: Vec::new(),
            triangles: Vec::new(),
        };
        (v, p, inv, t, grid)
    }
    #[test]
    fn nearby_boarding_keeps_weapon_and_emits_one_objective_and_delayed_help() {
        let (mut v, mut p, inv, mut t, g) = fixture();
        let weapon = p.weapon.clone();
        assert!(v.order(&mut p, &g));
        v.board(&mut p, &mut t, &g);
        assert!(v.occupied && !v.approaching);
        assert_eq!(p.position, v.seat_position());
        assert_eq!(p.weapon, weapon);
        assert_eq!(inv.rounds, 9);
        assert!(t.completed.contains("vehicle"));
        for _ in 0..3 {
            v.board(&mut p, &mut t, &g)
        }
        assert_eq!(v.entries, 1);
        t.tick(0.999, false);
        assert!(t.active.is_none());
        t.tick(0.002, false);
        assert_eq!(t.active.as_deref(), Some("board"));
    }
    #[test]
    fn stopped_approach_and_posture_change_cannot_board_remotely() {
        let (mut v, mut p, _, mut t, g) = fixture();
        p.position = Vec3::new(60.0, 0.0, -50.0);
        assert!(v.order(&mut p, &g));
        p.destination = None;
        v.board(&mut p, &mut t, &g);
        assert!(!v.occupied && !v.approaching);
        assert!(v.order(&mut p, &g));
        p.request_posture(crate::stance::Posture::Crouch);
        v.board(&mut p, &mut t, &g);
        assert!(!v.approaching && !v.occupied);
    }
    #[test]
    fn pause_and_context_help_block_boarding() {
        let (mut v, mut p, _, mut t, g) = fixture();
        assert!(v.order(&mut p, &g));
        p.playback.paused = true;
        v.board(&mut p, &mut t, &g);
        assert!(!v.occupied);
        p.playback.paused = false;
        t.active = Some("board".into());
        v.board(&mut p, &mut t, &g);
        assert!(!v.occupied);
        t.close();
        v.board(&mut p, &mut t, &g);
        assert!(v.occupied);
    }
    #[test]
    fn throttle_reverse_and_coasting_respect_caps() {
        let (mut v, _, _, _, g) = fixture();
        v.occupied = true;
        for _ in 0..40 {
            v.drive(1.0, 0.0, 0.25, &g, None)
        }
        assert!(v.speed > 0.0 && v.speed <= 12.0);
        let speed = v.speed;
        v.drive(0.0, 0.0, 0.25, &g, None);
        assert!(v.speed < speed);
        v.position = v.spawn;
        v.speed = 0.0;
        for _ in 0..20 {
            v.drive(-1.0, 0.0, 0.25, &g, None)
        }
        assert!((-4.0..0.0).contains(&v.speed));
    }
    #[test]
    fn steering_requires_motion_and_reverses_when_backing() {
        let (mut v, _, _, _, g) = fixture();
        v.occupied = true;
        v.drive(0.0, 1.0, 0.1, &g, None);
        assert_eq!(v.heading, 0.0);
        v.speed = 2.0;
        v.drive(0.0, 1.0, 0.1, &g, None);
        assert!(v.heading > 0.0);
        v.heading = 0.0;
        v.speed = -2.0;
        v.drive(0.0, 1.0, 0.1, &g, None);
        assert!(v.heading < 0.0);
    }
    #[test]
    fn thin_wall_blocks_full_footprint_even_on_long_frame() {
        let (mut v, _, _, _, g) = fixture();
        v.occupied = true;
        v.speed = 12.0;
        v.obstacles.push(Obstacle {
            id: [1, 8],
            bounds: Bounds {
                low: Vec3::new(48.0, -1.0, -53.1),
                high: Vec3::new(52.0, 3.0, -53.05),
            },
        });
        v.drive(1.0, 0.0, 0.25, &g, None);
        assert_eq!(v.speed, 0.0);
        assert!(v.blocked > 0 && v.position.z > -51.1);
    }
    #[test]
    fn open_cut_fence_is_excluded_and_map_edges_block() {
        let (mut v, _, _, _, g) = fixture();
        v.occupied = true;
        v.speed = 12.0;
        v.obstacles.push(Obstacle {
            id: [1, 8],
            bounds: Bounds {
                low: Vec3::new(48.0, -1.0, -53.1),
                high: Vec3::new(52.0, 3.0, -53.05),
            },
        });
        v.drive(1.0, 0.0, 0.25, &g, Some([1, 8]));
        assert!(v.position.z < -52.0 && v.speed > 0.0);
        v.position = Vec3::new(50.0, 0.0, -298.8);
        v.heading = 0.0;
        v.drive(1.0, 0.0, 0.25, &g, None);
        assert_eq!(v.speed, 0.0);
    }
    #[test]
    fn exit_rejects_inside_body_outside_circle_nonfinite_and_obstructed_point() {
        let (mut v, mut p, inv, _, g) = fixture();
        v.occupied = true;
        assert!(v.arm_exit());
        let before = p.position;
        for point in [
            v.center(),
            Vec3::new(60.0, 0.0, -50.0),
            Vec3::splat(f32::NAN),
        ] {
            assert!(!v.exit(point, &mut p, &g, None));
            assert_eq!(p.position, before)
        }
        v.obstacles.push(Obstacle {
            id: [1, 9],
            bounds: Bounds {
                low: Vec3::new(52.5, 0.0, -51.0),
                high: Vec3::new(53.5, 2.0, -49.0),
            },
        });
        let point = Vec3::new(53.0, 0.0, -50.0);
        assert!(!v.exit(point, &mut p, &g, None));
        assert!(v.exit(point, &mut p, &g, Some([1, 9])));
        assert!(!v.occupied && !v.exit_armed);
        assert_eq!(p.position, point);
        assert_eq!(inv.rounds, 9);
    }
    #[test]
    fn exit_mode_blocks_driving_and_moving_exit_requires_stop() {
        let (mut v, _, _, _, g) = fixture();
        v.occupied = true;
        v.speed = 2.0;
        assert!(!v.arm_exit());
        v.speed = 0.0;
        assert!(v.arm_exit());
        let point = v.position;
        v.drive(1.0, 1.0, 0.25, &g, None);
        assert_eq!(v.position, point);
        assert_eq!(v.speed, 0.0);
    }
    #[test]
    fn frame_substeps_make_acceleration_independent_of_outer_frame_length() {
        let (mut a, _, _, _, g) = fixture();
        let (mut b, _, _, _, _) = fixture();
        a.occupied = true;
        b.occupied = true;
        for _ in 0..8 {
            a.drive(1.0, 0.3, 0.25, &g, None)
        }
        for _ in 0..240 {
            b.drive(1.0, 0.3, 1.0 / 120.0, &g, None)
        }
        assert!(a.position.distance(b.position) < 0.005);
        assert!((a.heading - b.heading).abs() < 0.005);
    }
    #[test]
    fn paused_driver_stays_on_seat_after_infantry_terrain_snap() {
        let (mut v, mut p, inv, t, grid) = fixture();
        v.occupied = true;
        let seat = v.seat_position();
        p.position = Vec3::new(seat.x, 0.0, seat.z);
        p.playback.paused = true;
        p.follow_camera = true;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(v)
            .insert_resource(p)
            .insert_resource(inv)
            .insert_resource(t)
            .insert_resource(crate::training::Preview {
                scene: None,
                screenshot: None,
                focus: Vec3::ZERO,
                span: 1.0,
                grid: Some(grid),
                playable: true,
            })
            .insert_resource(crate::training::Orbit {
                target: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.5,
                distance: 10.0,
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, tick);
        app.update();
        assert_eq!(app.world().resource::<player::Player>().position, seat);
        assert!((app.world().resource::<crate::training::Orbit>().target.y - seat.y).abs() < 0.001);
    }
    #[test]
    fn real_input_e_button_and_home_preserve_reset_contract() {
        let (mut v, p, inv, t, _) = fixture();
        v.occupied = true;
        v.position += Vec3::X * 8.0;
        v.entries = 1;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(v)
            .insert_resource(p)
            .insert_resource(inv)
            .insert_resource(t)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyE);
        app.update();
        assert!(app.world().resource::<Vehicle>().exit_armed);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset_all();
        app.world_mut()
            .spawn((ExitButton, ButtonVisualState::Pressed));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        let v = app.world().resource::<Vehicle>();
        assert!(!v.exit_armed && v.consumed_input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Home);
        app.update();
        let v = app.world().resource::<Vehicle>();
        assert_eq!(v.position, v.spawn);
        assert!(!v.occupied && !v.exit_armed);
        assert_eq!((v.entries, v.exits), (0, 0));
    }
    #[test]
    fn dynamic_gate_obstacles_replace_old_pose_and_preserve_other_objects() {
        let (mut v, _, _, _, grid) = fixture();
        let gate = [1, 99];
        let wall = Bounds {
            low: Vec3::new(48.0, -1.0, -53.1),
            high: Vec3::new(52.0, 3.0, -53.05),
        };
        v.obstacles.push(Obstacle {
            id: [2, 7],
            bounds: Bounds {
                low: Vec3::new(1.0, 0.0, -2.0),
                high: Vec3::new(2.0, 3.0, -1.0),
            },
        });
        v.replace_obstacles(gate, vec![wall]);
        v.occupied = true;
        v.speed = 12.0;
        v.drive(1.0, 0.0, 0.25, &grid, None);
        assert_eq!(v.speed, 0.0);
        assert!(v.blocked > 0);
        v.position = v.spawn;
        v.speed = 12.0;
        v.replace_obstacles(
            gate,
            vec![Bounds {
                low: Vec3::new(20.0, 0.0, -53.0),
                high: Vec3::new(21.0, 3.0, -52.0),
            }],
        );
        assert_eq!(v.obstacles.iter().filter(|o| o.id == gate).count(), 1);
        assert!(v.obstacles.iter().any(|o| o.id == [2, 7]));
        v.drive(1.0, 0.0, 0.25, &grid, None);
        assert!(v.speed > 0.0 && v.position.z < -52.0);
    }
    #[test]
    fn unselected_driver_cannot_arm_exit_but_home_still_resets_vehicle() {
        let (mut vehicle, mut player, inventory, tutorial, _) = fixture();
        vehicle.occupied = true;
        player.selected = false;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(vehicle)
            .insert_resource(player)
            .insert_resource(inventory)
            .insert_resource(tutorial)
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, input);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyE);
        app.update();
        assert!(!app.world().resource::<Vehicle>().exit_armed);
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.reset_all();
        keys.press(KeyCode::Home);
        app.update();
        assert!(!app.world().resource::<Vehicle>().occupied);
    }
}
