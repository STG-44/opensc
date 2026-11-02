//! Sinister Model File 1.0/1.1. Читатель не зависит от Bevy и не запускает EXE.
use crate::{Error, Result, normalize_path};
use std::collections::{BTreeMap, VecDeque};

const MAGIC: [u8; 8] = [0x1a, 0xfa, 0x31, 0xc1, 0xde, 0xed, 0x42, 0x14];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmfVersion {
    V1_0,
    V1_1,
}

impl SmfVersion {
    pub fn label(self) -> &'static str {
        match self {
            Self::V1_0 => "SMF V1.0",
            Self::V1_1 => "SMF V1.1",
        }
    }
}

#[derive(Debug)]
pub struct SmfModel {
    pub version: SmfVersion,
    pub name: String,
    pub radius: f32,
    /// Три исходных поля заголовка; их назначение ещё исследуется. Это не render scale.
    pub header_vector: [f32; 3],
    /// Сохраняем исходные флаги, не приписывая пока неизвестным битам смысл.
    pub flags: u32,
    pub nodes: Vec<SmfNode>,
    /// Индексы родителей и порядок, в котором родитель предшествует ребёнку.
    pub parents: Vec<Option<usize>>,
    pub node_order: Vec<usize>,
}

#[derive(Debug)]
pub struct SmfNode {
    pub name: String,
    pub parent: String,
    pub id: u32,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub flags: Option<u32>,
    pub meshes: Vec<SmfMesh>,
    pub bounds: Vec<SmfBounds>,
}

#[derive(Debug)]
pub struct SmfMesh {
    pub name: String,
    pub texture: String,
    pub vertices: Vec<SmfVertex>,
    pub faces: Vec<SmfFace>,
}

#[derive(Debug)]
pub struct SmfVertex {
    pub id: u32,
    pub position: [f32; 3],
    pub color: u32,
    pub specular: u32,
    pub uv: [f32; 2],
    /// В оригинале есть NaN и нулевые нормали. Рендер восстановит их по граням.
    pub normal: Option<[f32; 3]>,
}

#[derive(Debug)]
pub struct SmfFace {
    pub id: u32,
    pub indices: [u32; 3],
}

#[derive(Debug)]
pub struct SmfBounds {
    pub coordinates: [f32; 6],
    pub tag: u32,
}

impl SmfModel {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 512 * 1024 * 1024 {
            return Err(Error::Invalid("SMF больше 512 МиБ".into()));
        }
        let signature = bytes[..bytes.len().min(65536)]
            .windows(MAGIC.len())
            .position(|window| window == MAGIC)
            .ok_or_else(|| Error::Invalid("сигнатура SMF не найдена".into()))?;
        let mut reader = Reader {
            bytes,
            offset: signature + MAGIC.len(),
        };
        let version = match reader.text(16)?.as_str() {
            "SMF V1.0" => SmfVersion::V1_0,
            "SMF V1.1" => SmfVersion::V1_1,
            unknown => return Err(Error::Invalid(format!("неподдерживаемая версия {unknown}"))),
        };
        let name = reader.text(128)?;
        let radius = reader.finite::<1>()?[0];
        let header_vector = reader.finite()?;
        let flags = reader.u32()?;
        let node_size = if version == SmfVersion::V1_1 {
            300
        } else {
            296
        };
        let count = reader.count(16384, node_size)?;
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            let name = reader.text(128)?;
            let parent = reader.text(128)?;
            let id = reader.u32()?;
            let translation = reader.finite()?;
            let rotation: [f32; 4] = reader.finite()?;
            let length = rotation.iter().map(|v| v * v).sum::<f32>();
            if !length.is_finite() || length <= f32::EPSILON {
                return Err(Error::Invalid(format!("нулевой кватернион узла {name}")));
            }
            let mesh_count = reader.count(4096, 264)?;
            let bounds_count = reader.count(4096, 28)?;
            let flags = if version == SmfVersion::V1_1 {
                Some(reader.u32()?)
            } else {
                None
            };
            let mut meshes = Vec::with_capacity(mesh_count);
            for _ in 0..mesh_count {
                let name = reader.text(128)?;
                let texture = reader.text(128)?;
                if !texture.is_empty() {
                    normalize_path(&texture)?;
                }
                let vertex_count = reader.count(1_000_000, 44)?;
                let face_count = reader.count(1_000_000, 16)?;
                // Проверка общего размера до выделения памяти и чтения массивов.
                reader.ensure(vertex_count * 44 + face_count * 16)?;
                let mut vertices = Vec::with_capacity(vertex_count);
                for _ in 0..vertex_count {
                    let id = reader.u32()?;
                    let position = reader.finite()?;
                    let color = reader.u32()?;
                    let specular = reader.u32()?;
                    let uv = reader.finite()?;
                    let normal: [f32; 3] = reader.floats()?;
                    let length = normal.iter().map(|v| v * v).sum::<f32>();
                    let normal = (length.is_finite() && length > f32::EPSILON).then_some(normal);
                    vertices.push(SmfVertex {
                        id,
                        position,
                        color,
                        specular,
                        uv,
                        normal,
                    });
                }
                let mut faces = Vec::with_capacity(face_count);
                for _ in 0..face_count {
                    let id = reader.u32()?;
                    let indices = [reader.u32()?, reader.u32()?, reader.u32()?];
                    if indices.iter().any(|&index| index as usize >= vertex_count) {
                        return Err(Error::Invalid(format!(
                            "индекс грани за пределами mesh {name}"
                        )));
                    }
                    faces.push(SmfFace { id, indices });
                }
                meshes.push(SmfMesh {
                    name,
                    texture,
                    vertices,
                    faces,
                });
            }
            let mut bounds = Vec::with_capacity(bounds_count);
            for _ in 0..bounds_count {
                bounds.push(SmfBounds {
                    coordinates: reader.finite()?,
                    tag: reader.u32()?,
                });
            }
            nodes.push(SmfNode {
                name,
                parent,
                id,
                translation,
                rotation,
                flags,
                meshes,
                bounds,
            });
        }
        if reader.offset != bytes.len() {
            return Err(Error::Invalid(format!(
                "непрочитанный хвост SMF: {} байт",
                bytes.len() - reader.offset
            )));
        }
        let (parents, node_order) = hierarchy(&nodes)?;
        Ok(Self {
            version,
            name,
            radius,
            header_vector,
            flags,
            nodes,
            parents,
            node_order,
        })
    }
}

fn hierarchy(nodes: &[SmfNode]) -> Result<(Vec<Option<usize>>, Vec<usize>)> {
    let mut names = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if node.name.is_empty() || names.insert(node.name.as_str(), index).is_some() {
            return Err(Error::Invalid(
                "пустое или повторяющееся имя узла SMF".into(),
            ));
        }
    }
    let mut parents = Vec::with_capacity(nodes.len());
    let mut children = vec![Vec::new(); nodes.len()];
    let mut queue = VecDeque::new();
    for (index, node) in nodes.iter().enumerate() {
        let parent = if node.parent == "<root>" {
            queue.push_back(index);
            None
        } else {
            let parent = *names
                .get(node.parent.as_str())
                .ok_or_else(|| Error::Invalid(format!("нет родителя {}", node.parent)))?;
            children[parent].push(index);
            Some(parent)
        };
        parents.push(parent);
    }
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(index) = queue.pop_front() {
        order.push(index);
        queue.extend(children[index].iter().copied());
    }
    if order.len() != nodes.len() {
        return Err(Error::Invalid("цикл в иерархии SMF".into()));
    }
    Ok((parents, order))
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Reader<'_> {
    fn ensure(&self, length: usize) -> Result<()> {
        if length > self.bytes.len().saturating_sub(self.offset) {
            return Err(Error::Invalid(format!(
                "SMF обрезан на смещении {}",
                self.offset
            )));
        }
        Ok(())
    }
    fn take(&mut self, length: usize) -> Result<&[u8]> {
        self.ensure(length)?;
        let start = self.offset;
        self.offset += length;
        Ok(&self.bytes[start..self.offset])
    }
    fn u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("проверено take");
        Ok(u32::from_le_bytes(bytes))
    }
    fn floats<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut values = [0.0; N];
        for value in &mut values {
            *value = f32::from_bits(self.u32()?);
        }
        Ok(values)
    }
    fn finite<const N: usize>(&mut self) -> Result<[f32; N]> {
        let values = self.floats()?;
        if values.iter().any(|v| !f32::is_finite(*v)) {
            return Err(Error::Invalid("NaN/Infinity в координатах SMF".into()));
        }
        Ok(values)
    }
    fn text(&mut self, length: usize) -> Result<String> {
        let bytes = self.take(length)?;
        let end = bytes
            .iter()
            .position(|&v| v == 0)
            .ok_or_else(|| Error::Invalid("строка SMF без NUL".into()))?;
        Ok(encoding_rs::WINDOWS_1252
            .decode(&bytes[..end])
            .0
            .into_owned())
    }
    fn count(&mut self, limit: usize, minimum_size: usize) -> Result<usize> {
        let count = self.u32()? as usize;
        if count > limit {
            return Err(Error::Invalid(format!(
                "число элементов SMF {count} превышает {limit}"
            )));
        }
        self.ensure(count * minimum_size)?;
        Ok(count)
    }
}
