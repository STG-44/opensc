//! North-up by default; the unlocked map keeps the camera facing up.
//! World, texture, input and marker coordinates share the same invertible view.
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use shadow_formats::TerrainGrid;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    pub center: Vec2,
    pub zoom: f32,
    pub angle: f32,
}
impl View {
    pub fn new(center: Vec3, extent: Vec2, zoom: f32, locked: bool, yaw: f32) -> Self {
        let half = 0.5 / zoom;
        let center = Vec2::new(center.x, -center.z) / extent;
        Self {
            center: if locked {
                center.clamp(Vec2::splat(half), Vec2::splat(1.0 - half))
            } else {
                center
            },
            zoom,
            angle: if locked {
                0.0
            } else {
                std::f32::consts::PI - yaw
            },
        }
    }
    pub fn project(self, uv: Vec2) -> Vec2 {
        Mat2::from_angle(self.angle) * (uv - self.center) * self.zoom + Vec2::splat(0.5)
    }
    pub fn unproject(self, point: Vec2) -> Vec2 {
        self.center + Mat2::from_angle(-self.angle) * (point - Vec2::splat(0.5)) / self.zoom
    }
    pub fn world(self, point: Vec2, grid: &TerrainGrid) -> Vec3 {
        let uv = self.unproject(point.clamp(Vec2::ZERO, Vec2::ONE));
        let p = (uv * extent(grid)).clamp(Vec2::ZERO, playable_extent(grid));
        let p = Vec3::new(p.x, 0.0, -p.y);
        p.with_y(crate::player::surface(grid, p).unwrap_or(0.0))
    }
}
pub(crate) fn extent(grid: &TerrainGrid) -> Vec2 {
    Vec2::new(grid.config.width as f32, grid.config.height as f32)
        * grid.config.edge
        * crate::map_scene::WORLD_SCALE
}
fn playable_extent(grid: &TerrainGrid) -> Vec2 {
    Vec2::new(
        (grid.config.width - 1) as f32,
        (grid.config.height - 1) as f32,
    ) * grid.config.edge
        * crate::map_scene::WORLD_SCALE
}
pub(crate) fn uv(position: Vec3, extent: Vec2) -> Vec2 {
    Vec2::new(position.x, -position.z) / extent
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NoiseRing {
    pub center: Vec2,
    pub radius: Vec2,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Contact {
    pub position: Vec2,
    pub color: [u8; 4],
    pub selected: bool,
    pub friendly: bool,
    pub noise: Option<NoiseRing>,
}
pub(crate) const FRIEND: [u8; 4] = [56, 255, 48, 255];
pub(crate) const ENEMY: [u8; 4] = [255, 48, 40, 255];
pub(crate) const NEUTRAL: [u8; 4] = [56, 156, 255, 255];
const WHITE: [u8; 4] = [255; 4];
const BLACK: [u8; 4] = [8, 12, 8, 255];

#[derive(Clone, PartialEq)]
struct Frame {
    size: UVec2,
    view: View,
    contacts: Vec<Contact>,
    camera: Vec<Vec2>,
    center: Vec2,
    fog_generation: u64,
}
#[derive(Default)]
pub(crate) struct Radar {
    pub handle: Option<Handle<Image>>,
    pub locked: bool,
    pub zoom: f32,
    pub drag: Option<View>,
    source: Vec<u8>,
    source_size: UVec2,
    previous: Option<Frame>,
}
impl Radar {
    pub fn new() -> Self {
        Self {
            locked: true,
            zoom: 1.0,
            ..default()
        }
    }
    pub fn setup(&mut self, source: &Image, images: &mut Assets<Image>) {
        self.source = source.data.clone().expect("decoded radar pixels");
        self.source_size = source.size();
        self.handle = Some(images.add(image(UVec2::ONE, vec![0; 4])));
    }
    pub fn view(&self, orbit: &crate::training::Orbit, grid: &TerrainGrid) -> View {
        self.drag.unwrap_or_else(|| {
            View::new(
                crate::training::camera_focus(orbit),
                extent(grid),
                self.zoom,
                self.locked,
                orbit.yaw,
            )
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        size: UVec2,
        view: View,
        contacts: Vec<Contact>,
        camera: Vec<Vec2>,
        center: Vec2,
        fog: Option<&crate::perception::VisibilityState>,
        images: &mut Assets<Image>,
    ) {
        // Only changed pixels reach the GPU; idle HUDs do not upload every frame.
        let size = size.clamp(UVec2::splat(32), UVec2::splat(1024));
        let frame = Frame {
            size,
            view,
            contacts,
            camera,
            center,
            fog_generation: fog.map_or(0, |f| f.generation),
        };
        if self.previous.as_ref() == Some(&frame) {
            return;
        }
        let mut canvas = Canvas {
            size,
            pixels: vec![0; (size.x * size.y * 4) as usize],
        };
        for y in 0..size.y {
            for x in 0..size.x {
                let point = (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / size.as_vec2();
                let uv = view.unproject(point);
                let mut color = self.sample(uv);
                if let Some(fog) = fog {
                    let shade = fog.shade(uv);
                    for channel in &mut color[..3] {
                        *channel = (f32::from(*channel) * shade).round() as u8;
                    }
                }
                canvas.pixel(IVec2::new(x as i32, y as i32), color);
            }
        }
        for contact in &frame.contacts {
            if let Some(noise) = contact
                .noise
                .as_ref()
                .filter(|_| contact.selected && contact.friendly)
            {
                canvas.noise_ring(view, noise);
            }
        }
        let to_pixel = |uv| view.project(uv) * (size.as_vec2() - Vec2::ONE);
        let camera: Vec<_> = frame.camera.iter().map(|&p| to_pixel(p)).collect();
        // Dark backing keeps the white camera footprint visible over pale terrain.
        for width in [2, 0] {
            for i in 0..camera.len() {
                canvas.line(
                    camera[i],
                    camera[(i + 1) % camera.len()],
                    if width == 2 { BLACK } else { WHITE },
                    width,
                );
            }
        }
        let center = to_pixel(center);
        canvas.line(center - Vec2::X * 3.0, center + Vec2::X * 3.0, BLACK, 1);
        canvas.line(center - Vec2::Y * 3.0, center + Vec2::Y * 3.0, BLACK, 1);
        canvas.line(center - Vec2::X * 2.0, center + Vec2::X * 2.0, WHITE, 0);
        canvas.line(center - Vec2::Y * 2.0, center + Vec2::Y * 2.0, WHITE, 0);
        for contact in &frame.contacts {
            let point = view.project(contact.position);
            if !(0.0..=1.0).contains(&point.x) || !(0.0..=1.0).contains(&point.y) {
                continue;
            }
            let point = to_pixel(contact.position);
            let scale = (size.min_element() as f32 / 109.0).max(1.0);
            let radius = ((if contact.friendly { 3.0 } else { 2.0 }) * scale).round() as i32;
            canvas.disc(point, radius + 1, BLACK);
            if contact.selected {
                canvas.disc(point, radius, WHITE);
            }
            canvas.disc(
                point,
                if contact.selected { radius - 1 } else { radius },
                contact.color,
            );
        }
        *images
            .get_mut(self.handle.as_ref().expect("radar setup"))
            .expect("radar image") = image(size, canvas.pixels);
        self.previous = Some(frame);
    }
    fn sample(&self, uv: Vec2) -> [u8; 4] {
        if uv.min_element() < 0.0 || uv.max_element() > 1.0 {
            return [13, 20, 13, 255];
        }
        let position = uv * (self.source_size.as_vec2() - Vec2::ONE);
        let low = position.floor().as_uvec2();
        let high = (low + UVec2::ONE).min(self.source_size - UVec2::ONE);
        let fract = position - low.as_vec2();
        let get = |x, y, c| self.source[((y * self.source_size.x + x) * 4 + c) as usize] as f32;
        let mut out = [255; 4];
        for c in 0..3 {
            let top = get(low.x, low.y, c).lerp(get(high.x, low.y, c), fract.x);
            let bottom = get(low.x, high.y, c).lerp(get(high.x, high.y, c), fract.x);
            out[c as usize] = top.lerp(bottom, fract.y).round() as u8;
        }
        out
    }
}
fn image(size: UVec2, pixels: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}
struct Canvas {
    size: UVec2,
    pixels: Vec<u8>,
}
impl Canvas {
    fn noise_ring(&mut self, view: View, ring: &NoiseRing) {
        if !ring.center.is_finite() || !ring.radius.is_finite() || ring.radius.min_element() <= 0.0
        {
            return;
        }
        let size = self.size.as_vec2() - Vec2::ONE;
        let point = |i: usize| {
            let angle = i as f32 * std::f32::consts::TAU / 96.0;
            view.project(ring.center + Vec2::new(angle.cos(), angle.sin()) * ring.radius) * size
        };
        for i in 0..96 {
            let a = point(i);
            let b = point(i + 1);
            let max = self.size.as_vec2() - Vec2::ONE;
            if (a.x < 0.0 && b.x < 0.0)
                || (a.y < 0.0 && b.y < 0.0)
                || (a.x > max.x && b.x > max.x)
                || (a.y > max.y && b.y > max.y)
            {
                continue;
            }
            self.line(a, b, [44, 168, 72, 255], 0);
        }
    }
    fn pixel(&mut self, p: IVec2, color: [u8; 4]) {
        if p.x >= 0 && p.y >= 0 && p.x < self.size.x as i32 && p.y < self.size.y as i32 {
            let i = ((p.y as u32 * self.size.x + p.x as u32) * 4) as usize;
            self.pixels[i..i + 4].copy_from_slice(&color);
        }
    }
    fn disc(&mut self, p: Vec2, radius: i32, color: [u8; 4]) {
        let p = p.round().as_ivec2();
        for y in -radius..=radius {
            for x in -radius..=radius {
                if x * x + y * y <= radius * radius {
                    self.pixel(p + IVec2::new(x, y), color);
                }
            }
        }
    }
    fn line(&mut self, a: Vec2, b: Vec2, color: [u8; 4], width: i32) {
        // Clip before rasterizing: a zoomed footprint can extend far beyond the map.
        let mut lo = 0.0f32;
        let mut hi = 1.0f32;
        let d = b - a;
        for (p, q) in [
            (-d.x, a.x),
            (d.x, self.size.x as f32 - 1.0 - a.x),
            (-d.y, a.y),
            (d.y, self.size.y as f32 - 1.0 - a.y),
        ] {
            if p.abs() < 1e-6 {
                if q < 0.0 {
                    return;
                }
            } else if p < 0.0 {
                lo = lo.max(q / p);
            } else {
                hi = hi.min(q / p);
            }
        }
        if lo > hi {
            return;
        }
        let start = a + d * lo;
        let end = a + d * hi;
        let steps = (end - start).abs().max_element().ceil() as u32;
        for i in 0..=steps {
            self.disc(
                start.lerp(end, i as f32 / steps.max(1) as f32),
                width,
                color,
            );
        }
    }
}

/// Intersect the camera frustum with a horizontal plane at its current focus.
/// Clip the map polygon instead of extending sky-facing rays to infinity.
/// Terrain relief and occlusion are intentionally not a visibility/fog model.
pub(crate) fn footprint(
    orbit: &crate::training::Orbit,
    fov: f32,
    aspect: f32,
    extent: Vec2,
) -> Vec<Vec2> {
    let pose = crate::training::camera_pose(orbit);
    let focus = crate::training::camera_focus(orbit);
    let to_local = |p: Vec3| pose.rotation.inverse() * (p - pose.translation);
    let mut polygon = vec![
        Vec3::new(0.0, focus.y, 0.0),
        Vec3::new(extent.x, focus.y, 0.0),
        Vec3::new(extent.x, focus.y, -extent.y),
        Vec3::new(0.0, focus.y, -extent.y),
    ];
    let t = (fov * 0.5).tan();
    for plane in [
        Vec4::new(1.0, 0.0, -t * aspect, 0.0),
        Vec4::new(-1.0, 0.0, -t * aspect, 0.0),
        Vec4::new(0.0, 1.0, -t, 0.0),
        Vec4::new(0.0, -1.0, -t, 0.0),
        Vec4::new(0.0, 0.0, -1.0, -0.01),
    ] {
        let value = |p| plane.dot(to_local(p).extend(1.0));
        let old = std::mem::take(&mut polygon);
        if old.is_empty() {
            break;
        }
        for i in 0..old.len() {
            let a = old[i];
            let b = old[(i + 1) % old.len()];
            let va = value(a);
            let vb = value(b);
            if va >= 0.0 {
                polygon.push(a);
            }
            if (va >= 0.0) != (vb >= 0.0) {
                polygon.push(a.lerp(b, va / (va - vb)));
            }
        }
    }
    polygon.into_iter().map(|p| uv(p, extent)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn noise_outline_uses_the_same_rotation_zoom_and_clipping_as_contacts() {
        for locked in [true, false] {
            let view = View::new(
                Vec3::new(50.0, 0.0, -50.0),
                Vec2::splat(100.0),
                2.0,
                locked,
                0.7,
            );
            let mut canvas = Canvas {
                size: UVec2::splat(101),
                pixels: vec![0; 101 * 101 * 4],
            };
            let ring = NoiseRing {
                center: Vec2::splat(0.5),
                radius: Vec2::splat(0.1),
            };
            canvas.noise_ring(view, &ring);
            let expected = (view.project(ring.center + Vec2::X * 0.1) * 100.0)
                .round()
                .as_ivec2();
            let index = ((expected.y as u32 * 101 + expected.x as u32) * 4) as usize;
            assert_eq!(&canvas.pixels[index..index + 4], &[44, 168, 72, 255]);
            assert_eq!(&canvas.pixels[(50 * 101 + 50) * 4..][..4], &[0; 4]);
            let count = canvas
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] > 0)
                .count();
            assert!((80..200).contains(&count), "ring must be an outline");
            canvas.noise_ring(
                view,
                &NoiseRing {
                    center: Vec2::ZERO,
                    radius: Vec2::splat(100000.0),
                },
            );
            assert_eq!(canvas.pixels.len(), 101 * 101 * 4);
        }
    }
    #[test]
    fn locked_map_and_rotating_map_share_invertible_click_coordinates() {
        for locked in [true, false] {
            for yaw in [0.0, 0.7, 3.1, 5.5] {
                for zoom in [1.0, 2.0, 8.0] {
                    let view = View::new(
                        Vec3::new(10.0, 7.0, -70.0),
                        Vec2::new(100.0, 200.0),
                        zoom,
                        locked,
                        yaw,
                    );
                    for uv in [Vec2::ZERO, Vec2::ONE, Vec2::new(0.1, 0.7)] {
                        assert!(view.unproject(view.project(uv)).distance(uv) < 1e-5);
                    }
                    if !locked {
                        assert!(view.project(view.center).distance(Vec2::splat(0.5)) < 1e-5);
                        let forward = Vec2::new(-yaw.sin(), yaw.cos());
                        let screen = Mat2::from_angle(view.angle) * forward;
                        assert!(screen.distance(-Vec2::Y) < 1e-5);
                    }
                }
            }
        }
    }
    #[test]
    fn full_map_always_includes_map_edges_and_does_not_follow_camera() {
        for center in [Vec3::ZERO, Vec3::splat(10000.0)] {
            let view = View::new(center, Vec2::splat(100.0), 1.0, true, 1.0);
            assert_eq!(view.project(Vec2::ZERO), Vec2::ZERO);
            assert_eq!(view.project(Vec2::ONE), Vec2::ONE);
        }
    }
    #[test]
    fn click_at_texture_edge_stays_on_last_playable_terrain_sample() {
        let (_, _, _, grid) = crate::party::tests::fixture();
        let view = View::new(Vec3::ZERO, extent(&grid), 1.0, true, 0.0);
        let p = view.world(Vec2::ONE, &grid);
        assert_eq!(Vec2::new(p.x, -p.z), playable_extent(&grid));
        assert!(crate::player::surface(&grid, p).is_some());
    }
    #[test]
    fn camera_footprint_handles_ultrawide_rotation_and_horizon() {
        for aspect in [16.0 / 9.0, 21.0 / 9.0] {
            for pitch in [0.1, 0.6, 1.2] {
                for yaw in [0.0, 0.8, 3.0] {
                    let orbit = crate::training::Orbit {
                        target: Vec3::new(50.0, 7.0, -50.0),
                        yaw,
                        pitch,
                        distance: 10.0,
                    };
                    let extent = Vec2::splat(100.0);
                    let points = footprint(&orbit, 0.8, aspect, extent);
                    assert!(points.len() >= 3);
                    let pose = crate::training::camera_pose(&orbit);
                    for point in points {
                        assert!(
                            point.is_finite()
                                && point.min_element() >= -1e-5
                                && point.max_element() <= 1.00001
                        );
                        let local = pose.rotation.inverse()
                            * (Vec3::new(point.x * 100.0, 7.0, -point.y * 100.0)
                                - pose.translation);
                        let t = (0.4f32).tan();
                        assert!(local.x.abs() <= -local.z * t * aspect + 0.001);
                        assert!(local.y.abs() <= -local.z * t + 0.001);
                    }
                }
            }
        }
    }
}
