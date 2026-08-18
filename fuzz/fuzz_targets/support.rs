use baozi_import::authoring::FormatId;
use baozi_import::{
    FormatImporter, FormatSelection, ImportConfigOverlay, ImportRequest, ImportRequestLayer,
    OptionCompileLimits, PreparedImportRequest, RegistryBuildLimits, RegistryBuilder,
    SceneSnapshotLimits, ValidatedScene,
};

const SNAPSHOT_PREVIEW_BYTES: usize = 64 * 1024;
const SNAPSHOT_COLLECTION_ITEMS: usize = 256;
const SNAPSHOT_WORK: u64 = 4_000_000;

pub fn prepared_request<I>(importer: I, format_id: &str) -> PreparedImportRequest
where
    I: FormatImporter,
{
    let mut registry = RegistryBuilder::new();
    registry
        .register(importer)
        .expect("the statically configured fuzz importer must register");
    let snapshot = registry
        .freeze(RegistryBuildLimits::default())
        .expect("the statically configured fuzz registry must freeze");
    let format = FormatId::new(format_id).expect("the static fuzz format id must be valid");
    snapshot
        .compile_request(
            &ImportRequest::from_layers(
                ImportRequestLayer::default(),
                ImportRequestLayer::new(
                    ImportConfigOverlay::default()
                        .with_format_selection(FormatSelection::Forced(format)),
                ),
            ),
            OptionCompileLimits::default(),
        )
        .expect("the forced fuzz format must exist in its frozen registry")
}

pub fn assert_validated_snapshot(scene: &ValidatedScene) {
    let limits = SceneSnapshotLimits::try_new(
        SNAPSHOT_PREVIEW_BYTES,
        SNAPSHOT_COLLECTION_ITEMS,
        SNAPSHOT_WORK,
    )
    .expect("the static fuzz snapshot limits must be valid");
    scene
        .capture_snapshot(limits)
        .expect("every successfully imported bounded fuzz Scene must admit a semantic snapshot");
}
