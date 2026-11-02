use bevy::prelude::*;
pub(crate) fn reconstructed_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for face in indices.as_chunks::<3>().0 {
        let [a, b, c] = [face[0] as usize, face[1] as usize, face[2] as usize];
        let p = |i| Vec3::from_array(positions[i]);
        let normal = (p(b) - p(a)).cross(p(c) - p(a));
        if normal.is_finite() {
            for i in [a, b, c] {
                normals[i] += normal;
            }
        }
    }
    normals
        .into_iter()
        .map(|normal| normal.try_normalize().unwrap_or(Vec3::Y).to_array())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::reconstructed_normals;
    #[test]
    fn rebuilds_normals_and_keeps_degenerate_faces_finite() {
        let vertices = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 2.0, 2.0],
        ];
        let normals = reconstructed_normals(&vertices, &[0, 1, 2, 3, 3, 3]);
        assert_eq!(&normals[..3], &[[0.0, 0.0, 1.0]; 3]);
        assert_eq!(normals[3], [0.0, 1.0, 0.0]);
    }
}
