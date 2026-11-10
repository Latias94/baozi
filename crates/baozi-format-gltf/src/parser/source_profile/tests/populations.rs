//! Actual directory populations on the original analytic inputs, without a second algorithm.

use super::*;
use sha2::{Digest, Sha256};

#[test]
fn original_comparison_inputs_report_directory_populations() {
    for (primitives, name, bytes, digest, baseline_work, old_populations, delta) in [
        (
            1,
            "gltf-shared-1.glb",
            include_bytes!("corpus/gltf-shared-1.glb").as_slice(),
            "983f52f10d23012604142c6ec013026a5dd1c02b8680b83628df7c6ffa4b755c",
            [11_911_u64, 13_109],
            [52_u64, 11, 65],
            -91_i64,
        ),
        (
            8,
            "gltf-shared-8.glb",
            include_bytes!("corpus/gltf-shared-8.glb").as_slice(),
            "a464970f7d6a3b3f6d7b721b1f7bbaef8068669de48cd4112b35d48563e05ab2",
            [24_764, 25_962],
            [312, 39, 287],
            -517,
        ),
        (
            32,
            "gltf-shared-32.glb",
            include_bytes!("corpus/gltf-shared-32.glb").as_slice(),
            "27bf60f547837af37e35c88d1482cf56659513359d5f6cfe7a38f76418293592",
            [70_332, 71_596],
            [1_539, 135, 1_233],
            -2_498,
        ),
    ] {
        let actual_digest: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual_digest, digest);
        for (source_mode, baseline_work) in ["bytes", "memory-asset"].into_iter().zip(baseline_work)
        {
            let guard = CompletionGuard::new(None);
            let mut registry = RegistryBuilder::new();
            crate::register(&mut registry).unwrap();
            let mut limits = ResourceLimitsOverlay::default();
            limits.max_work_units = Some(200_000_000);
            let request = ImportRequest::new().with_caller_layer(ImportRequestLayer::new(
                ImportConfigOverlay::default().with_resource_limits(limits),
            ));
            let plan = registry
                .freeze(RegistryBuildLimits::default())
                .unwrap()
                .plan(&request, OptionCompileLimits::default())
                .unwrap();
            let path = AssetPath::new(name).unwrap();
            let report = if source_mode == "bytes" {
                plan.read_bytes(path, bytes, ImportOperationControl::default())
            } else {
                let io = MemoryAssetIo::new();
                io.try_insert(path.clone(), Arc::<[u8]>::from(bytes))
                    .unwrap();
                plan.read_asset(&io, path, ImportOperationControl::default())
            }
            .unwrap();
            let (scene, evidence) = report.into_parts();
            assert_eq!(scene.mesh_count(), 1);
            assert_eq!(scene.primitive_count(), primitives);
            assert_eq!(scene.mesh_instance_count(), 2);
            let row = evidence
                .ledger()
                .dimension(ImportDimension::WorkUnits)
                .unwrap();
            assert_eq!(row.attempted(), row.consumed());
            let population = COMPLETION_PROBE.with(|slot| slot.borrow_mut().take().unwrap());
            let owners = 4 * primitives + 8;
            assert_eq!(population.directory_covered, owners);
            assert_eq!(population.directory_queries, owners + 1);
            assert_eq!(
                population.completed[ComparisonSite::DirectoryInit as usize],
                owners,
            );
            assert_eq!(
                population.completed[ComparisonSite::DirectoryRegister as usize],
                owners,
            );
            assert_eq!(population.directory_query_work, (owners + 1) as u64);
            assert_eq!(
                population.completed[ComparisonSite::OrdinalTransfer as usize],
                22 * primitives + 10,
            );
            assert_eq!(
                row.consumed(),
                baseline_work.checked_add_signed(delta).unwrap()
            );
            let removed_work = old_populations.into_iter().sum::<u64>();
            let actual_directory_work = population.completed[ComparisonSite::DirectoryInit as usize]
                as u64
                + population.completed[ComparisonSite::DirectoryRegister as usize] as u64
                + population.directory_query_work;
            assert_eq!(
                i128::from(actual_directory_work) - i128::from(removed_work),
                i128::from(delta)
            );
            assert_eq!(
                baseline_work - removed_work,
                row.consumed() - actual_directory_work
            );
            println!(
                "DIRECTORY_POPULATION {}",
                serde_json::json!({
                    "schema": "baozi-gltf-directory-population/v1-dense",
                    "input": name,
                    "input_sha256": digest,
                    "source_mode": source_mode,
                    "registry": "single-gltf-format",
                    "scope": "unit directory populations; not full-builtin/native-file worker totals",
                    "max_work_units": 200_000_000_u64,
                    "unit_import_work": row.consumed(),
                    "baseline_unit_import_work": baseline_work,
                    "declared_work_delta": delta,
                    "baseline_directory_work": removed_work,
                    "actual_directory_work": actual_directory_work,
                    "owners": population.directory_covered,
                    "queries": population.directory_queries,
                    "query_work": population.directory_query_work,
                    "directory_init": population.completed[ComparisonSite::DirectoryInit as usize],
                    "directory_register": population.completed[ComparisonSite::DirectoryRegister as usize],
                    "ordinal_transfer": population.completed[ComparisonSite::OrdinalTransfer as usize],
                }),
            );
            drop((scene, evidence, guard));
        }
    }
}
