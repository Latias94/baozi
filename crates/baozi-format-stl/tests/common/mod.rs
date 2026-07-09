#![allow(dead_code)]

use baozi_core::{BaoziError, Diagnostic, Result, Scene};
use baozi_format_stl::StlImporter;
use baozi_import::{FormatImporter, ImportContext, ImportOptions};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub struct BinaryFacet {
    pub normal: [f32; 3],
    pub vertices: [[f32; 3]; 3],
    pub attribute: u16,
}

impl BinaryFacet {
    pub fn unit_triangle() -> Self {
        Self {
            normal: [0.0, 0.0, 1.0],
            vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            attribute: 0,
        }
    }
}

pub fn binary_stl(header_prefix: &[u8], facets: &[BinaryFacet]) -> Vec<u8> {
    let mut bytes = vec![0; 80];
    for (target, source) in bytes.iter_mut().zip(header_prefix.iter().copied()) {
        *target = source;
    }
    bytes.extend_from_slice(&(facets.len() as u32).to_le_bytes());
    for facet in facets {
        push_vec3(&mut bytes, facet.normal);
        for vertex in facet.vertices {
            push_vec3(&mut bytes, vertex);
        }
        bytes.extend_from_slice(&facet.attribute.to_le_bytes());
    }
    bytes
}

pub fn ascii_triangle(name: &str) -> String {
    format!(
        "solid {name}\n  facet normal 0 0 1\n    outer loop\n      vertex 0 0 0\n      vertex 1 0 0\n      vertex 0 1 0\n    endloop\n  endfacet\nendsolid {name}\n"
    )
}

pub fn import_bytes(source: &str, bytes: &[u8]) -> Result<(Scene, Vec<Diagnostic>)> {
    let (scene, diagnostics) = import_bytes_result(source, bytes, ImportOptions::memory())?;
    Ok((scene?, diagnostics))
}

pub fn import_bytes_result(
    source: &str,
    bytes: &[u8],
    options: ImportOptions,
) -> Result<(Result<Scene>, Vec<Diagnostic>)> {
    let path = AssetPath::new(source)?;
    let mut io = MemoryAssetIo::new();
    io.insert(path.clone(), Arc::<[u8]>::from(bytes));
    let mut ctx = ImportContext::with_options(&io, path, options);
    let result = StlImporter.read(&mut ctx);
    let diagnostics = ctx.into_diagnostics();
    Ok((result, diagnostics))
}

pub fn expected_error(result: Result<Scene>) -> Result<BaoziError> {
    match result {
        Ok(_) => Err(BaoziError::parse(
            "test",
            None,
            "expected STL import to fail",
        )),
        Err(error) => Ok(error),
    }
}

fn push_vec3(bytes: &mut Vec<u8>, values: [f32; 3]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}
