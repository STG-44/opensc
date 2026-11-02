//! Начальная поза по созданию узла 0x474BB0 и выбору матрицы 0x491050 оригинала.
use bevy::prelude::*;
use shadow_formats::SmfNode;
pub fn rest(node: &SmfNode, root: bool) -> Transform {
    let mut flags = 4u32; // Конструктор выбирает Euler; его исходные углы равны нулю.
    if node.meshes.is_empty() && !node.bounds.is_empty() {
        flags &= !4;
    }
    flags |= node.flags.unwrap_or(0);
    let rotation = if root || flags & 4 != 0 {
        Quat::IDENTITY
    } else {
        Quat::from_array(node.rotation).normalize().conjugate()
    };
    Transform::from_translation(Vec3::from_array(node.translation)).with_rotation(rotation)
}
#[cfg(test)]
mod tests {
    use super::*;
    use shadow_formats::SmfBounds;
    #[test]
    fn original_euler_flag_overrides_stored_quaternion_and_root_orientation() {
        let mut node = SmfNode {
            name: "node".into(),
            parent: "parent".into(),
            id: 1,
            translation: [10.0, 20.0, 30.0],
            rotation: Quat::from_rotation_x(1.0).to_array(),
            flags: Some(4),
            meshes: Vec::new(),
            bounds: vec![SmfBounds {
                coordinates: [0.0; 6],
                tag: 0,
            }],
        };
        assert_eq!(rest(&node, false).rotation, Quat::IDENTITY);
        assert_eq!(rest(&node, false).translation, Vec3::new(10.0, 20.0, 30.0));
        node.flags = None;
        assert!(
            rest(&node, false)
                .rotation
                .angle_between(Quat::from_rotation_x(-1.0))
                < 1e-4
        );
        assert_eq!(rest(&node, true).rotation, Quat::IDENTITY);
        node.bounds.clear();
        assert_eq!(rest(&node, false).rotation, Quat::IDENTITY);
    }
}
