//! Распашные ворота: исходные MTF/SMF, Act01/02 и узел 291.
//! EXE 0x4157c0/0x414b40: линейно 0 → −90° за 200 мс.
use crate::{
    map_scene::{MapScene, WORLD_SCALE},
    pickup::{self, MapObject, Pickup},
    player,
    tutorial::Tutorial,
    vehicle::{Bounds, Vehicle},
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use shadow_formats::{
    AssetCatalog, CampaignEventType as Event, SmfModel, TerrainGrid, config_lines,
};
use std::error::Error;

const NAME: &str = "AlStFn-ConsertinaFenceGate";
const DURATION: f32 = 0.2;
const OPEN_ANGLE: f32 = -std::f32::consts::FRAC_PI_2;
#[derive(Clone, Debug)]
struct Box3 {
    low: Vec3,
    high: Vec3,
}
impl Box3 {
    fn points(points: impl Iterator<Item = Vec3>) -> Result<Self, Box<dyn Error>> {
        let mut b = Self {
            low: Vec3::splat(f32::INFINITY),
            high: Vec3::splat(f32::NEG_INFINITY),
        };
        for p in points {
            b.low = b.low.min(p);
            b.high = b.high.max(p);
        }
        if !b.low.is_finite() || !b.high.is_finite() || (b.high - b.low).max_element() <= 0.01 {
            return Err("неверные границы ворот".into());
        }
        Ok(b)
    }
    fn corners(&self) -> impl Iterator<Item = Vec3> + '_ {
        [self.low.x, self.high.x].into_iter().flat_map(move |x| {
            [self.low.y, self.high.y].into_iter().flat_map(move |y| {
                [self.low.z, self.high.z]
                    .into_iter()
                    .map(move |z| Vec3::new(x, y, z))
            })
        })
    }
    fn hit(&self, from: Vec3, to: Vec3, margin: f32) -> Option<f32> {
        let low = self.low - Vec3::new(margin, 0.0, margin);
        let high = self.high + Vec3::new(margin, 0.0, margin);
        let delta = to - from;
        let (mut near, mut far): (f32, f32) = (0.0, 1.0);
        for i in [0, 2] {
            if delta[i].abs() < 1e-6 {
                if from[i] < low[i] || from[i] > high[i] {
                    return None;
                }
            } else {
                let a = (low[i] - from[i]) / delta[i];
                let b = (high[i] - from[i]) / delta[i];
                near = near.max(a.min(b));
                far = far.min(a.max(b));
                if near > far {
                    return None;
                }
            }
        }
        // Боец, уже оказавшийся внутри границы, может выйти из неё.
        if near == 0.0 && (to.x < low.x || to.x > high.x || to.z < low.z || to.z > high.z) {
            None
        } else {
            Some(near)
        }
    }
}
#[derive(Component)]
pub struct Leaf;
#[derive(Resource)]
pub struct Gate {
    pub id: [i32; 2],
    pub center: Vec3,
    pub sides: [Vec3; 2],
    pub parts: Vec<String>,
    pub open: bool,
    pub angle: f32,
    pub activations: u32,
    pub blocked: u32,
    pub message: String,
    pub source: serde_json::Value,
    pub test_cursor: Option<Vec2>,
    pub approaching: bool,
    frame: Mat4,
    pivot: Vec3,
    fixed: Vec<Box3>,
    leaf: Box3,
    triangles: Vec<[Vec3; 3]>,
    side: usize,
    target: Option<bool>,
    elapsed: f32,
}
impl Gate {
    pub fn prepare(catalog: &AssetCatalog, scene: &MapScene) -> Result<Self, Box<dyn Error>> {
        let matches: Vec<_> = scene
            .instances
            .iter()
            .filter(|i| i.name.eq_ignore_ascii_case(NAME) && !i.inventory)
            .collect();
        let [object] = matches.as_slice() else {
            return Err("нужны одни учебные ворота".into());
        };
        let map = shadow_formats::MtfMap::from_bytes(&catalog.read("maps/training_final.mtf")?)?;
        if !map
            .objects
            .iter()
            .any(|o| o.id == object.id && o.class == "Structure_Swing_Door")
        {
            return Err("неподдержанный класс ворот".into());
        }
        let defs = config_lines(&catalog.read("config/object_templates.txt")?)?;
        if !defs.iter().any(|l| {
            l.words.len() == 2
                && l.words[0].eq_ignore_ascii_case(NAME)
                && l.words[1] == "Structure_Swing_Door"
        }) {
            return Err("профиль ворот не является Structure_Swing_Door".into());
        }
        let smf = SmfModel::from_bytes(&catalog.read(&object.path)?)?;
        let basis = Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
        let mut matrices = vec![Mat4::IDENTITY; smf.nodes.len()];
        for &i in &smf.node_order {
            matrices[i] = smf.parents[i].map(|p| matrices[p]).unwrap_or(basis)
                * crate::smf_transform::rest(&smf.nodes[i], smf.parents[i].is_none()).to_matrix();
        }
        let helper = |id: u32| -> Result<usize, Box<dyn Error>> {
            let hits: Vec<_> = smf
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.id == id)
                .map(|(i, _)| i)
                .collect();
            let [i] = hits.as_slice() else {
                return Err(format!("нет однозначного узла ворот {id}").into());
            };
            Ok(*i)
        };
        let leaf_id = helper(291)?;
        if smf.nodes[leaf_id].meshes.is_empty() || smf.parents.contains(&Some(leaf_id)) {
            return Err("неподдержанная иерархия створки ворот".into());
        }
        let pivot = matrices[leaf_id].transform_point3(Vec3::ZERO);
        let parts: Vec<_> = smf.nodes[leaf_id]
            .meshes
            .iter()
            .map(|m| m.name.clone())
            .collect();
        for part in &parts {
            if smf
                .nodes
                .iter()
                .flat_map(|n| &n.meshes)
                .filter(|m| &m.name == part)
                .count()
                != 1
            {
                return Err("неоднозначная часть створки".into());
            }
        }
        let frame = Mat4::from_scale(Vec3::splat(WORLD_SCALE)) * object.transform.to_matrix();
        let sides = [helper(251)?, helper(261)?]
            .map(|i| frame.transform_point3(matrices[i].transform_point3(Vec3::ZERO)));
        if (sides[0] - sides[1]).length() < 0.5 || (sides[0] - sides[1]).length() > 5.0 {
            return Err("неверные точки подхода к воротам".into());
        }
        let mut fixed = Vec::new();
        for (i, n) in smf.nodes.iter().enumerate() {
            if i == leaf_id {
                continue;
            }
            for b in &n.bounds {
                let a = Vec3::from_slice(&b.coordinates[..3]);
                let c = Vec3::from_slice(&b.coordinates[3..]);
                let source = Box3 {
                    low: a.min(c),
                    high: a.max(c),
                };
                let bounds =
                    Box3::points(source.corners().map(|v| matrices[i].transform_point3(v)))?;
                if bounds.low.y < 30.0 {
                    fixed.push(bounds);
                }
            }
        }
        if fixed.len() != 2 {
            return Err("нужны две неподвижные границы ворот".into());
        }
        let mut points = Vec::new();
        let mut triangles = Vec::new();
        for part in scene.models[&object.path]
            .parts
            .iter()
            .filter(|p| parts.contains(&p.name))
        {
            let Some(VertexAttributeValues::Float32x3(v)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                return Err("нет геометрии створки".into());
            };
            let vs: Vec<_> = v.iter().map(|p| Vec3::from_array(*p)).collect();
            points.extend(vs.iter().copied());
            let ids: Vec<_> = part
                .mesh
                .indices()
                .ok_or("нет индексов створки")?
                .iter()
                .collect();
            triangles.extend(ids.as_chunks::<3>().0.iter().map(|ids| ids.map(|i| vs[i])));
        }
        let leaf = Box3::points(points.into_iter())?;
        if triangles.is_empty() {
            return Err("пустая створка ворот".into());
        }
        let center = frame.transform_point3((leaf.low + leaf.high) * 0.5);
        let source = serde_json::json!({"map":"maps/training_final.mtf","id":object.id,"position_source_units":object.transform.translation.to_array(),"rotation_xyzw":object.transform.rotation.to_array(),"model":object.path,"model_origin":catalog.source_label(&object.path)?,"leaf_node_id":291,"leaf_node":smf.nodes[leaf_id].name,"pivot_model_source_units":pivot.to_array(),"leaf_meshes":parts,"action_nodes":[251,261],"action_points_metres":sides.map(|p|p.to_array()),"fixed_bounds":fixed.iter().map(|b|[b.low.to_array(),b.high.to_array()]).collect::<Vec<_>>(),"leaf_bounds_model_units":[leaf.low.to_array(),leaf.high.to_array()],"angle_degrees":-90,"duration_ms":200,"interpolation":"linear","exe_evidence":["0x4157c0 constructor: duration 0xc8 ms","0x415a00: leaf id 0x123, rotation degrees * pi / 180","0x414b40: elapsed / duration * -90; clamp on completion","0x41f9b0: linear ratio"],"event":"CET_OBJECT_ACTIVATED:AlStFn-ConsertinaFenceGate","separate_objective":false,"prototype":"two action-point approach; opening on arrival, explicit click to close; no building traversal BMF or gate audio yet"});
        Ok(Self {
            id: object.id,
            center,
            sides,
            parts,
            open: false,
            angle: 0.0,
            activations: 0,
            blocked: 0,
            message: String::new(),
            source,
            test_cursor: None,
            approaching: false,
            frame,
            pivot,
            fixed,
            leaf,
            triangles,
            side: 0,
            target: None,
            elapsed: 0.0,
        })
    }
    fn local_matrix(&self) -> Mat4 {
        Mat4::from_translation(self.pivot)
            * Mat4::from_rotation_y(self.angle)
            * Mat4::from_translation(-self.pivot)
    }
    pub fn leaf_transform(&self) -> Transform {
        Transform::from_matrix(self.local_matrix())
    }
    fn world_leaf(&self) -> Mat4 {
        self.frame * self.local_matrix()
    }
    pub fn moving(&self) -> bool {
        self.target.is_some()
    }
    pub fn cancel(&mut self) {
        self.approaching = false;
    }
    pub fn order(&mut self, p: &mut player::Player, grid: &TerrainGrid) -> bool {
        if !p.selected || p.playback.paused || self.moving() {
            return false;
        }
        self.cancel();
        self.side = usize::from(
            (p.position - self.sides[1]).length_squared()
                < (p.position - self.sides[0]).length_squared(),
        );
        p.request_posture(crate::stance::Posture::Stand);
        if p.order(grid, self.sides[self.side], false) {
            self.approaching = true;
            self.message = "Подойти к воротам".into();
            true
        } else {
            false
        }
    }
    pub fn clicked(
        &self,
        camera: &Camera,
        tr: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> bool {
        let world = self.world_leaf();
        let center = world.transform_point3((self.leaf.low + self.leaf.high) * 0.5);
        if !geometry.clear_line_ignoring(tr.translation(), center, Some(self.id)) {
            return false;
        }
        camera.viewport_to_world(tr, cursor).is_ok_and(|ray| {
            self.triangles.iter().any(|t| {
                pickup::ray_triangle(
                    ray.origin,
                    ray.direction.as_vec3(),
                    t.map(|v| world.transform_point3(v)),
                )
                .is_some()
            })
        })
    }
    pub fn movement_hit(&self, from: Vec3, to: Vec3) -> Option<f32> {
        if !from.is_finite()
            || !to.is_finite()
            || Vec2::new(to.x - from.x, to.z - from.z).length_squared() < 1e-10
        {
            return None;
        }
        let inv = self.frame.inverse();
        let a = inv.transform_point3(from);
        let b = inv.transform_point3(to);
        let mut result = self
            .fixed
            .iter()
            .filter_map(|s| s.hit(a, b, 20.0))
            .min_by(f32::total_cmp);
        // При движении створки проход ещё закрыт; после открытия проверяем её новое положение.
        let inverse = if self.moving() {
            self.frame.inverse()
        } else {
            self.world_leaf().inverse()
        };
        if let Some(t) = self.leaf.hit(
            inverse.transform_point3(from),
            inverse.transform_point3(to),
            20.0,
        ) {
            result = Some(result.map_or(t, |v| v.min(t)));
        }
        result
    }
    pub fn guard_movement(&mut self, p: &mut player::Player, previous: Vec3) {
        if let Some(t) = self.movement_hit(previous, p.position) {
            p.position = previous.lerp(p.position, (t - 0.001).max(0.0));
            p.destination = None;
            p.playback.select(&p.clips, p.idle_clip());
            self.blocked += 1;
            self.message = "Ворота или ограждение перекрывают путь".into();
            self.cancel();
        }
    }
    pub fn update(&mut self, p: &mut player::Player, t: &mut Tutorial, dt: f32) {
        if p.playback.paused || t.pausing() || !dt.is_finite() || dt < 0.0 {
            return;
        }
        if self.approaching && p.destination.is_none() && !p.changing_posture() {
            self.approaching = false;
            if Vec2::new(
                p.position.x - self.sides[self.side].x,
                p.position.z - self.sides[self.side].z,
            )
            .length()
                <= 0.24
            {
                self.target = Some(!self.open);
                self.elapsed = 0.0;
                self.message = if self.open {
                    "Закрыть ворота"
                } else {
                    "Открыть ворота"
                }
                .into();
            }
        }
        if let Some(open) = self.target {
            self.elapsed = (self.elapsed + dt).min(DURATION);
            let fraction = self.elapsed / DURATION;
            self.angle = OPEN_ANGLE * if open { fraction } else { 1.0 - fraction };
            if self.elapsed >= DURATION {
                self.open = open;
                self.target = None;
                self.activations += 1;
                t.fire(Event::ObjectActivated, NAME);
                self.message = if open {
                    "Ворота открыты"
                } else {
                    "Ворота закрыты"
                }
                .into();
            }
        }
    }
    pub fn vehicle_bounds(&self) -> Vec<Bounds> {
        self.fixed
            .iter()
            .map(|b| (b, self.frame))
            .chain(std::iter::once((&self.leaf, self.world_leaf())))
            .map(|(b, m)| {
                let world = Box3::points(b.corners().map(|v| m.transform_point3(v)))
                    .expect("validated rigid gate transform");
                Bounds {
                    low: world.low,
                    high: world.high,
                }
            })
            .collect()
    }
    pub fn reset(&mut self) {
        self.open = false;
        self.angle = 0.0;
        self.target = None;
        self.elapsed = 0.0;
        self.activations = 0;
        self.blocked = 0;
        self.message.clear();
        self.cancel();
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id,"open":self.open,"angle_radians":self.angle,"moving":self.moving(),"approaching":self.approaching,"activations":self.activations,"blocked_moves":self.blocked,"message":self.message,"source":self.source})
    }
}
#[allow(clippy::too_many_arguments)]
pub fn tick(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    gate: Option<ResMut<Gate>>,
    p: Option<ResMut<player::Player>>,
    t: Option<ResMut<Tutorial>>,
    inv: Option<Res<crate::inventory::Inventory>>,
    geometry: Option<ResMut<Pickup>>,
    vehicle: Option<ResMut<Vehicle>>,
) {
    let (
        Some(mut gate),
        Some(mut p),
        Some(mut t),
        Some(inv),
        Some(mut geometry),
        Some(mut vehicle),
    ) = (gate, p, t, inv, geometry, vehicle)
    else {
        return;
    };
    let before = gate.world_leaf();
    if keys.just_pressed(KeyCode::Home) && !inv.open && !t.pausing() {
        gate.reset();
    } else if !inv.open {
        if keys.just_pressed(KeyCode::KeyH) || vehicle.occupied {
            gate.cancel();
        }
        gate.update(&mut p, &mut t, time.delta_secs());
    }
    let after = gate.world_leaf();
    if before != after {
        geometry.move_parts(gate.id, &gate.parts, after * before.inverse());
    }
    vehicle.replace_obstacles(gate.id, gate.vehicle_bounds());
}
pub fn visuals(
    gate: Option<Res<Gate>>,
    mut leaf: Query<(&ChildOf, &mut Transform), With<Leaf>>,
    objects: Query<&MapObject>,
) {
    let Some(gate) = gate else { return };
    for (parent, mut transform) in &mut leaf {
        if objects.get(parent.parent()).is_ok_and(|id| id.0 == gate.id) {
            *transform = gate.leaf_transform();
        }
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
    position: Vec3,
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
            position: Vec3::ZERO,
            checks: Vec::new(),
            before_reset: None,
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn native_smoke(
    mut commands: Commands,
    s: Option<ResMut<NativeSmoke>>,
    gate: Option<ResMut<Gate>>,
    p: Option<ResMut<player::Player>>,
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
    leaf: Query<&Transform, With<Leaf>>,
    vehicle: Option<Res<Vehicle>>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut s), Some(mut gate), Some(mut p), Some(t), Some(vehicle)) =
        (s, gate, p, t, vehicle)
    else {
        return;
    };
    let (camera, window, geometry, preview) = view;
    let (camera, tr) = *camera;
    let grid = preview.grid.as_ref().unwrap();
    s.total += 1;
    s.frames += 1;
    if s.last != Some(s.stage) {
        eprintln!(
            "gate smoke stage {}: {} player={:?}",
            s.stage,
            gate.report(),
            p.position
        );
        s.last = Some(s.stage);
        s.frames = 0;
    }
    assert!(s.total < 1400, "gate smoke timeout: stage {}", s.stage);
    keys.reset_all();
    mouse.reset_all();
    gate.test_cursor = None;
    if s.total < 20 {
        return;
    }
    if t.active.is_some() {
        keys.press(KeyCode::Enter);
        return;
    }
    let click = |gate: &mut Gate, mouse: &mut ButtonInput<MouseButton>| -> bool {
        let m = gate.world_leaf();
        for triangle in &gate.triangles {
            let point = m.transform_point3((triangle[0] + triangle[1] + triangle[2]) / 3.0);
            if let Ok(cursor) = camera.world_to_viewport(tr, point)
                && cursor.x > 315.0
                && gate.clicked(camera, tr, cursor, &geometry)
            {
                gate.test_cursor = Some(cursor);
                mouse.press(MouseButton::Left);
                return true;
            }
        }
        false
    };
    match s.stage {
        0 => {
            assert!(!vehicle.occupied, "gate smoke starts on foot");
            p.playback.paused = false;
            p.follow_camera = false;
            p.selected = true;
            let normal = (gate.sides[0] - gate.sides[1]).normalize();
            let mut point = gate.sides[0] + normal * 1.5;
            point.y = player::surface(grid, point).unwrap();
            p.position = point;
            p.destination = None;
            p.finish_pickup();
            orbit.target = gate.center;
            orbit.yaw = std::f32::consts::PI;
            orbit.pitch = 0.65;
            orbit.distance = 12.0;
            let mut beyond = gate.sides[1];
            beyond.y = player::surface(grid, beyond).unwrap();
            assert!(gate.movement_hit(gate.sides[0], beyond).is_some());
            s.checks.push(serde_json::json!({"closed_leaf_blocks":true,"setup":"John placed near original gate; earlier objectives not awarded by this fixture"}));
            s.stage = 1;
        }
        1 if s.frames > 4 && !p.changing_posture() => {
            if click(&mut gate, &mut mouse) {
                s.stage = 2;
            }
        }
        2 if gate.approaching && s.frames > 1 => {
            s.position = p.position;
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::KeyP);
            s.stage = 3;
        }
        3 if s.frames >= 12 => {
            assert!(p.playback.paused);
            assert!((p.position - s.position).length() < 0.001);
            assert_eq!(gate.angle, 0.0);
            s.checks
                .push(serde_json::json!({"pause_freezes_approach_and_gate":true}));
            keys.press(KeyCode::ControlLeft);
            keys.press(KeyCode::KeyP);
            s.stage = 4;
        }
        4 if gate.open && !gate.moving() && s.frames > 3 => {
            assert!(
                leaf.iter().count() == gate.parts.len()
                    && leaf
                        .iter()
                        .all(
                            |tr| tr.rotation.angle_between(gate.leaf_transform().rotation) < 0.001
                        )
            );
            assert_eq!(gate.activations, 1);
            assert_eq!(
                t.events
                    .iter()
                    .filter(|e| e.as_str() == format!("CET_OBJECT_ACTIVATED:{NAME}"))
                    .count(),
                1
            );
            assert!(!t.completed.contains("obj_rescue_beecher"));
            assert!(gate.movement_hit(gate.sides[0], gate.sides[1]).is_none());
            s.checks.push(serde_json::json!({"mouse_open":true,"original_event_once":true,"leaf_rotated":true,"jack_objective_not_completed":true}));
            if let Some(path) = &s.screenshot {
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
            }
            p.order(grid, gate.sides[1], false);
            s.stage = 5;
        }
        5 if p.destination.is_none() && s.frames > 5 => {
            assert!(
                (Vec2::new(p.position.x, p.position.z)
                    - Vec2::new(gate.sides[1].x, gate.sides[1].z))
                .length()
                    < 0.24,
                "open gate crossing failed: {:?}",
                p.position
            );
            s.checks
                .push(serde_json::json!({"walk_through_open_gate":true}));
            if click(&mut gate, &mut mouse) {
                s.stage = 6;
            }
        }
        6 if !gate.open && !gate.moving() && gate.activations == 2 && s.frames > 5 => {
            assert!(gate.movement_hit(gate.sides[1], gate.sides[0]).is_some());
            s.checks
                .push(serde_json::json!({"mouse_close":true,"closed_collision_restored":true}));
            s.before_reset = Some(gate.report());
            keys.press(KeyCode::Home);
            s.stage = 7;
        }
        7 if s.frames > 8 => {
            assert!(
                !gate.open
                    && !gate.moving()
                    && !gate.approaching
                    && gate.activations == 0
                    && gate.angle == 0.0
            );
            assert!(
                leaf.iter()
                    .all(|tr| tr.rotation.angle_between(Quat::IDENTITY) < 0.001)
            );
            let spawn = p.source["source_position"].as_array().unwrap();
            let x = spawn[0].as_f64().unwrap() as f32 * WORLD_SCALE;
            let z = -spawn[1].as_f64().unwrap() as f32 * WORLD_SCALE;
            assert!(
                Vec2::new(p.position.x - x, p.position.z - z).length() < 0.001
                    && p.destination.is_none(),
                "Home did not restore original John spawn: {:?}",
                p.position
            );
            s.checks.push(
                serde_json::json!({"home_restores_gate":true,"home_restores_john_spawn":true}),
            );
            let report = serde_json::json!({"status":"passed","window":[window.width(),window.height()],"checks":s.checks,"before_reset":s.before_reset,"after_reset":gate.report(),"actual_os_mouse_input":false,"input":"viewport ray and Bevy ButtonInput; physical mouse not exercised"});
            std::fs::write(
                &s.path,
                serde_json::to_string_pretty(&report).unwrap() + "\n",
            )
            .unwrap();
            s.stage = 8;
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shadow_formats::{HelpDefinitions, MissionDefinition};
    fn tutorial() -> Tutorial {
        let mut t = Tutorial::new(MissionDefinition::from_bytes(format!("SPECIFY_MTF test.mtf\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_ACTIVATED\nCER_FOR {NAME}\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER").as_bytes()).unwrap(),HelpDefinitions::from_bytes(b"HELP_DEF\nHELP_ID pause\nHELP_CONTEXT\nHELP_BODY \"Pause\"\n").unwrap()).unwrap();
        t.events.clear();
        t
    }
    fn gate() -> Gate {
        let frame = Mat4::from_translation(Vec3::new(1.0, 0.0, -1.0))
            * Mat4::from_scale(Vec3::splat(WORLD_SCALE));
        let b = |a, b| Box3 {
            low: Vec3::new(a, 0.0, -5.0),
            high: Vec3::new(b, 200.0, 5.0),
        };
        Gate {
            id: [1, -9],
            center: Vec3::new(1.0, 1.0, -1.0),
            sides: [Vec3::new(1.0, 0.0, -1.7), Vec3::new(1.0, 0.0, -0.3)],
            parts: vec!["leaf".into()],
            open: false,
            angle: 0.0,
            activations: 0,
            blocked: 0,
            message: String::new(),
            source: serde_json::Value::Null,
            test_cursor: None,
            approaching: false,
            frame,
            pivot: Vec3::new(50.0, 0.0, 0.0),
            fixed: vec![b(-200.0, -50.0), b(50.0, 200.0)],
            leaf: b(-45.0, 45.0),
            triangles: vec![],
            side: 0,
            target: None,
            elapsed: 0.0,
        }
    }
    #[test]
    fn closed_gate_and_side_walls_block_swept_motion() {
        let g = gate();
        assert!(g.movement_hit(g.sides[0], g.sides[1]).is_some());
        assert!(
            g.movement_hit(g.sides[0] + Vec3::X, g.sides[1] + Vec3::X)
                .is_some()
        );
        assert!(g.movement_hit(g.sides[0], g.sides[0]).is_none());
    }
    #[test]
    fn open_leaf_moves_around_source_pivot_but_side_walls_stay() {
        let mut g = gate();
        g.open = true;
        g.angle = OPEN_ANGLE;
        assert!(g.movement_hit(g.sides[0], g.sides[1]).is_none());
        assert!(
            g.movement_hit(g.sides[0] + Vec3::X, g.sides[1] + Vec3::X)
                .is_some()
        );
        assert!((g.local_matrix().transform_point3(g.pivot) - g.pivot).length() < 0.001);
        assert!(
            g.movement_hit(Vec3::new(1.2, 0.0, -1.5), Vec3::new(1.8, 0.0, -1.5))
                .is_some()
        );
    }
    #[test]
    fn activation_on_arrival_uses_two_action_points_and_linear_200_ms() {
        let mut g = gate();
        let mut p = player::tests::player();
        let grid = player::tests::grid();
        let mut t = tutorial();
        p.position = g.sides[0];
        assert!(g.order(&mut p, &grid));
        assert_eq!(g.side, 0);
        p.destination = None;
        g.update(&mut p, &mut t, 0.1);
        assert!(!g.open && g.moving());
        assert!((g.angle - OPEN_ANGLE * 0.5).abs() < 1e-6);
        assert!(t.events.is_empty());
        g.update(&mut p, &mut t, 0.1);
        assert!(g.open && !g.moving());
        assert_eq!(g.activations, 1);
        assert_eq!(t.events.len(), 1);
        g.update(&mut p, &mut t, 5.0);
        assert_eq!(t.events.len(), 1);
        p.position = g.sides[1];
        assert!(g.order(&mut p, &grid));
        assert_eq!(g.side, 1);
        p.destination = None;
        g.update(&mut p, &mut t, 0.2);
        assert!(!g.open);
        assert_eq!(g.angle, 0.0);
        assert_eq!(g.activations, 2);
    }
    #[test]
    fn pause_context_help_and_invalid_time_stop_gate_animation() {
        let mut g = gate();
        let mut p = player::tests::player();
        let mut t = tutorial();
        g.target = Some(true);
        p.playback.paused = true;
        g.update(&mut p, &mut t, 1.0);
        assert_eq!(g.angle, 0.0);
        p.playback.paused = false;
        for dt in [f32::NAN, f32::INFINITY, -1.0] {
            g.update(&mut p, &mut t, dt);
            assert_eq!(g.angle, 0.0);
        }
        t.active = Some("pause".into());
        g.update(&mut p, &mut t, 1.0);
        assert_eq!(g.angle, 0.0);
        t.active = None;
        g.update(&mut p, &mut t, 0.2);
        assert!(g.open);
    }
    #[test]
    fn cancel_distant_arrival_selection_and_busy_gate_do_not_activate() {
        let mut g = gate();
        let mut p = player::tests::player();
        let grid = player::tests::grid();
        let mut t = tutorial();
        p.selected = false;
        assert!(!g.order(&mut p, &grid));
        p.selected = true;
        assert!(g.order(&mut p, &grid));
        g.cancel();
        p.destination = None;
        g.update(&mut p, &mut t, 1.0);
        assert!(!g.moving());
        g.approaching = true;
        p.position = Vec3::ZERO;
        g.update(&mut p, &mut t, 1.0);
        assert!(!g.moving());
        g.target = Some(true);
        assert!(!g.order(&mut p, &grid));
    }
    #[test]
    fn collision_clamps_footstep_and_cancels_approach() {
        let mut g = gate();
        let mut p = player::tests::player();
        let from = g.sides[0];
        p.position = g.sides[1];
        p.destination = Some(from);
        g.approaching = true;
        g.guard_movement(&mut p, from);
        assert!(p.position.z < -1.0);
        assert!(p.destination.is_none() && !g.approaching);
        assert_eq!(g.blocked, 1);
    }
    #[test]
    fn vehicle_bounds_are_separate_walls_and_moved_leaf() {
        let mut g = gate();
        let closed = g.vehicle_bounds();
        g.open = true;
        g.angle = OPEN_ANGLE;
        let open = g.vehicle_bounds();
        assert_eq!(closed.len(), 3);
        assert_eq!(closed[0].low, open[0].low);
        assert_eq!(closed[1].high, open[1].high);
        assert!((closed[2].low - open[2].low).length() > 0.2);
    }
    #[test]
    fn reset_restores_original_leaf_and_counters() {
        let mut g = gate();
        g.angle = OPEN_ANGLE;
        g.open = true;
        g.activations = 2;
        g.approaching = true;
        g.blocked = 1;
        g.target = Some(false);
        g.reset();
        assert!(!g.open && !g.moving() && !g.approaching);
        assert_eq!(g.angle, 0.0);
        assert_eq!(g.activations, 0);
        assert_eq!(g.blocked, 0);
    }
    #[test]
    fn transformed_gate_collision_uses_original_mtf_frame() {
        let mut g = gate();
        let move_by =
            Mat4::from_translation(Vec3::new(12.0, 0.0, -20.0)) * Mat4::from_rotation_y(0.8);
        g.frame = move_by * g.frame;
        assert!(
            g.movement_hit(
                move_by.transform_point3(g.sides[0]),
                move_by.transform_point3(g.sides[1])
            )
            .is_some()
        );
        assert!(g.movement_hit(g.sides[0], g.sides[1]).is_none());
    }
}
