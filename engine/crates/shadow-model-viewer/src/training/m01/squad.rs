//! M01 squad state and its shared player rigs.
use super::{Blockout, prepare_order};
use crate::{
    player::{self, Player},
    selection,
    training::{MapCamera, Orbit, Preview, player_view},
};
use bevy::prelude::*;
use shadow_formats::TerrainGrid;
use std::collections::VecDeque;

pub(super) struct Member {
    pub actor: Player,
    pub queued: VecDeque<Vec3>,
    pub render: Option<MemberRig>,
    pub(super) in_formation: bool,
    follow_target: Option<Vec3>,
}

pub(super) struct MemberRig {
    root: Entity,
    bones: Vec<Entity>,
    selection_ring: Entity,
    target_ring: Entity,
    commander_ring: Entity,
}

#[derive(Resource)]
pub(crate) struct Squad {
    pub(super) members: Vec<Member>,
    pub(super) commander: usize,
    trail: VecDeque<Vec3>,
    pub(super) following: Option<usize>,
    pub(crate) paused: bool,
}

impl Squad {
    pub(super) fn new(actors: impl IntoIterator<Item = Player>) -> Self {
        Self {
            members: actors
                .into_iter()
                .enumerate()
                .map(|(index, actor)| Member {
                    actor,
                    queued: VecDeque::new(),
                    render: None,
                    in_formation: index != 0,
                    follow_target: None,
                })
                .collect(),
            commander: 0,
            trail: VecDeque::new(),
            following: None,
            paused: false,
        }
    }

    pub(super) fn mask(&self) -> u8 {
        self.members
            .iter()
            .enumerate()
            .fold(0, |mask, (index, member)| {
                mask | (u8::from(member.actor.selected) << index)
            })
    }

    pub(super) fn select(&mut self, mask: u8) {
        for (index, member) in self.members.iter_mut().enumerate() {
            member.actor.selected = mask & (1 << index) != 0;
        }
        if self
            .following
            .is_some_and(|index| !self.members[index].actor.selected)
        {
            self.following = None;
        }
    }

    pub(super) fn command_actor(&self) -> Option<usize> {
        self.members[self.commander]
            .actor
            .selected
            .then_some(self.commander)
            .or_else(|| self.members.iter().position(|member| member.actor.selected))
    }

    pub(super) fn promote(&mut self) {
        if self.mask().count_ones() == 1
            && let Some(index) = self.command_actor()
        {
            self.commander = index;
            let member = &mut self.members[index];
            if member.in_formation {
                member.clear_orders();
                member.in_formation = false;
            }
            self.regroup();
        }
    }

    pub(super) fn regroup(&mut self) {
        self.trail.clear();
        self.trail
            .push_back(self.members[self.commander].actor.position);
        for (index, member) in self.members.iter_mut().enumerate() {
            if index != self.commander {
                member.clear_orders();
                member.in_formation = true;
            }
        }
    }

    pub(super) fn halt(&mut self, direct: Option<usize>) {
        let selected =
            |index, member: &Member| direct.map_or(member.actor.selected, |i| i == index);
        let commander_stops = selected(self.commander, &self.members[self.commander]);
        if commander_stops {
            self.trail.clear();
        }
        for (index, member) in self.members.iter_mut().enumerate() {
            if selected(index, member) || (commander_stops && member.in_formation) {
                member.clear_orders();
                if !commander_stops {
                    member.in_formation = false;
                }
            }
        }
    }

    pub(super) fn reset(&mut self, orbit: &mut Orbit) {
        for (index, member) in self.members.iter_mut().enumerate() {
            member.actor.reset_spawn();
            member.queued.clear();
            member.follow_target = None;
            member.in_formation = index != 0;
        }
        self.trail.clear();
        self.following = None;
        self.commander = 0;
        self.select(u8::MAX);
        *orbit = player_view(self.members[self.commander].actor.camera_target());
    }

    pub(super) fn order(&mut self, grid: &TerrainGrid, anchor: Vec3, run: bool, append: bool) {
        if player::surface(grid, anchor).is_none() {
            return;
        }
        let group = self.mask().count_ones() > 1 && self.members[self.commander].actor.selected;
        if group
            && self
                .members
                .iter()
                .enumerate()
                .any(|(index, member)| index != self.commander && !member.in_formation)
        {
            self.regroup();
        }
        for (index, member) in self.members.iter_mut().enumerate() {
            if !member.actor.selected || (group && index != self.commander) {
                continue;
            }
            if index != self.commander {
                member.in_formation = false;
                member.follow_target = None;
            }
            if append && member.actor.destination.is_some() {
                if member.queued.len() < 16 {
                    member.queued.push_back(anchor);
                }
            } else if member.actor.order(grid, anchor, run) {
                member.queued.clear();
            }
        }
    }

    pub(super) fn step(
        &mut self,
        grid: &TerrainGrid,
        site: Option<&Blockout>,
        direct: Option<usize>,
        dt: f32,
    ) {
        if self.paused {
            return;
        }

        let commander = &mut self.members[self.commander];
        let previous = commander.actor.position;
        commander.step(grid, site, direct == Some(self.commander), dt);
        let position = commander.actor.position;
        let running = commander.actor.running && commander.actor.destination.is_some();
        let stalled = commander.actor.route.failed;
        if stalled {
            self.trail.clear();
        } else if previous.xz().distance_squared(position.xz()) > 1e-8 {
            if self.trail.is_empty() {
                self.trail.push_back(previous);
            }
            if self
                .trail
                .back()
                .is_some_and(|last| last.xz().distance(position.xz()) >= 0.45)
            {
                self.trail.push_back(position);
                if self.trail.len() > 96 {
                    self.trail.pop_front();
                }
            }
        }

        for (index, member) in self.members.iter_mut().enumerate() {
            if index == self.commander {
                continue;
            }
            let manual = direct == Some(index);
            if member.in_formation && !manual {
                if stalled || member.actor.position.xz().distance(position.xz()) < 1.1 {
                    member.clear_orders();
                } else if let Some(target) = follow_goal(&self.trail, position, member, grid, site)
                    && member
                        .follow_target
                        .is_none_or(|p| p.xz().distance(target.xz()) >= 0.7)
                {
                    let catch_up = member.actor.position.xz().distance(position.xz()) > 4.5;
                    member.actor.order(grid, target, running || catch_up);
                    // Wait for the trail to move before retrying an unreachable point.
                    member.follow_target = Some(target);
                }
            }
            member.step(grid, site, manual, dt);
        }
    }

    pub(crate) fn actors(&self) -> impl Iterator<Item = &Player> {
        self.members.iter().map(|member| &member.actor)
    }
}

impl Member {
    pub(super) fn clear_orders(&mut self) {
        self.actor.destination = None;
        self.actor.route.clear();
        self.queued.clear();
        self.follow_target = None;
    }

    fn step(&mut self, grid: &TerrainGrid, site: Option<&Blockout>, direct: bool, dt: f32) {
        let actor = &mut self.actor;
        if !direct {
            if actor.route.failed {
                self.queued.clear();
            } else if actor.destination.is_none()
                && let Some(next) = self.queued.pop_front()
            {
                actor.order(grid, next, false);
            }
            if let Some(site) = site {
                prepare_order(actor, grid, site);
            }
        }
        let previous = actor.position;
        actor.step(&ButtonInput::default(), dt, grid);
        if let Some(site) = site {
            site.guard(actor, previous, grid);
        }
    }
}

fn follow_goal(
    trail: &VecDeque<Vec3>,
    leader: Vec3,
    member: &Member,
    grid: &TerrainGrid,
    site: Option<&Blockout>,
) -> Option<Vec3> {
    const GAP: f32 = 1.8;
    let oldest = *trail.front()?;
    let clearance = member.actor.body_height() * [1.0, 0.65, 0.25][member.actor.posture.index()];
    let clear_ground = |point| {
        let height = player::surface(grid, point)?;
        let point = point.with_y(height);
        site.is_none_or(|site| site.free(point, clearance))
            .then_some(point)
    };
    // A regroup issued while stationary uses a clear slot near the commander.
    if oldest.xz().distance_squared(leader.xz()) < 1e-4 && trail.len() == 1 {
        let toward_member = (member.actor.position - leader)
            .with_y(0.)
            .try_normalize()
            .unwrap_or(Vec3::X)
            * GAP;
        let side = toward_member.cross(Vec3::Y);
        return [toward_member, side, -side, -toward_member]
            .into_iter()
            .filter_map(|offset| clear_ground(leader + offset))
            .find(|&point| {
                site.is_none_or(|site| {
                    crate::navigation::World::blockout(grid, site, clearance).clear(leader, point)
                })
            });
    }
    let mut remaining = GAP;
    let mut current = leader;
    for &previous in trail.iter().rev() {
        let length = current.xz().distance(previous.xz());
        if length >= remaining {
            let target = current.lerp(previous, remaining / length);
            // A shortcut between sampled frames can graze a jamb. The recorded
            // position is safe when the interpolated point lacks clearance.
            return clear_ground(target).or_else(|| clear_ground(previous));
        }
        remaining -= length;
        current = previous;
    }
    clear_ground(oldest)
}

pub(super) fn spawn(
    mut commands: Commands,
    mut squad: ResMut<Squad>,
    camera: Single<Entity, With<MapCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    let selected = meshes.add(Torus::new(0.48, 0.52));
    let target = meshes.add(Torus::new(0.27, 0.30));
    let commander_circle = meshes.add(Torus::new(0.62, 0.66));
    let gold = materials.add(StandardMaterial {
        base_color: Color::srgb(1., 0.78, 0.2),
        unlit: true,
        ..default()
    });
    for (index, member) in squad.members.iter_mut().enumerate() {
        let color = [Color::srgb(0.32, 1., 0.43), Color::srgb(0.44, 0.78, 1.)][index];
        let marker = materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            ..default()
        });
        let (root, bones) = player::spawn_rig(
            &mut commands,
            &mut member.actor,
            &mut meshes,
            &mut materials,
            &mut images,
            None,
        );
        let mut ring = |handle| {
            commands
                .spawn((
                    player::SceneEntity,
                    Mesh3d(handle),
                    MeshMaterial3d(marker.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id()
        };
        member.render = Some(MemberRig {
            root,
            bones,
            selection_ring: ring(selected.clone()),
            target_ring: ring(target.clone()),
            commander_ring: commands
                .spawn((
                    player::SceneEntity,
                    Mesh3d(commander_circle.clone()),
                    MeshMaterial3d(gold.clone()),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id(),
        });
    }
    player::spawn_status(&mut commands, &mut fonts, *camera);
}

pub(super) fn tick(
    time: Res<Time>,
    preview: Res<Preview>,
    site: Option<Res<Blockout>>,
    input: Res<super::TacticalInput>,
    mut squad: ResMut<Squad>,
    mut orbit: ResMut<Orbit>,
) {
    let Some(grid) = &preview.grid else { return };
    let followed = squad
        .following
        .map(|i| (i, squad.members[i].actor.position));
    squad.step(
        grid,
        site.as_deref(),
        input.direct_actor(),
        time.delta_secs(),
    );
    if let Some((index, previous)) = followed {
        orbit.target += squad.members[index].actor.position - previous;
    }
}

pub(super) fn visuals(
    squad: Res<Squad>,
    input: Res<super::TacticalInput>,
    mut transforms: Query<&mut Transform>,
    mut markers: Query<&mut Visibility>,
    mut overlay: Query<&mut Node, With<selection::Overlay>>,
) {
    for (index, member) in squad.members.iter().enumerate() {
        let Some(rig) = &member.render else { continue };
        let actor = &member.actor;
        if let Ok(mut root) = transforms.get_mut(rig.root) {
            *root = actor.transform();
        }
        for (&entity, &pose) in rig.bones.iter().zip(&actor.playback.pose.bones) {
            if let Ok(mut bone) = transforms.get_mut(entity) {
                *bone = pose;
            }
        }
        let selected = input.direct_actor().map_or(actor.selected, |i| i == index);
        for (entity, point) in [
            (rig.selection_ring, selected.then_some(actor.position)),
            (
                rig.commander_ring,
                (index == squad.commander).then_some(actor.position),
            ),
            (
                rig.target_ring,
                actor
                    .selected
                    .then_some(actor.destination)
                    .flatten()
                    .filter(|_| Some(index) != input.direct_actor() && !member.in_formation),
            ),
        ] {
            if let Ok(mut show) = markers.get_mut(entity) {
                show.set_if_neq(if point.is_some() {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                });
            }
            if let Some(point) = point
                && let Ok(mut transform) = transforms.get_mut(entity)
            {
                transform.translation = point + Vec3::Y * 0.035;
            }
        }
    }
    for mut node in &mut overlay {
        selection::show_rectangle(&mut node, input.gesture());
    }
}

#[cfg(test)]
mod formation_tests {
    use super::*;

    #[test]
    fn formation_follows_the_commander_including_after_promotion() {
        let mut grid = crate::player::tests::grid();
        grid.config.edge = 1000.;
        grid.heights.fill(0.);
        let mut first = crate::player::tests::player();
        first.position = Vec3::new(8., 0., -10.);
        let mut second = crate::player::tests::player();
        second.position = Vec3::new(5., 0., -10.);
        let mut squad = Squad::new([first, second]);
        squad.select(1);
        squad.order(&grid, Vec3::new(14., 0., -10.), false, false);
        for _ in 0..800 {
            squad.step(&grid, None, None, 0.025);
        }
        assert!((squad.members[0].actor.position.x - 14.).abs() < 0.03);
        assert!((11.3..12.5).contains(&squad.members[1].actor.position.x));

        squad.select(2);
        squad.promote();
        squad.order(&grid, Vec3::new(11., 0., -17.), false, false);
        for _ in 0..800 {
            squad.step(&grid, None, None, 0.025);
        }
        let commander = squad.members[1].actor.position;
        let follower = squad.members[0].actor.position;
        assert!(commander.distance(Vec3::new(11., 0., -17.)) < 0.03);
        assert!(follower.z < -14.);
        assert!((1.1..2.6).contains(&follower.distance(commander)));
    }
}
