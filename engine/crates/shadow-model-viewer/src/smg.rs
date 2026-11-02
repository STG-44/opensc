//! UZI лежит в оригинальном MTF. Подбор и экипировка — разные действия.
use crate::{
    inventory::{Inventory, Weapon},
    map_scene::{MapScene, WORLD_SCALE},
    pickup::{MapObject, Pickup},
    player,
    tutorial::Tutorial,
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use shadow_formats::{AssetCatalog, CampaignEventType as Event, TerrainGrid};
use std::error::Error;

#[derive(Resource)]
pub struct Uzi {
    pub id: [i32; 2],
    pub position: Vec3,
    pub center: Vec3,
    pub collected: bool,
    pub ordered: bool,
    pub source: serde_json::Value,
    triangles: Vec<[Vec3; 3]>,
}
impl Uzi {
    pub fn prepare(
        _catalog: &AssetCatalog,
        scene: &MapScene,
        inv: &Inventory,
    ) -> Result<Self, Box<dyn Error>> {
        let gun = inv.uzi.as_ref().ok_or("не подготовлен UZI")?;
        Self::ground_item(scene, "UZI", &gun.model, gun.auto_equip)
    }
    pub(crate) fn ground_item(
        scene: &MapScene,
        name: &str,
        model: &str,
        auto_equip: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let found: Vec<_> = scene
            .instances
            .iter()
            .filter(|i| i.inventory && i.name.eq_ignore_ascii_case(name) && i.path == model)
            .collect();
        let [instance] = found.as_slice() else {
            return Err("нужен один предмет в MTF обучения".into());
        };
        let world = Mat4::from_scale(Vec3::splat(WORLD_SCALE)) * instance.transform.to_matrix();
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        let mut triangles = Vec::new();
        for part in &scene.models[model].parts {
            let Some(VertexAttributeValues::Float32x3(v)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                return Err("нет вершин UZI".into());
            };
            let points: Vec<_> = v
                .iter()
                .map(|p| world.transform_point3(Vec3::from_array(*p)))
                .collect();
            for p in &points {
                low = low.min(*p);
                high = high.max(*p);
            }
            let indices: Vec<_> = part
                .mesh
                .indices()
                .ok_or("нет индексов UZI")?
                .iter()
                .collect();
            for index in indices.as_chunks::<3>().0 {
                triangles.push([points[index[0]], points[index[1]], points[index[2]]]);
            }
        }
        if triangles.is_empty() || !low.is_finite() || !high.is_finite() {
            return Err("пустая геометрия UZI".into());
        }
        let mut position = instance.transform.translation * WORLD_SCALE;
        position.y = player::surface(&scene.grid, position).ok_or("UZI вне рельефа")?;
        Ok(Self {
            id: instance.id,
            position,
            center: (low + high) * 0.5,
            collected: false,
            ordered: false,
            triangles,
            source: serde_json::json!({"id":instance.id,"model":model,"placement":"original MTF","transform_source_units":instance.transform.translation.to_array(),"auto_equip":auto_equip}),
        })
    }
    pub fn order(&mut self, p: &mut player::Player, grid: &TerrainGrid) -> bool {
        if self.collected || p.playback.paused {
            return false;
        }
        self.ordered = p.order(grid, self.position, false);
        self.ordered
    }
    pub fn cancel(&mut self, p: &mut player::Player) {
        if self.ordered {
            p.pickup_seconds = None;
        }
        self.ordered = false;
    }
    pub fn restart(&mut self, p: &mut player::Player) {
        self.cancel(p);
        self.collected = false;
    }
    pub fn update(
        &mut self,
        p: &mut player::Player,
        inv: &mut Inventory,
        t: &mut Tutorial,
        callback: f32,
    ) {
        if !self.ordered || inv.open || p.playback.paused || p.changing_posture() || t.pausing() {
            return;
        }
        let distance = Vec2::new(
            p.position.x - self.position.x,
            p.position.z - self.position.z,
        )
        .length();
        if !self.collected && p.pickup_seconds.is_none() && distance <= 0.5 {
            p.destination = None;
            p.begin_pickup();
        }
        if !self.collected
            && p.pickup_seconds.is_none()
            && p.destination.is_none()
            && distance > 0.5
        {
            self.cancel(p);
            return;
        }
        if let Some(seconds) = p.pickup_seconds {
            if !self.collected && seconds >= p.pickup_callback(callback) && distance <= 0.5 {
                self.collected = true;
                inv.owns_uzi = true;
                if inv.uzi.as_ref().unwrap().auto_equip {
                    inv.equip(Weapon::Uzi, p);
                }
                t.fire(Event::ObjectPickedUp, "UZI");
                t.tick(0.0, false);
            }
            if seconds >= p.pickup_duration() {
                p.finish_pickup();
                self.ordered = false;
            }
        }
    }
    pub fn clicked(
        &self,
        camera: &Camera,
        transform: &GlobalTransform,
        cursor: Vec2,
        geometry: &Pickup,
    ) -> bool {
        if self.collected
            || !geometry.clear_line_ignoring(transform.translation(), self.center, Some(self.id))
        {
            return false;
        }
        let Ok(ray) = camera.viewport_to_world(transform, cursor) else {
            return false;
        };
        self.triangles
            .iter()
            .any(|t| crate::pickup::ray_triangle(ray.origin, ray.direction.as_vec3(), *t).is_some())
            || camera
                .world_to_viewport(transform, self.center)
                .is_ok_and(|p| p.distance(cursor) <= 15.0)
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"collected":self.collected,"ordered":self.ordered,"position_metres":self.position.to_array(),"source":self.source})
    }
}
#[derive(Component)]
pub struct Ring;
pub fn setup(
    mut commands: Commands,
    u: Option<Res<Uzi>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(u) = u else {
        return;
    };
    commands.spawn((
        player::SceneEntity,
        Ring,
        Mesh3d(meshes.add(Torus::new(0.22, 0.25))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.8, 0.12),
            unlit: true,
            ..default()
        })),
        Transform::from_translation(u.position + Vec3::Y * 0.04),
        Visibility::default(),
    ));
}
pub fn input(
    keys: Res<ButtonInput<KeyCode>>,
    u: Option<ResMut<Uzi>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<Res<Inventory>>,
    t: Option<Res<Tutorial>>,
) {
    if inv.is_some_and(|i| i.open || i.consumed_input)
        || t.is_some_and(|t| t.active.is_some() || t.consumed_input)
    {
        return;
    }
    if let (Some(mut u), Some(mut p)) = (u, p) {
        if keys.just_pressed(KeyCode::Home) {
            u.restart(&mut p);
        } else if keys.just_pressed(KeyCode::KeyH) && p.selected {
            u.cancel(&mut p);
        }
    }
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync(
    u: Option<ResMut<Uzi>>,
    p: Option<ResMut<player::Player>>,
    inv: Option<ResMut<Inventory>>,
    t: Option<ResMut<Tutorial>>,
    item: Option<Res<Pickup>>,
    mut objects: Query<(&MapObject, &mut Visibility)>,
    mut rings: Query<&mut Visibility, (With<Ring>, Without<MapObject>)>,
) {
    let (Some(mut u), Some(mut p), Some(mut inv), Some(mut t), Some(item)) = (u, p, inv, t, item)
    else {
        return;
    };
    u.update(&mut p, &mut inv, &mut t, item.callback_seconds);
    for (id, mut visible) in &mut objects {
        if id.0 == u.id {
            *visible = if u.collected {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
    for mut visible in &mut rings {
        *visible = if u.collected {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn inventory() -> Inventory {
        let mut inv = crate::inventory::tests::inventory();
        inv.collected = true;
        inv.rounds = 9;
        inv.uzi = Some(crate::inventory::GunDefinition {
            model: "uzi".into(),
            magazine: 32,
            range: 25.0,
            delay: 0.25,
            base_damage: 27.0,
            damage_mod: 95.0,
            auto_equip: false,
            burst_count: 3,
            burst_round_delay: 0.08,
            sound: Vec::new(),
            source: serde_json::Value::Null,
        });
        inv.uzi_rounds = 32;
        inv
    }
    pub(crate) fn tutorial() -> Tutorial {
        let mission=shadow_formats::MissionDefinition::from_bytes(b"SPECIFY_MTF training.mtf\nSPM_PLAYER_MANAGER_ADD_PLAYER LOCAL_SCRIPTED AI_SMG 6\nDEFINE_OBJECTIVE true guards \"SMG\"\nCER_DEFINITION\nCER_FOR_TYPE CET_IMMEDIATE\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_ENABLE_OBJECTIVE\nCER_BODY_IDENTIFIER guards\nCER_DEFINITION\nCER_FOR_TYPE CET_OBJECT_PICKED_UP\nCER_FOR UZI\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER picked\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\nCER_DEFINITION\nCER_FOR_TYPE CET_ALL_PLAYERS_INFANTRY_KILLED\nCER_FOR AI_SMG\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_NOTIFY_OBJECTIVE_COMPLETE\nCER_BODY_IDENTIFIER guards\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_SHOW_CONTEXT_HELP\nCER_BODY_IDENTIFIER after\nCER_BODY_MS_DELAY 1000\nCER_BODY_DEFINITION\nCER_BODY_TYPE CER_BODY_REMOVE_CER\n").unwrap();
        let helps=shadow_formats::HelpDefinitions::from_bytes(b"HELP_DEF\nHELP_ID picked\nHELP_CONTEXT\nHELP_BODY \"Equip UZI\"\nHELP_DEF\nHELP_ID after\nHELP_CONTEXT\nHELP_BODY \"All six eliminated\"\n").unwrap();
        Tutorial::new(mission, helps).unwrap()
    }
    pub(crate) fn fixture() -> (Uzi, player::Player, Inventory, Tutorial, TerrainGrid) {
        let (_, mut p, _, grid) = crate::pickup::tests::fixture();
        crate::stance::tests::add(&mut p);
        let inv = inventory();
        inv.equip(Weapon::Bda, &mut p);
        let u = Uzi {
            id: [1, -1],
            position: p.position,
            center: p.position,
            collected: false,
            ordered: false,
            source: serde_json::Value::Null,
            triangles: Vec::new(),
        };
        (u, p, inv, tutorial(), grid)
    }
    #[test]
    fn crouched_pickup_fires_once_at_source_callback_and_does_not_auto_equip() {
        let (mut u, mut p, mut inv, mut t, grid) = fixture();
        p.posture = crate::stance::Posture::Crouch;
        u.order(&mut p, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        p.step(&ButtonInput::default(), 0.9, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert!(!u.collected);
        p.step(&ButtonInput::default(), 0.1, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert!(u.collected && inv.owns_uzi);
        assert_eq!(inv.weapon(&p), Some(Weapon::Bda));
        assert_eq!(inv.rounds, 9);
        for _ in 0..10 {
            u.update(&mut p, &mut inv, &mut t, 0.6);
        }
        assert_eq!(
            t.events
                .iter()
                .filter(|e| e.as_str() == "CET_OBJECT_PICKED_UP:UZI")
                .count(),
            1
        );
        assert!(!t.completed.contains("guards"));
        t.tick(1.0, false);
        assert_eq!(t.active.as_deref(), Some("picked"));
    }
    #[test]
    fn auto_equip_override_and_weapon_swap_keep_both_magazines() {
        let (mut u, mut p, mut inv, mut t, grid) = fixture();
        inv.uzi.as_mut().unwrap().auto_equip = true;
        u.order(&mut p, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        p.step(&ButtonInput::default(), 0.6, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert_eq!(inv.weapon(&p), Some(Weapon::Uzi));
        assert!(p.rifle_equipped);
        assert!(inv.take_round_for(Weapon::Uzi));
        assert!(inv.equip(Weapon::Bda, &mut p));
        assert!(!p.rifle_equipped);
        assert!(inv.take_round_for(Weapon::Bda));
        assert_eq!((inv.uzi_rounds, inv.rounds), (31, 8));
        inv.equip(Weapon::Uzi, &mut p);
        assert_eq!((inv.uzi_rounds, inv.rounds), (31, 8));
    }
    #[test]
    fn pause_inventory_and_transition_block_pending_pickup() {
        let (mut u, mut p, mut inv, mut t, grid) = fixture();
        u.order(&mut p, &grid);
        p.begin_pickup();
        p.pickup_seconds = Some(2.0);
        inv.open = true;
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert!(!u.collected);
        inv.open = false;
        p.playback.paused = true;
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert!(!u.collected);
        p.playback.paused = false;
        p.posture_transition = Some((0, 0.0, crate::stance::Posture::Crouch));
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert!(!u.collected);
    }
    #[test]
    fn cancellation_new_order_and_restart_leave_no_pending_pickup_or_ownership() {
        let (mut u, mut p, mut inv, mut t, grid) = fixture();
        u.order(&mut p, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        u.cancel(&mut p);
        p.step(&ButtonInput::default(), 2.0, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert!(!u.collected && !inv.owns_uzi);
        assert!(p.pickup_seconds.is_none());
        u.order(&mut p, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        p.step(&ButtonInput::default(), 0.6, &grid);
        u.update(&mut p, &mut inv, &mut t, 0.6);
        assert!(u.collected);
        u.restart(&mut p);
        inv.restart();
        assert!(!u.collected && !u.ordered && !inv.owns_uzi);
        assert_eq!(inv.uzi_rounds, 32);
    }
}
