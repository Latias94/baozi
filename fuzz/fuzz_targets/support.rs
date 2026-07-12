use baozi_core::FormatId;
use baozi_import::{
    FormatImporter, FormatSelection, ImportConfigOverlay, ImportRequest, ImportRequestLayer,
    OptionCompileLimits, PreparedImportRequest, RegistryBuildLimits, RegistryBuilder,
};

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
