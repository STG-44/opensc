//! Separate glass from painted UV regions without moving the installed body.
//! Only the render mesh is subdivided. Collision keeps the installed geometry.
use super::PreparedPart;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, VertexAttributeValues},
    prelude::*,
    render::render_resource::PrimitiveTopology,
};
use std::error::Error;

#[derive(Clone, Copy)]
struct Vertex {
    position: Vec3,
    normal: Vec3,
    uv: Vec2,
}
impl Vertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            position: self.position.lerp(other.position, t),
            normal: self.normal.lerp(other.normal, t),
            uv: self.uv.lerp(other.uv, t),
        }
    }
}
fn clip(poly: &[Vertex], axis: usize, bound: f32, less: bool) -> Vec<Vertex> {
    let mut output = Vec::new();
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let inside = |v: Vertex| {
            if less {
                v.uv[axis] <= bound
            } else {
                v.uv[axis] >= bound
            }
        };
        if inside(a) {
            output.push(a);
        }
        if inside(a) != inside(b) {
            output.push(a.lerp(
                b,
                ((bound - a.uv[axis]) / (b.uv[axis] - a.uv[axis])).clamp(0., 1.),
            ));
        }
    }
    output
}
fn triangulate(poly: &[Vertex], out: &mut Vec<Vertex>) {
    for i in 1..poly.len().saturating_sub(1) {
        let a = poly[0];
        let b = poly[i];
        let c = poly[i + 1];
        if (b.position - a.position)
            .cross(c.position - a.position)
            .length_squared()
            > 1e-12
        {
            out.extend([a, b, c]);
        }
    }
}
fn split_rect(mut poly: Vec<Vertex>, rect: [f32; 4]) -> (Vec<Vec<Vertex>>, Vec<Vertex>) {
    let [left, top, right, bottom] = rect;
    let mut outside = Vec::new();
    for (axis, bound, less) in [
        (0, left, false),
        (0, right, true),
        (1, top, false),
        (1, bottom, true),
    ] {
        if poly.iter().any(|v| {
            if less {
                v.uv[axis] > bound
            } else {
                v.uv[axis] < bound
            }
        }) {
            let p = clip(&poly, axis, bound, !less);
            if p.len() >= 3 {
                outside.push(p);
            }
        }
        poly = clip(&poly, axis, bound, less);
        if poly.len() < 3 {
            break;
        }
    }
    (outside, poly)
}

/// The owned atlas keeps the legacy side layout. Only its stencil on the
/// positive-X body side needs reversing; door and camouflage UVs stay intact.
pub(super) fn readable_hummer_stencil(part: &mut PreparedPart) -> Result<(), Box<dyn Error>> {
    if part.name != "hummer-back01"
        || part.texture.as_deref() != Some("textures/opensc-environment/hummer-body-v1.png")
    {
        return Ok(());
    }
    flip_positive_side(part, [0.512, 0.684, 0.67, 0.747])
}
fn flip_positive_side(part: &mut PreparedPart, rect: [f32; 4]) -> Result<(), Box<dyn Error>> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err("Missing stencil positions".into());
    };
    let Some(VertexAttributeValues::Float32x3(normals)) =
        part.mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        return Err("Missing stencil normals".into());
    };
    let Some(VertexAttributeValues::Float32x2(uv)) = part.mesh.attribute(Mesh::ATTRIBUTE_UV_0)
    else {
        return Err("Missing stencil UVs".into());
    };
    let indices = part
        .mesh
        .indices()
        .ok_or("Missing stencil indices")?
        .iter()
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    for tri in indices.as_chunks::<3>().0 {
        let poly = tri
            .iter()
            .map(|&i| Vertex {
                position: Vec3::from_array(positions[i]),
                normal: Vec3::from_array(normals[i]),
                uv: Vec2::from_array(uv[i]),
            })
            .collect::<Vec<_>>();
        if poly.iter().all(|v| v.position.x > 0.) {
            let (outside, mut text) = split_rect(poly, rect);
            for p in outside {
                triangulate(&p, &mut output);
            }
            for v in &mut text {
                v.uv.x = rect[0] + rect[2] - v.uv.x;
            }
            triangulate(&text, &mut output);
        } else {
            triangulate(&poly, &mut output);
        }
    }
    part.mesh = mesh(&output);
    Ok(())
}
fn mesh(vertices: &[Vertex]) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices
            .iter()
            .map(|v| v.position.to_array())
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vertices
            .iter()
            .map(|v| v.normal.to_array())
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vertices.iter().map(|v| v.uv.to_array()).collect::<Vec<_>>(),
    )
    .with_inserted_indices(Indices::U32((0..vertices.len() as u32).collect()))
}
pub(super) fn split_glass(
    part: PreparedPart,
    regions: &[[f32; 4]],
) -> Result<Vec<PreparedPart>, Box<dyn Error>> {
    if regions.is_empty() {
        return Ok(vec![part]);
    }
    let Some(VertexAttributeValues::Float32x3(positions)) =
        part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Err("Missing retained positions".into());
    };
    let Some(VertexAttributeValues::Float32x3(normals)) =
        part.mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        return Err("Missing retained normals".into());
    };
    let Some(VertexAttributeValues::Float32x2(uv)) = part.mesh.attribute(Mesh::ATTRIBUTE_UV_0)
    else {
        return Err("Missing retained UVs".into());
    };
    let indices = part
        .mesh
        .indices()
        .ok_or("Missing retained indices")?
        .iter()
        .collect::<Vec<_>>();
    let mut paint = Vec::new();
    let mut glass = Vec::new();
    for triangle in indices.as_chunks::<3>().0 {
        let mut polys = vec![
            triangle
                .iter()
                .map(|&i| Vertex {
                    position: Vec3::from_array(positions[i]),
                    normal: Vec3::from_array(normals[i]),
                    uv: Vec2::from_array(uv[i]),
                })
                .collect::<Vec<_>>(),
        ];
        for &[left, top, right, bottom] in regions {
            let mut next = Vec::new();
            for poly in polys {
                let (outside, poly) = split_rect(poly, [left, top, right, bottom]);
                next.extend(outside);
                triangulate(&poly, &mut glass);
            }
            polys = next;
        }
        for poly in polys {
            triangulate(&poly, &mut paint);
        }
    }
    let mut result = Vec::new();
    if !paint.is_empty() {
        let mut p = part.clone();
        p.mesh = mesh(&paint);
        result.push(p);
    }
    if !glass.is_empty() {
        let mut p = part;
        p.name.push_str(" · glass");
        p.mesh = mesh(&glass);
        p.glass = true;
        result.push(p);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn panel() -> PreparedPart {
        let vertices =
            [[0., 0.], [1., 0.], [1., 1.], [0., 0.], [1., 1.], [0., 1.]].map(|p| Vertex {
                position: Vec3::new(p[0] * 2., p[1] * 3., 5.),
                normal: Vec3::Z,
                uv: Vec2::from_array(p),
            });
        PreparedPart {
            name: "body".into(),
            texture: Some("textures/opensc-environment/hummer-body-v1.png".into()),
            alpha_mode: AlphaMode::Opaque,
            glass: false,
            wiper: None,
            lamp: None,
            mesh: mesh(&vertices),
        }
    }
    fn area(part: &PreparedPart) -> f32 {
        let Some(VertexAttributeValues::Float32x3(p)) =
            part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        part.mesh
            .indices()
            .unwrap()
            .iter()
            .collect::<Vec<_>>()
            .as_chunks::<3>()
            .0
            .iter()
            .map(|i| {
                let [a, b, c] = [i[0], i[1], i[2]].map(|i| Vec3::from_array(p[i]));
                let cross = (b - a).cross(c - a);
                assert!(cross.z > 0.);
                cross.length() * 0.5
            })
            .sum()
    }
    #[test]
    fn windows_preserve_body_area_uvs_winding_and_material_separation() {
        let parts = split_glass(panel(), &[[0.2, 0.3, 0.4, 0.8], [0.6, 0.3, 0.8, 0.8]]).unwrap();
        assert_eq!(parts.len(), 2);
        assert!((parts.iter().map(area).sum::<f32>() - 6.).abs() < 1e-5);
        let glass = parts.iter().find(|p| p.glass).unwrap();
        assert!((area(glass) - 1.2).abs() < 1e-5);
        assert_eq!(glass.surface().perceptual_roughness, 0.16);
        assert_eq!(glass.surface().reflectance, 0.12);
        assert!(
            parts
                .iter()
                .find(|p| !p.glass)
                .unwrap()
                .surface()
                .perceptual_roughness
                > 0.7
        );
        for part in &parts {
            let Some(VertexAttributeValues::Float32x3(p)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!()
            };
            let Some(VertexAttributeValues::Float32x2(uv)) =
                part.mesh.attribute(Mesh::ATTRIBUTE_UV_0)
            else {
                panic!()
            };
            for (p, uv) in p.iter().zip(uv) {
                assert!((p[0] - uv[0] * 2.).abs() < 1e-5);
                assert!((p[1] - uv[1] * 3.).abs() < 1e-5);
                assert_eq!(p[2], 5.);
            }
        }
    }
    #[test]
    fn overlapping_regions_are_a_union_and_no_regions_leave_mesh_unchanged() {
        let p = panel();
        let count = p.mesh.count_vertices();
        assert_eq!(split_glass(p, &[]).unwrap()[0].mesh.count_vertices(), count);
        let parts = split_glass(panel(), &[[0.2, 0.3, 0.6, 0.8], [0.4, 0.3, 0.8, 0.8]]).unwrap();
        assert!((parts.iter().map(area).sum::<f32>() - 6.).abs() < 1e-5);
        assert!((area(parts.iter().find(|p| p.glass).unwrap()) - 1.8).abs() < 1e-5);
    }
    #[test]
    fn constant_uv_on_glass_boundary_does_not_duplicate_geometry() {
        let mut p = panel();
        if let Some(VertexAttributeValues::Float32x2(uv)) =
            p.mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
        {
            for p in uv {
                p[0] = 0.2;
            }
        }
        let parts = split_glass(p, &[[0.2, 0.3, 0.8, 0.8]]).unwrap();
        assert!((parts.iter().map(area).sum::<f32>() - 6.).abs() < 1e-5);
        assert!((area(parts.iter().find(|p| p.glass).unwrap()) - 3.).abs() < 1e-5);
    }
    #[test]
    fn stencil_reverses_only_one_side_and_preserves_surface_area() {
        for side in [-5., 5.] {
            let mut p = panel();
            if let Some(VertexAttributeValues::Float32x3(v)) =
                p.mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
            {
                for v in v {
                    v[0] += side;
                }
            }
            flip_positive_side(&mut p, [0.2, 0.3, 0.8, 0.8]).unwrap();
            assert!((area(&p) - 6.).abs() < 1e-5);
            let Some(VertexAttributeValues::Float32x3(v)) =
                p.mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!()
            };
            let Some(VertexAttributeValues::Float32x2(uv)) = p.mesh.attribute(Mesh::ATTRIBUTE_UV_0)
            else {
                panic!()
            };
            let reversed = v
                .iter()
                .zip(uv)
                .filter(|(p, uv)| ((p[0] - side) / 2. - uv[0]).abs() > 1e-4)
                .count();
            if side > 0. {
                assert!(reversed > 0);
            } else {
                assert_eq!(reversed, 0);
            }
        }
    }
}
