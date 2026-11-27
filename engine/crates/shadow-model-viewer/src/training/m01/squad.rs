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
}

pub(super) struct MemberRig {
    root: Entity,
    bones: Vec<Entity>,
    selection_ring: Entity,
    target_ring: Entity,
}

#[derive(Resource)]
pub(crate) struct Squad {
    pub(super) members: Vec<Member>,
    pub(super) leader: usize,
    pub(super) following: Option<usize>,
    pub(crate) paused: bool,
}

impl Squad {
    pub(super) fn new(actors: impl IntoIterator<Item = Player>) -> Self {
        Self {
            members: actors
                .into_iter()
                .map(|actor| Member {
                    actor,
                    queued: VecDeque::new(),
                    render: None,
                })
                .collect(),
            leader: 0,
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
        if !self.members[self.leader].actor.selected
            && let Some(next) = self.members.iter().position(|member| member.actor.selected)
        {
            self.leader = next;
        }
        if self
            .following
            .is_some_and(|index| !self.members[index].actor.selected)
        {
            self.following = None;
        }
    }

    pub(super) fn reset(&mut self, orbit: &mut Orbit) {
        for member in &mut self.members {
            member.actor.reset_spawn();
            member.queued.clear();
        }
        self.following = None;
        self.leader = 0;
        self.select(u8::MAX);
        *orbit = player_view(self.members[self.leader].actor.camera_target());
    }

    pub(super) fn order(
        &mut self,
        grid: &TerrainGrid,
        site: Option<&Blockout>,
        anchor: Vec3,
        run: bool,
        append: bool,
    ) {
        let heading = (anchor - self.members[self.leader].actor.position).with_y(0.);
        let side = heading.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X) * 1.35;
        let mut first = true;
        for index in std::iter::once(self.leader)
            .chain((0..self.members.len()).filter(|i| *i != self.leader))
        {
            let member = &mut self.members[index];
            if !member.actor.selected {
                continue;
            }
            let clear = |point| {
                player::surface(grid, point).is_some_and(|height| {
                    site.is_none_or(|site| {
                        site.free(point.with_y(height), member.actor.body_height())
                    })
                })
            };
            let point = if first || !clear(anchor) {
                anchor
            } else {
                [side, -side, side.cross(Vec3::Y), -side.cross(Vec3::Y)]
                    .into_iter()
                    .map(|offset| anchor + offset)
                    .find(|&p| clear(p))
                    .unwrap_or(anchor)
            };
            first = false;
            if append && member.actor.destination.is_some() {
                if member.queued.len() < 16 {
                    member.queued.push_back(point);
                }
            } else if member.actor.order(grid, point, run) {
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
        let empty = ButtonInput::default();
        for (index, member) in self.members.iter_mut().enumerate() {
            let actor = &mut member.actor;
            if Some(index) != direct {
                if actor.route.failed {
                    member.queued.clear();
                } else if actor.destination.is_none()
                    && let Some(next) = member.queued.pop_front()
                {
                    actor.order(grid, next, false);
                }
                if let Some(site) = site {
                    prepare_order(actor, grid, site);
                }
            }
            let previous = actor.position;
            actor.step(&empty, dt, grid);
            if let Some(site) = site {
                site.guard(actor, previous, grid);
            }
        }
    }

    pub(crate) fn actors(&self) -> impl Iterator<Item = &Player> {
        self.members.iter().map(|member| &member.actor)
    }
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
    for (index, member) in squad.members.iter_mut().enumerate() {
        let mesh = |handle| Mesh3d(handle);
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
                    mesh(handle),
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
                rig.target_ring,
                actor
                    .selected
                    .then_some(actor.destination)
                    .flatten()
                    .filter(|_| Some(index) != input.direct_actor()),
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
