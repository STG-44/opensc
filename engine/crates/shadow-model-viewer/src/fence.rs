//! Кусачки MTF, оригинальная модель прохода и последовательности cut/equip/crawl.
use crate::{
    inventory::{self, Inventory, Weapon},
    map_scene::{MapScene, WORLD_SCALE},
    motion::Clip,
    pickup::{self, Pickup},
    player, rig, scene,
    tutorial::Tutorial,
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use shadow_formats::{
    AssetCatalog, BmfMotion, CampaignEventType as Event, ConfigLine, SmfModel, TerrainGrid,
    config_lines,
};
use std::{collections::BTreeSet, error::Error};

const EVENT_NAME: &str = "AlStFn-Consertinafence_END";
const CUT_MODEL: &str = "AlStFn-ConsertinaFence_CUT_END";
#[derive(Clone, Debug)]
struct Box2 {
    low: Vec2,
    high: Vec2,
}
impl Box2 {
    fn hit(&self, from: Vec2, to: Vec2, margin: f32) -> Option<f32> {
        let low = self.low - Vec2::splat(margin);
        let high = self.high + Vec2::splat(margin);
        let delta = to - from;
        let (mut near, mut far): (f32, f32) = (0.0, 1.0);
        for i in 0..2 {
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
        // Разрешаем выйти, если исходная точка уже внутри коробки.
        if near == 0.0 && (to.cmplt(low).any() || to.cmpgt(high).any()) {
            None
        } else {
            Some(near)
        }
    }
}
struct Barrier {
    id: [i32; 2],
    inverse: Mat4,
    boxes: Vec<Box2>,
}
fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}
struct FenceGeometry {
    bounds: Vec<Box2>,
    actions: Option<[Vec3; 2]>,
}
fn node_data(catalog: &AssetCatalog, path: &str) -> Result<FenceGeometry, Box<dyn Error>> {
    let model = SmfModel::from_bytes(&catalog.read(path)?)?;
    let basis = Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
    let mut matrices = vec![Mat4::IDENTITY; model.nodes.len()];
    for &i in &model.node_order {
        matrices[i] = model.parents[i].map(|p| matrices[p]).unwrap_or(basis)
            * crate::smf_transform::rest(&model.nodes[i], model.parents[i].is_none()).to_matrix();
    }
    let mut boxes = Vec::new();
    let mut actions = [None, None];
    for (i, n) in model.nodes.iter().enumerate() {
        if let Some(j) = ["Act01", "Act02"]
            .iter()
            .position(|s| n.name.eq_ignore_ascii_case(s))
        {
            actions[j] = Some(matrices[i].transform_point3(Vec3::ZERO));
        }
        for b in &n.bounds {
            let a = Vec3::from_slice(&b.coordinates[..3]);
            let c = Vec3::from_slice(&b.coordinates[3..6]);
            let (lo, hi) = (a.min(c), a.max(c));
            let mut low = Vec3::splat(f32::INFINITY);
            let mut high = Vec3::splat(f32::NEG_INFINITY);
            for x in [lo.x, hi.x] {
                for y in [lo.y, hi.y] {
                    for z in [lo.z, hi.z] {
                        let p = matrices[i].transform_point3(Vec3::new(x, y, z));
                        low = low.min(p);
                        high = high.max(p);
                    }
                }
            }
            if !low.is_finite() || !high.is_finite() {
                return Err("неверные границы ограждения".into());
            }
            // Верхняя перекладина cut-модели не перекрывает проход на уровне земли.
            if low.y < 10.0 {
                boxes.push(Box2 {
                    low: flat(low),
                    high: flat(high),
                });
            }
        }
    }
    if boxes.is_empty() {
        return Err("у ограждения нет нижних границ столкновения".into());
    }
    Ok(FenceGeometry {
        bounds: boxes,
        actions: actions[0].zip(actions[1]).map(|(a, b)| [a, b]),
    })
}
fn triangles(model: &scene::PreparedModel) -> Result<Vec<[Vec3; 3]>, Box<dyn Error>> {
    let mut result = Vec::new();
    for part in &model.parts {
        let Some(VertexAttributeValues::Float32x3(v)) =
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return Err("нет геометрии предмета/сетки".into());
        };
        let ids: Vec<_> = part
            .mesh
            .indices()
            .ok_or("нет индексов предмета/сетки")?
            .iter()
            .collect();
        for t in ids.as_chunks::<3>().0 {
            result.push(t.map(|i| Vec3::from_array(v[i])));
        }
    }
    if result.is_empty() {
        return Err("пустая геометрия предмета/сетки".into());
    }
    Ok(result)
}
fn sequence_body<'a>(
    lines: &'a [ConfigLine],
    name: &str,
) -> Result<&'a [ConfigLine], Box<dyn Error>> {
    let matches: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.words == ["BEGIN_SEQUENCE", name])
        .map(|(i, _)| i)
        .collect();
    let [start] = matches.as_slice() else {
        return Err(format!("нужна одна последовательность {name}").into());
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.words[0] == "END_SEQUENCE")
        .ok_or("последовательность сетки не завершена")?
        + start
        + 1;
    Ok(&lines[start + 1..end])
}
fn load_clip(
    catalog: &AssetCatalog,
    name: &str,
    bones: &[rig::BoneDefinition],
) -> Result<Clip, Box<dyn Error>> {
    Clip::new(
        &BmfMotion::from_bytes(
            &catalog.read(&format!("motions/{}.bmf", name.to_ascii_lowercase()))?,
        )?,
        bones,
    )
}
struct FenceClips {
    cut: Clip,
    cut_frame: usize,
    swap: Clip,
    swap_frame: usize,
    crawl: Clip,
    reps: u32,
}
fn cut_sequences(
    catalog: &AssetCatalog,
    bones: &[rig::BoneDefinition],
) -> Result<FenceClips, Box<dyn Error>> {
    let lines = config_lines(&catalog.read("config/mot_sequencer_defs.txt")?)?;
    let cut = sequence_body(&lines, "MSEQ_CUT_FENCE")?;
    let motions: Vec<_> = cut
        .iter()
        .enumerate()
        .filter(|(_, l)| l.words[0] == "MOTION")
        .collect();
    let [(a, ma), (b, mb)] = motions.as_slice() else {
        return Err("нужны два MOTION разрезания".into());
    };
    if ma.words.len() != 2 || mb.words.len() != 2 {
        return Err("неподдержанный режим разрезания".into());
    }
    let first = load_clip(catalog, &ma.words[1], bones)?;
    let second = load_clip(catalog, &mb.words[1], bones)?;
    let callback =
        |slice: &[ConfigLine], name: &str, clip: &Clip| -> Result<usize, Box<dyn Error>> {
            let found: Vec<_> = slice
                .iter()
                .filter(|l| l.words.get(2).is_some_and(|s| s == name))
                .collect();
            let [line] = found.as_slice() else {
                return Err(format!("нужен один callback {name}").into());
            };
            if line.words.len() != 3 || line.words[0] != "CALLBACK" {
                return Err("неверный callback сетки".into());
            }
            let frame = line.words[1].parse::<usize>()?;
            if frame >= clip.frames.len() {
                return Err(format!("callback {name} за пределами BMF").into());
            }
            Ok(frame)
        };
    let cut_frame = callback(&cut[a + 1..*b], "MCB_FENCE_CUT", &first)?;
    let swap_frame = callback(&cut[b + 1..], "MCB_SWAP_ITEM_FOR_INVENTORY_ITEM", &second)?;
    let crawl = sequence_body(&lines, "MSEQ_CROUCH_CRAWL_THROUGH_FENCE")?;
    let motions: Vec<_> = crawl.iter().filter(|l| l.words[0] == "MOTION").collect();
    let [motion] = motions.as_slice() else {
        return Err("нужен один MOTION пролезания".into());
    };
    if motion.words.len() != 3 {
        return Err("нет REPS пролезания".into());
    }
    let reps = motion.words[2]
        .strip_prefix("REPS=")
        .ok_or("неверный REPS пролезания")?
        .parse::<u32>()?;
    if !(1..=20).contains(&reps) {
        return Err("REPS пролезания вне диапазона".into());
    }
    let mut repeated = load_clip(catalog, &motion.words[1], bones)?;
    repeat_crawl(&mut repeated, reps)?;
    Ok(FenceClips {
        cut: first,
        cut_frame,
        swap: second,
        swap_frame,
        crawl: repeated,
        reps,
    })
}
fn repeat_crawl(clip: &mut Clip, reps: u32) -> Result<(), Box<dyn Error>> {
    let original = clip.frames.clone();
    let displacement = clip.cycle_displacement();
    if clip.duration() <= 0.0 || displacement.length() < 1e-6 {
        return Err("пролезание без движения".into());
    }
    for i in 1..reps {
        for frame in original.iter().skip(1) {
            let mut pose = frame.clone();
            pose.root += displacement * i as f32;
            clip.frames.push(pose);
        }
    }
    Ok(())
}
#[derive(Clone, Copy, Debug)]
enum Phase {
    ToCut(usize),
    Cutting,
    Swapping,
    ToCross(usize),
    Crossing(usize),
}
#[derive(Resource)]
pub struct Fence {
    pub tool_id: [i32; 2],
    pub tool_model: String,
    pub tool_position: Vec3,
    pub tool_center: Vec3,
    pub collected: bool,
    pub pickup_ordered: bool,
    pub auto_equip: bool,
    pub id: [i32; 2],
    pub center: Vec3,
    pub sides: [Vec3; 2],
    pub crawl_sides: [Vec3; 2],
    pub cut: bool,
    pub cuts: u32,
    pub crossings: u32,
    pub blocked_moves: u32,
    pub message: String,
    pub source: serde_json::Value,
    phase: Option<Phase>,
    cut_clip: usize,
    swap_clip: usize,
    crawl_clip: usize,
    cut_callback: f32,
    swap_callback: f32,
    swap_pending: bool,
    resume_weapon: Option<String>,
    resume_rifle: bool,
    tool_transform: Mat4,
    fence_transform: Mat4,
    tool_triangles: Vec<[Vec3; 3]>,
    fence_triangles: Vec<[Vec3; 3]>,
    barriers: Vec<Barrier>,
    open_boxes: Vec<Box2>,
    cut_model: Option<scene::PreparedModel>,
}
impl Fence {
    pub fn prepare(
        catalog: &AssetCatalog,
        map: &MapScene,
        p: &mut player::Player,
        inv: &mut Inventory,
    ) -> Result<Self, Box<dyn Error>> {
        let path = "config/inventory_object_profiles/tools.txt";
        let lines = config_lines(&catalog.read(path)?)?;
        let profile = inventory::block(&lines, "OBJECT_NAME", "Fence Cutters", "OBJECT_DEF")?;
        if inventory::field(profile, "OBJECT_EQUIP_LOC")? != "EQL_RIGHT_HAND"
            || inventory::field(profile, "OBJECT_CARRY_STYLE")? != "CSTYLE_ONE_HAND"
        {
            return Err("экипировка кусачек не поддерживается".into());
        }
        let auto_equip = match inventory::field(profile, "OBJECT_AUTO_EQUIP")? {
            "TRUE" => true,
            "FALSE" => false,
            _ => return Err("неверный auto-equip кусачек".into()),
        };
        let tool_model = player::model_path(catalog, inventory::field(profile, "OBJECT_MODEL")?)?;
        let tools: Vec<_> = map
            .instances
            .iter()
            .filter(|i| {
                i.inventory && i.name.eq_ignore_ascii_case("Fence Cutters") && i.path == tool_model
            })
            .collect();
        let [tool] = tools.as_slice() else {
            return Err("нужны одни кусачки в MTF".into());
        };
        let fences: Vec<_> = map
            .instances
            .iter()
            .filter(|i| i.name.eq_ignore_ascii_case(EVENT_NAME))
            .collect();
        let [target] = fences.as_slice() else {
            return Err("нужна одна учебная сетка END".into());
        };
        let cut_path = player::model_path(catalog, CUT_MODEL)?;
        let cut_model = scene::prepare(catalog, &cut_path, false)?;
        let FenceGeometry {
            bounds: open_boxes,
            actions,
        } = node_data(catalog, &cut_path)?;
        if open_boxes.len() != 2 {
            return Err("cut-модель без двух боковых границ прохода".into());
        }
        let closed_actions = node_data(catalog, &target.path)?.actions;
        let frame = Mat4::from_scale(Vec3::splat(WORLD_SCALE)) * target.transform.to_matrix();
        let sides = closed_actions
            .ok_or("нет Act01/Act02 закрытой сетки")?
            .map(|v| frame.transform_point3(v));
        let crawl_sides = actions
            .ok_or("нет Act01/Act02 прохода")?
            .map(|v| frame.transform_point3(v));
        if flat(sides[0] - sides[1]).length() < 0.5
            || flat(crawl_sides[0] - crawl_sides[1]).length() < 0.5
        {
            return Err("точки сетки совпадают".into());
        }
        let templates = config_lines(&catalog.read("config/object_templates.txt")?)?;
        let names: BTreeSet<_> = templates
            .iter()
            .filter(|l| l.words.len() == 2 && l.words[1] == "Structure_Fence")
            .map(|l| l.words[0].to_ascii_lowercase())
            .collect();
        let mut barriers = Vec::new();
        for i in map
            .instances
            .iter()
            .filter(|i| names.contains(&i.name.to_ascii_lowercase()))
        {
            let boxes = node_data(catalog, &i.path)?.bounds;
            barriers.push(Barrier {
                id: i.id,
                inverse: (Mat4::from_scale(Vec3::splat(WORLD_SCALE)) * i.transform.to_matrix())
                    .inverse(),
                boxes,
            });
        }
        if !barriers.iter().any(|b| b.id == target.id) {
            return Err("сетка не относится к Structure_Fence".into());
        }
        let r = p
            .rig
            .as_mut()
            .ok_or("кусачки готовятся до создания сцены")?;
        let first = r.parts.len();
        rig::prepare_weapon(catalog, r, &tool_model, "Fence Cutters")?;
        p.equipment_parts
            .push((tool_model.clone(), first..r.parts.len()));
        let FenceClips {
            cut: a,
            cut_frame: af,
            swap: b,
            swap_frame: bf,
            crawl: c,
            reps,
        } = cut_sequences(catalog, &r.bones)?;
        let index = p.clips.len();
        let cut_callback = af as f32 / a.fps;
        let swap_callback = bf as f32 / b.fps;
        let source = serde_json::json!({"profile":path,"profile_origin":catalog.source_label(path)?,"tool_id":tool.id,"tool_position_converted_source_units":tool.transform.translation.to_array(),"tool_rotation_converted_xyzw":tool.transform.rotation.to_array(),"tool_placement":"original MTF","auto_equip":auto_equip,"fence_id":target.id,"fence_name":target.name,"closed_model":target.path,"cut_model":cut_path,"cut_model_origin":catalog.source_label(&cut_path)?,"replacement_rule_original_verified":false,"cut_sequence":"MSEQ_CUT_FENCE","cut_callback_frame":af,"cut_fps":a.fps,"swap_callback_frame":bf,"swap_fps":b.fps,"crawl_sequence":"MSEQ_CROUCH_CRAWL_THROUGH_FENCE","crawl_repetitions":reps,"crawl_fps":c.fps,"crawl_duration_seconds":c.duration(),"closed_action_points_metres":sides.map(|p|p.to_array()),"open_action_points_metres":crawl_sides.map(|p|p.to_array()),"action_point_semantics_original_verified":false,"movement_collision":"SMF ground-level bounds, 0.20 m player radius; no general obstacle navigation","collision_radius_original_verified":false,"crawl_path_scaling_original_verified":false,"cut_sound_reconstructed":false,"primary_restore_original_verified":false});
        p.clips.extend([a, b, c]);
        inv.cutters_model = Some(tool_model.clone());
        let tool_transform =
            Mat4::from_scale(Vec3::splat(WORLD_SCALE)) * tool.transform.to_matrix();
        let tool_triangles = triangles(&map.models[&tool_model])?;
        let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for t in &tool_triangles {
            for v in t {
                let v = tool_transform.transform_point3(*v);
                low = low.min(v);
                high = high.max(v);
            }
        }
        Ok(Self {
            tool_id: tool.id,
            tool_model,
            tool_position: tool.transform.translation * WORLD_SCALE,
            tool_center: (low + high) * 0.5,
            collected: false,
            pickup_ordered: false,
            auto_equip,
            id: target.id,
            center: (crawl_sides[0] + crawl_sides[1]) * 0.5 + Vec3::Y * 0.9,
            sides,
            crawl_sides,
            cut: false,
            cuts: 0,
            crossings: 0,
            blocked_moves: 0,
            message: String::new(),
            source,
            phase: None,
            cut_clip: index,
            swap_clip: index + 1,
            crawl_clip: index + 2,
            cut_callback,
            swap_callback,
            swap_pending: false,
            resume_rifle: false,
            resume_weapon: None,
            tool_transform,
            fence_transform: frame,
            tool_triangles,
            fence_triangles: triangles(&map.models[&target.path])?,
            barriers,
            open_boxes,
            cut_model: Some(cut_model),
        })
    }
    pub fn has_order(&self) -> bool {
        self.pickup_ordered || self.phase.is_some()
    }
    pub fn crossing(&self) -> bool {
        matches!(self.phase, Some(Phase::Crossing(_)))
    }
    pub fn cancel(&mut self, p: &mut player::Player) {
        if self.pickup_ordered && p.pickup_seconds.is_some() {
            p.finish_pickup();
        }
        self.pickup_ordered = false;
        self.phase = None;
        if p.action
            .is_some_and(|(i, _)| [self.cut_clip, self.swap_clip, self.crawl_clip].contains(&i))
        {
            p.action = None;
            p.playback.select(&p.clips, 0);
        }
        if self.swap_pending && p.weapon.as_ref() == Some(&self.tool_model) {
            p.weapon = self.resume_weapon.clone();
            p.rifle_equipped = self.resume_rifle;
        }
        self.swap_pending = false;
    }
    pub fn restart(&mut self, p: &mut player::Player, inv: &mut Inventory) {
        self.cancel(p);
        if p.weapon.as_ref() == Some(&self.tool_model) {
            p.weapon = None;
        }
        self.collected = false;
        inv.owns_cutters = false;
        self.cut = false;
        self.cuts = 0;
        self.crossings = 0;
        self.blocked_moves = 0;
        self.message.clear();
        self.resume_weapon = None;
        self.resume_rifle = false;
    }
    pub fn order_pickup(&mut self, p: &mut player::Player, grid: &TerrainGrid) -> bool {
        if self.collected || p.playback.paused {
            return false;
        }
        self.cancel(p);
        if p.order(grid, self.tool_position, false) {
            self.pickup_ordered = true;
            true
        } else {
            false
        }
    }
    fn nearest(&self, p: Vec3, points: [Vec3; 2]) -> usize {
        usize::from(flat(p - points[1]).length_squared() < flat(p - points[0]).length_squared())
    }
    pub fn order_fence(&mut self, p: &mut player::Player, grid: &TerrainGrid) -> bool {
        if p.playback.paused {
            return false;
        }
        if !self.cut && !self.collected {
            self.message = "Нужны кусачки".into();
            return false;
        }
        self.cancel(p);
        let points = if self.cut {
            self.crawl_sides
        } else {
            self.sides
        };
        let side = self.nearest(p.position, points);
        if !p.order(grid, points[side], false) {
            return false;
        }
        self.phase = Some(if self.cut {
            Phase::ToCross(side)
        } else {
            Phase::ToCut(side)
        });
        self.message.clear();
        true
    }
    pub fn tool_clicked(
        &self,
        camera: &Camera,
        tr: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> bool {
        !self.collected
            && geometry.clear_line_ignoring(tr.translation(), self.tool_center, Some(self.tool_id))
            && self.mesh_clicked(
                camera,
                tr,
                cursor,
                &self.tool_triangles,
                self.tool_transform,
                self.tool_center,
                14.0,
            )
    }
    pub fn fence_clicked(
        &self,
        camera: &Camera,
        tr: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> bool {
        geometry.clear_line_ignoring(tr.translation(), self.center, Some(self.id))
            && self.mesh_clicked(
                camera,
                tr,
                cursor,
                &self.fence_triangles,
                self.fence_transform,
                self.center,
                20.0,
            )
    }
    #[allow(clippy::too_many_arguments)]
    fn mesh_clicked(
        &self,
        camera: &Camera,
        tr: &GlobalTransform,
        cursor: Vec2,
        triangles: &[[Vec3; 3]],
        frame: Mat4,
        center: Vec3,
        radius: f32,
    ) -> bool {
        let Ok(ray) = camera.viewport_to_world(tr, cursor) else {
            return false;
        };
        triangles.iter().any(|t| {
            pickup::ray_triangle(
                ray.origin,
                ray.direction.as_vec3(),
                t.map(|v| frame.transform_point3(v)),
            )
            .is_some()
        }) || camera
            .world_to_viewport(tr, center)
            .is_ok_and(|v| v.distance(cursor) <= radius)
    }
    pub fn movement_hit(&self, from: Vec3, to: Vec3) -> Option<f32> {
        if !from.is_finite() || !to.is_finite() || flat(to - from).length_squared() < 1e-10 {
            return None;
        }
        self.barriers
            .iter()
            .filter_map(|b| {
                let boxes = if b.id == self.id && self.cut {
                    &self.open_boxes
                } else {
                    &b.boxes
                };
                let a = flat(b.inverse.transform_point3(from));
                let z = flat(b.inverse.transform_point3(to));
                boxes
                    .iter()
                    .filter_map(|bounds| bounds.hit(a, z, 20.0))
                    .min_by(f32::total_cmp)
            })
            .min_by(f32::total_cmp)
    }
    pub fn guard_movement(&mut self, p: &mut player::Player, previous: Vec3) {
        if let Some(t) = self.movement_hit(previous, p.position) {
            p.position = previous.lerp(p.position, (t - 0.001).max(0.0));
            p.destination = None;
            p.playback.select(&p.clips, 0);
            self.blocked_moves += 1;
            self.message = "Сетка перекрывает движение".into();
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        p: &mut player::Player,
        inv: &mut Inventory,
        t: &mut Tutorial,
        geometry: &Pickup,
        grid: &TerrainGrid,
        dt: f32,
    ) {
        if inv.open
            || t.pausing()
            || p.playback.paused
            || p.changing_posture()
            || !dt.is_finite()
            || dt < 0.0
        {
            return;
        }
        if self.collected && !self.swap_pending && p.weapon.as_ref() != Some(&self.tool_model) {
            self.resume_weapon = p.weapon.clone();
            self.resume_rifle = p.rifle_equipped;
        }
        if self.pickup_ordered {
            let d = flat(p.position - self.tool_position).length();
            if !self.collected && p.pickup_seconds.is_none() && d <= 0.5 {
                p.begin_pickup();
            }
            if !self.collected && p.pickup_seconds.is_none() && p.destination.is_none() && d > 0.5 {
                self.pickup_ordered = false;
            }
            if let Some(seconds) = p.pickup_seconds {
                if !self.collected
                    && seconds >= p.pickup_callback(geometry.callback_seconds)
                    && d <= 0.5
                {
                    self.collected = true;
                    inv.owns_cutters = true;
                    self.resume_weapon = p.weapon.clone();
                    self.resume_rifle = p.rifle_equipped;
                    if self.auto_equip {
                        inv.equip(Weapon::Cutters, p);
                    }
                    t.fire(Event::ObjectPickedUp, "Fence Cutters");
                    t.tick(0.0, false);
                }
                if seconds >= p.pickup_duration() {
                    p.finish_pickup();
                    self.pickup_ordered = false;
                }
            }
        }
        let Some(phase) = self.phase else {
            return;
        };
        match phase {
            Phase::ToCut(side) | Phase::ToCross(side) => {
                let cross = matches!(phase, Phase::ToCross(_));
                let point = if cross {
                    self.crawl_sides[side]
                } else {
                    self.sides[side]
                };
                if flat(point - p.position).length() <= 0.15 {
                    let facing = self.crawl_sides[1 - side] - p.position;
                    p.heading = (-facing.x).atan2(-facing.z);
                    if cross {
                        p.begin_action(self.crawl_clip);
                        self.phase = Some(Phase::Crossing(side));
                    } else if geometry.clear_line_ignoring(
                        p.camera_target(),
                        self.center,
                        Some(self.id),
                    ) {
                        if p.weapon.as_ref() != Some(&self.tool_model) {
                            self.resume_weapon = p.weapon.clone();
                            self.resume_rifle = p.rifle_equipped;
                        }
                        inv.equip(Weapon::Cutters, p);
                        self.swap_pending = true;
                        p.begin_action(self.cut_clip);
                        self.phase = Some(Phase::Cutting);
                    } else {
                        self.message = "Препятствие перед сеткой".into();
                        self.cancel(p);
                    }
                } else if p.destination.is_none() {
                    self.cancel(p);
                }
            }
            Phase::Cutting => {
                let Some((index, seconds)) = p.action.filter(|(i, _)| *i == self.cut_clip) else {
                    self.cancel(p);
                    return;
                };
                if !self.cut && seconds >= self.cut_callback {
                    self.cut = true;
                    self.cuts += 1;
                    t.fire(Event::FenceCut, EVENT_NAME);
                    t.tick(0.0, false);
                }
                if seconds >= p.clips[index].duration() {
                    p.begin_action(self.swap_clip);
                    self.phase = Some(Phase::Swapping);
                }
            }
            Phase::Swapping => {
                let Some((index, seconds)) = p.action.filter(|(i, _)| *i == self.swap_clip) else {
                    self.cancel(p);
                    return;
                };
                if self.swap_pending && seconds >= self.swap_callback {
                    if p.weapon.as_ref() == Some(&self.tool_model) {
                        p.weapon = self.resume_weapon.clone();
                        p.rifle_equipped = self.resume_rifle;
                    }
                    self.swap_pending = false;
                }
                if seconds >= p.clips[index].duration() {
                    self.cancel(p);
                }
            }
            Phase::Crossing(side) => {
                let Some((index, seconds)) = p.action.filter(|(i, _)| *i == self.crawl_clip) else {
                    self.cancel(p);
                    return;
                };
                let clip = &p.clips[index];
                let ratio = (clip.displacement_at(seconds).length()
                    / clip.cycle_displacement().length())
                .clamp(0.0, 1.0);
                let mut next = self.crawl_sides[side].lerp(self.crawl_sides[1 - side], ratio);
                next.y = player::surface(grid, next).unwrap_or(p.position.y);
                if self.movement_hit(p.position, next).is_some() {
                    self.message = "Проход перекрыт".into();
                    self.cancel(p);
                    return;
                }
                p.position = next;
                if seconds >= clip.duration() {
                    self.crossings += 1;
                    self.cancel(p);
                }
            }
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"collected":self.collected,"pickup_ordered":self.pickup_ordered,"cut":self.cut,"cuts":self.cuts,"crossings":self.crossings,"blocked_moves":self.blocked_moves,"phase":self.phase.map(|p|format!("{p:?}")),"tool_position_metres":self.tool_position.to_array(),"tool_center_metres":self.tool_center.to_array(),"fence_center_metres":self.center.to_array(),"message":self.message,"source":self.source})
    }
}
#[derive(Component)]
pub(crate) struct CutRoot;
#[derive(Component)]
pub(crate) struct ToolRing;
pub fn setup(
    mut commands: Commands,
    f: Option<ResMut<Fence>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut f) = f else {
        return;
    };
    let Some(model) = f.cut_model.take() else {
        return;
    };
    let textures = model
        .images
        .into_iter()
        .map(|(p, t)| (p, images.add(t.image)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let parent = commands
        .spawn((
            player::SceneEntity,
            CutRoot,
            Transform::from_matrix(f.fence_transform),
            Visibility::Hidden,
            Name::new("Fence cut passage"),
        ))
        .id();
    for part in model.parts {
        commands.spawn((
            Mesh3d(meshes.add(part.mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color_texture: part.texture.as_ref().map(|p| textures[p].clone()),
                alpha_mode: part.alpha_mode,
                cull_mode: None,
                perceptual_roughness: 0.95,
                ..default()
            })),
            Transform::default(),
            ChildOf(parent),
        ));
    }
    commands.spawn((
        player::SceneEntity,
        ToolRing,
        Mesh3d(meshes.add(Torus::new(0.16, 0.19))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.8, 0.12),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(f.tool_position + Vec3::Y * 0.04),
        Visibility::Inherited,
    ));
}
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    f: Option<ResMut<Fence>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<ResMut<Inventory>>,
    t: Option<Res<Tutorial>>,
) {
    if inv.as_ref().is_some_and(|i| i.open || i.consumed_input)
        || t.is_some_and(|t| t.active.is_some() || t.consumed_input)
    {
        return;
    }
    if let (Some(mut f), Some(mut p), Some(mut inv)) = (f, p, inv) {
        if keys.just_pressed(KeyCode::Home) {
            f.restart(&mut p, &mut inv);
        } else if keys.just_pressed(KeyCode::KeyH) && p.selected {
            f.cancel(&mut p);
        }
    }
}
#[allow(clippy::type_complexity)]
pub fn sync(
    f: Option<Res<Fence>>,
    mut objects: Query<
        (&pickup::MapObject, &mut Visibility),
        (Without<CutRoot>, Without<ToolRing>),
    >,
    mut cut: Query<&mut Visibility, (With<CutRoot>, Without<ToolRing>)>,
    mut rings: Query<&mut Visibility, (With<ToolRing>, Without<CutRoot>)>,
) {
    let Some(f) = f else {
        return;
    };
    for (id, mut v) in &mut objects {
        if id.0 == f.id {
            *v = if f.cut {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        } else if id.0 == f.tool_id {
            *v = if f.collected {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
    for mut v in &mut cut {
        *v = if f.cut {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for mut v in &mut rings {
        *v = if f.collected {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        Fence,
        player::Player,
        Inventory,
        Tutorial,
        Pickup,
        TerrainGrid,
    ) {
        let (item, mut p, t, grid) = pickup::tests::fixture();
        for name in ["cut", "equip", "crawl"] {
            p.clips.push(Clip {
                name: name.into(),
                fps: 10.0,
                frames: (0..11)
                    .map(|i| crate::motion::Pose {
                        bones: vec![],
                        root: Vec3::Y
                            * if name == "crawl" {
                                i as f32 * 10.0
                            } else {
                                0.0
                            },
                    })
                    .collect(),
            });
        }
        let points = [Vec3::new(0.5, 0.0, -0.45), Vec3::new(0.5, 0.0, -1.35)].map(|mut v| {
            v.y = player::surface(&grid, v).unwrap();
            v
        });
        let frame =
            Mat4::from_translation(Vec3::new(0.0, 0.0, -0.9)) * Mat4::from_scale(Vec3::splat(0.01));
        let box2 = |x0, x1| Box2 {
            low: Vec2::new(x0, -5.0),
            high: Vec2::new(x1, 5.0),
        };
        let f = Fence {
            tool_id: [1, 99],
            tool_model: "cutters".into(),
            tool_position: p.position,
            tool_center: p.camera_target(),
            collected: false,
            pickup_ordered: false,
            auto_equip: true,
            id: [1, 88],
            center: Vec3::new(0.5, 2.7, -0.9),
            sides: points,
            crawl_sides: points,
            cut: false,
            cuts: 0,
            crossings: 0,
            blocked_moves: 0,
            message: String::new(),
            source: serde_json::Value::Null,
            phase: None,
            cut_clip: 5,
            swap_clip: 6,
            crawl_clip: 7,
            cut_callback: 0.6,
            swap_callback: 0.4,
            swap_pending: false,
            resume_rifle: false,
            resume_weapon: None,
            tool_transform: Mat4::IDENTITY,
            fence_transform: frame,
            tool_triangles: vec![],
            fence_triangles: vec![],
            barriers: vec![Barrier {
                id: [1, 88],
                inverse: frame.inverse(),
                boxes: vec![box2(-100.0, 200.0)],
            }],
            open_boxes: vec![box2(-100.0, 0.0), box2(100.0, 200.0)],
            cut_model: None,
        };
        let mut inv = inventory::tests::inventory();
        inv.collected = true;
        inv.rounds = 9;
        inv.cutters_model = Some("cutters".into());
        inv.equip(Weapon::Bda, &mut p);
        (f, p, inv, t, item, grid)
    }
    fn start_cut(
        f: &mut Fence,
        p: &mut player::Player,
        inv: &mut Inventory,
        t: &mut Tutorial,
        item: &Pickup,
        grid: &TerrainGrid,
    ) {
        f.collected = true;
        inv.owns_cutters = true;
        p.position = f.sides[0];
        assert!(f.order_fence(p, grid));
        f.update(p, inv, t, item, grid, 0.0);
        assert_eq!(p.action, Some((5, 0.0)));
    }
    #[test]
    fn tool_callback_once_and_auto_equip_preserves_owned_weapons_and_ammunition() {
        for auto in [true, false] {
            let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
            f.auto_equip = auto;
            assert!(f.order_pickup(&mut p, &grid));
            f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.0);
            p.step(&ButtonInput::default(), 0.29, &grid);
            f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.29);
            assert!(!f.collected);
            p.step(&ButtonInput::default(), 0.02, &grid);
            for _ in 0..2 {
                f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.02);
            }
            assert!(f.collected && inv.owns_cutters && inv.owns_knife && inv.collected);
            assert_eq!(inv.rounds, 9);
            assert_eq!(
                inv.weapon(&p),
                Some(if auto { Weapon::Cutters } else { Weapon::Bda })
            );
            assert_eq!(
                t.events
                    .iter()
                    .filter(|e| e.as_str() == "CET_OBJECT_PICKED_UP:Fence Cutters")
                    .count(),
                1
            );
            p.step(&ButtonInput::default(), 1.0, &grid);
            f.update(&mut p, &mut inv, &mut t, &item, &grid, 1.0);
            assert!(!f.has_order());
            assert!(p.pickup_seconds.is_none());
        }
    }
    #[test]
    fn cancelling_pickup_stops_its_animation_and_prevents_late_collection() {
        let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
        f.order_pickup(&mut p, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.0);
        assert!(p.pickup_seconds.is_some());
        f.cancel(&mut p);
        p.step(&ButtonInput::default(), 1.0, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 1.0);
        assert!(!f.collected && !f.has_order() && !inv.owns_cutters);
        assert!(p.pickup_seconds.is_none());
        assert_eq!(inv.weapon(&p), Some(Weapon::Bda));
    }
    #[test]
    fn cut_callback_opens_once_then_second_motion_restores_previous_weapon() {
        let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
        start_cut(&mut f, &mut p, &mut inv, &mut t, &item, &grid);
        assert_eq!(inv.weapon(&p), Some(Weapon::Cutters));
        p.step(&ButtonInput::default(), 0.59, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.59);
        assert!(!f.cut);
        p.step(&ButtonInput::default(), 0.02, &grid);
        for _ in 0..2 {
            f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.02);
        }
        assert!(f.cut);
        assert_eq!(f.cuts, 1);
        assert_eq!(
            t.events
                .iter()
                .filter(|e| e.as_str() == "CET_FENCE_CUT:AlStFn-Consertinafence_END")
                .count(),
            1
        );
        assert!(t.completed.is_empty());
        p.step(&ButtonInput::default(), 1.0, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 1.0);
        assert_eq!(p.action, Some((6, 0.0)));
        p.step(&ButtonInput::default(), 0.39, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.39);
        assert_eq!(inv.weapon(&p), Some(Weapon::Cutters));
        p.step(&ButtonInput::default(), 0.02, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.02);
        assert_eq!(inv.weapon(&p), Some(Weapon::Bda));
        assert_eq!(inv.rounds, 9);
        p.step(&ButtonInput::default(), 1.0, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 1.0);
        assert!(!f.has_order());
        assert!(p.action.is_none());
    }
    #[test]
    fn cancelling_before_callback_does_not_cut_and_after_callback_keeps_passage() {
        for seconds in [0.4, 0.7] {
            let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
            start_cut(&mut f, &mut p, &mut inv, &mut t, &item, &grid);
            p.step(&ButtonInput::default(), seconds, &grid);
            f.update(&mut p, &mut inv, &mut t, &item, &grid, seconds);
            f.cancel(&mut p);
            assert_eq!(f.cut, seconds >= 0.6);
            assert!(!f.has_order());
            assert!(p.action.is_none());
            assert!(inv.owns_cutters);
            assert_eq!(inv.weapon(&p), Some(Weapon::Bda));
        }
    }
    #[test]
    fn manual_primary_change_is_preserved_when_cancelling_tool_sequence() {
        let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
        start_cut(&mut f, &mut p, &mut inv, &mut t, &item, &grid);
        inv.equip(Weapon::Knife, &mut p);
        f.cancel(&mut p);
        assert_eq!(inv.weapon(&p), Some(Weapon::Knife));
    }
    #[test]
    fn later_weapon_equipment_updates_restore_choice_after_early_tool_pickup() {
        let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
        f.collected = true;
        inv.owns_cutters = true;
        f.resume_weapon = None;
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.0);
        inv.equip(Weapon::Cutters, &mut p);
        p.position = f.sides[0];
        f.order_fence(&mut p, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.0);
        f.cancel(&mut p);
        assert_eq!(inv.weapon(&p), Some(Weapon::Bda));
        assert!(inv.owns_cutters);
    }
    #[test]
    fn no_tool_prevents_cut_but_open_fence_can_be_crossed_without_tool() {
        let (mut f, mut p, _, _, _, grid) = fixture();
        assert!(!f.order_fence(&mut p, &grid));
        assert!(!f.has_order());
        f.cut = true;
        assert!(f.order_fence(&mut p, &grid));
    }
    #[test]
    fn pause_inventory_help_and_invalid_time_hold_callback() {
        for mode in 0..5 {
            let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
            start_cut(&mut f, &mut p, &mut inv, &mut t, &item, &grid);
            p.action = Some((5, 0.7));
            let dt = match mode {
                0 => {
                    inv.open = true;
                    1.0
                }
                1 => {
                    t.active = Some("welcome".into());
                    1.0
                }
                2 => {
                    p.playback.paused = true;
                    1.0
                }
                3 => f32::NAN,
                _ => -1.0,
            };
            f.update(&mut p, &mut inv, &mut t, &item, &grid, dt);
            assert!(!f.cut);
            assert_eq!(f.cuts, 0);
        }
    }
    #[test]
    fn closed_fence_blocks_long_steps_and_open_hole_allows_only_central_passage() {
        let (mut f, _, _, _, _, _) = fixture();
        assert!(f.movement_hit(f.sides[0], f.sides[1]).is_some());
        f.cut = true;
        assert!(f.movement_hit(f.sides[0], f.sides[1]).is_none());
        for x in [-0.2, 0.1, 0.9, 1.2] {
            let a = Vec3::new(x, 0.0, -0.4);
            let b = Vec3::new(x, 0.0, -1.4);
            assert!(f.movement_hit(a, b).is_some());
        }
    }
    #[test]
    fn collision_clamps_position_clears_order_and_other_barriers_stay_closed() {
        let (mut f, mut p, _, _, _, _) = fixture();
        let from = f.sides[0];
        p.position = f.sides[1];
        p.destination = Some(p.position);
        f.guard_movement(&mut p, from);
        assert_eq!(f.blocked_moves, 1);
        assert!(p.destination.is_none());
        assert!(p.position.z > -0.66);
        let frame =
            Mat4::from_translation(Vec3::new(0.0, 0.0, -1.7)) * Mat4::from_scale(Vec3::splat(0.01));
        f.barriers.push(Barrier {
            id: [1, 77],
            inverse: frame.inverse(),
            boxes: vec![Box2 {
                low: Vec2::new(-100.0, -5.0),
                high: Vec2::new(200.0, 5.0),
            }],
        });
        f.cut = true;
        assert!(f.movement_hit(f.sides[0], f.sides[1]).is_none());
        assert!(
            f.movement_hit(f.sides[1], Vec3::new(0.5, 0.0, -2.0))
                .is_some()
        );
    }
    #[test]
    fn rotated_and_translated_fence_uses_original_local_bounds() {
        let (mut f, _, _, _, _, _) = fixture();
        let frame = Mat4::from_translation(Vec3::new(10.0, 0.0, -10.0))
            * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
            * Mat4::from_scale(Vec3::splat(0.01));
        f.barriers[0].inverse = frame.inverse();
        let a = frame.transform_point3(Vec3::new(50.0, 0.0, -100.0));
        let b = frame.transform_point3(Vec3::new(50.0, 0.0, 100.0));
        assert!(f.movement_hit(a, b).is_some());
        f.cut = true;
        assert!(f.movement_hit(a, b).is_none());
    }
    #[test]
    fn slab_collision_handles_parallel_tangential_and_initial_inside_paths() {
        let b = Box2 {
            low: Vec2::ZERO,
            high: Vec2::ONE,
        };
        assert!(
            b.hit(Vec2::new(-1.0, 2.0), Vec2::new(2.0, 2.0), 0.0)
                .is_none()
        );
        assert!(
            b.hit(Vec2::new(-1.0, 1.0), Vec2::new(2.0, 1.0), 0.0)
                .is_some()
        );
        assert!(b.hit(Vec2::splat(0.5), Vec2::splat(2.0), 0.0).is_none());
        assert_eq!(b.hit(Vec2::splat(0.5), Vec2::splat(0.7), 0.0), Some(0.0));
    }
    #[test]
    fn original_crawl_repetitions_accumulate_root_without_cycle_jumps() {
        let (_, mut p, _, _, _, _) = fixture();
        let c = &mut p.clips[7];
        repeat_crawl(c, 5).unwrap();
        assert_eq!(c.frames.len(), 51);
        assert_eq!(c.duration(), 5.0);
        assert_eq!(c.cycle_displacement(), Vec3::Y * 500.0);
        for i in 1..5 {
            let before = c.displacement_at(i as f32 - 0.001);
            let after = c.displacement_at(i as f32 + 0.001);
            assert!((after - before).length() < 0.21);
        }
    }
    #[test]
    fn crawling_moves_continuously_along_both_original_action_point_directions() {
        for side in [0, 1] {
            let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
            f.cut = true;
            p.position = f.crawl_sides[side];
            assert!(f.order_fence(&mut p, &grid));
            f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.0);
            let start = p.position;
            p.step(&ButtonInput::default(), 0.5, &grid);
            f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.5);
            assert!(
                (flat(p.position) - flat(start.lerp(f.crawl_sides[1 - side], 0.5))).length()
                    < 0.0001
            );
            assert_eq!(f.crossings, 0);
            assert_eq!(inv.rounds, 9);
            p.step(&ButtonInput::default(), 0.5, &grid);
            f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.5);
            assert_eq!(f.crossings, 1);
            assert!(!f.has_order());
            assert!(p.position.distance(f.crawl_sides[1 - side]) < 0.001);
        }
    }
    #[test]
    fn cancelled_crawl_stays_at_current_position_and_does_not_count_crossing() {
        let (mut f, mut p, mut inv, mut t, item, grid) = fixture();
        f.cut = true;
        p.position = f.crawl_sides[0];
        f.order_fence(&mut p, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.0);
        p.step(&ButtonInput::default(), 0.5, &grid);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 0.5);
        let middle = p.position;
        f.cancel(&mut p);
        f.update(&mut p, &mut inv, &mut t, &item, &grid, 1.0);
        assert_eq!(p.position, middle);
        assert_eq!(f.crossings, 0);
        assert!(f.cut);
    }
    #[test]
    fn restart_closes_fence_returns_tool_and_clears_tool_equipment() {
        let (mut f, mut p, mut inv, _, _, _) = fixture();
        f.collected = true;
        inv.owns_cutters = true;
        f.cut = true;
        f.cuts = 1;
        f.crossings = 2;
        inv.equip(Weapon::Cutters, &mut p);
        f.restart(&mut p, &mut inv);
        assert!(!f.collected && !f.cut && !inv.owns_cutters);
        assert_eq!(f.cuts + f.crossings, 0);
        assert!(p.weapon.is_none());
        assert!(f.movement_hit(f.sides[0], f.sides[1]).is_some());
    }
    #[test]
    fn scene_visibility_switches_cut_model_and_removes_picked_ground_tool() {
        let (f, _, _, _, _, _) = fixture();
        let mut app = App::new();
        app.insert_resource(f).add_systems(Update, sync);
        let old = app
            .world_mut()
            .spawn((pickup::MapObject([1, 88]), Visibility::Inherited))
            .id();
        let tool = app
            .world_mut()
            .spawn((pickup::MapObject([1, 99]), Visibility::Inherited))
            .id();
        let cut = app.world_mut().spawn((CutRoot, Visibility::Hidden)).id();
        let ring = app
            .world_mut()
            .spawn((ToolRing, Visibility::Inherited))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(cut),
            Some(&Visibility::Hidden)
        );
        {
            let mut f = app.world_mut().resource_mut::<Fence>();
            f.cut = true;
            f.collected = true;
        }
        app.update();
        for entity in [old, tool, ring] {
            assert_eq!(
                app.world().get::<Visibility>(entity),
                Some(&Visibility::Hidden)
            );
        }
        assert_eq!(
            app.world().get::<Visibility>(cut),
            Some(&Visibility::Inherited)
        );
    }
}
