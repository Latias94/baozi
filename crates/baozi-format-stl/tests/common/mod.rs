#![allow(dead_code)]

use baozi_core::FormatId;
use baozi_format_stl::{FORMAT_ID, StlImporter};
use baozi_import::{
    FormatFailureDetailValue, FormatFailureEnvelope, FormatSelection, ImportCause,
    ImportConfigOverlay, ImportFailure, ImportOperationControl, ImportReport, ImportRequest,
    ImportRequestLayer, ImportResult, OptionCompileLimits, PreparedImportRequest,
    RegistryBuildLimits, RegistryBuilder, ResourceLimits, ResourceLimitsOverlay, SceneSnapshot,
    SceneSnapshotError, SceneSnapshotLimits, import_asset,
};
use baozi_io::{AssetPath, MemoryAssetIo};
use std::error::Error;
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

pub fn import_bytes(
    source: &str,
    bytes: &[u8],
) -> Result<ImportReport, Box<dyn Error + Send + Sync>> {
    match import_bytes_with_limits(source, bytes, ResourceLimits::default()).result {
        Ok(report) => Ok(report),
        Err(failure) => Err(Box::new(failure)),
    }
}

pub fn scene_snapshot(report: &ImportReport) -> Result<SceneSnapshot, SceneSnapshotError> {
    let limits = SceneSnapshotLimits::try_new(16 * 1024, 64, 1_000_000)?;
    let snapshot = report.scene().capture_snapshot(limits)?;
    assert!(!snapshot.preview_bytes_truncated());
    assert!(!snapshot.preview_items_truncated());
    Ok(snapshot)
}

pub fn snapshot_line<'snapshot>(
    snapshot: &'snapshot SceneSnapshot,
    prefix: &str,
) -> &'snapshot str {
    snapshot
        .preview()
        .lines()
        .find(|line| line.trim_start().starts_with(prefix))
        .unwrap_or_else(|| panic!("missing canonical snapshot line: {prefix}"))
}

pub struct TestImportOutcome<T> {
    result: Result<T, ImportFailure>,
}

impl<T> TestImportOutcome<T> {
    pub fn new(result: ImportResult<T>) -> Self {
        let result = result.map_err(|error| {
            error.into_operation_failure().unwrap_or_else(|source| {
                panic!("test import operation could not initialize: {source}")
            })
        });
        Self { result }
    }

    pub fn expect(self, message: &str) -> T {
        match self.result {
            Ok(value) => value,
            Err(failure) => panic!("{message}: {failure:?}"),
        }
    }

    pub fn expect_err(self, message: &str) -> ImportFailure {
        match self.result {
            Ok(_) => panic!("{message}"),
            Err(failure) => failure,
        }
    }
}

pub fn import_bytes_with_limits(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
) -> TestImportOutcome<ImportReport> {
    let path = AssetPath::new(source).expect("test source path must be valid");
    let io = MemoryAssetIo::new();
    io.try_insert(path.clone(), Arc::<[u8]>::from(bytes))
        .expect("test asset insertion must succeed");
    let request = request(limits, false);
    TestImportOutcome::new(import_asset(
        &request,
        &io,
        path,
        ImportOperationControl::default(),
    ))
}

pub fn import_bytes_forced_with_limits(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
) -> TestImportOutcome<ImportReport> {
    import_bytes_forced_with_limits_and_control(
        source,
        bytes,
        limits,
        ImportOperationControl::default(),
    )
}

pub fn import_bytes_forced_with_limits_and_control(
    source: &str,
    bytes: &[u8],
    limits: ResourceLimits,
    control: ImportOperationControl,
) -> TestImportOutcome<ImportReport> {
    let path = AssetPath::new(source).expect("test source path must be valid");
    let io = MemoryAssetIo::new();
    io.try_insert(path.clone(), Arc::<[u8]>::from(bytes))
        .expect("test asset insertion must succeed");
    let request = request(limits, true);
    TestImportOutcome::new(import_asset(&request, &io, path, control))
}

pub fn expected_stl_failure(failure: &ImportFailure) -> &FormatFailureEnvelope {
    match failure.cause() {
        ImportCause::Format(envelope) if envelope.format().as_str() == "org.baozi.stl" => envelope,
        cause => panic!("expected an STL-owned format failure, got {cause:?}"),
    }
}

pub fn stl_unsigned_detail(envelope: &FormatFailureEnvelope, key: &str) -> Option<u64> {
    envelope.details().iter().find_map(|detail| {
        if detail.key() == key {
            match detail.value() {
                FormatFailureDetailValue::Unsigned(value) => Some(value),
                _ => None,
            }
        } else {
            None
        }
    })
}

pub fn stl_string_detail<'failure>(
    envelope: &'failure FormatFailureEnvelope,
    key: &str,
) -> Option<&'failure str> {
    envelope.details().iter().find_map(|detail| {
        if detail.key() == key {
            match detail.value() {
                FormatFailureDetailValue::String(value) => Some(value.as_str()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn request(limits: ResourceLimits, forced: bool) -> PreparedImportRequest {
    let mut builder = RegistryBuilder::new();
    builder
        .register(StlImporter)
        .expect("STL registration must be valid");
    let snapshot = builder
        .freeze(RegistryBuildLimits::default())
        .expect("STL registry must freeze");
    let mut config = ImportConfigOverlay::default()
        .with_resource_limits(ResourceLimitsOverlay::from_limits(limits));
    if forced {
        config = config.with_format_selection(FormatSelection::Forced(
            FormatId::new(FORMAT_ID).expect("static STL format id must be valid"),
        ));
    }
    snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(config),
            ),
            OptionCompileLimits::default(),
        )
        .expect("STL request must compile")
}

fn push_vec3(bytes: &mut Vec<u8>, values: [f32; 3]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}
