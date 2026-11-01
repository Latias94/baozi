use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct StopAt {
    polls: Arc<AtomicUsize>,
    at: usize,
}
impl baozi_operation::CancellationSignal for StopAt {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::Relaxed) >= self.at
    }
}

fn fixture() -> baozi_import::ImportReport {
    let mut obj = String::from("v 0 0 0\nv 1 0 0\nv 0 1 0\n");
    for i in 0..32 {
        obj.push_str(&format!("o selected-{i}\nf 1 2 3\n"));
    }
    crate::test_fixtures::import_obj(obj.as_bytes())
}

#[test]
fn cancellation_during_later_preparation_validation_and_publication_returns_original() {
    let polls = Arc::new(AtomicUsize::new(0));
    let (scene, _) = fixture().into_parts();
    let targets: Vec<_> = scene
        .meshes()
        .flat_map(|m| m.primitives().map(|p| p.id()))
        .collect();
    let options = TrianglePreparationOptions {
        normals: Some(crate::NormalOptions::default()),
        ..Default::default()
    };
    let baseline = prepare_triangles(
        scene,
        &targets,
        options,
        PostProcessControl::default().with_cancellation_signal(StopAt {
            polls: polls.clone(),
            at: usize::MAX,
        }),
    )
    .unwrap();
    assert_eq!(
        baseline.report().geometry_preparation().unwrap().len(),
        targets.len()
    );
    drop(baseline);
    let completed = polls.load(Ordering::Relaxed);
    assert!(completed > 100);
    for at in [completed / 3, completed * 2 / 3, completed - 3] {
        let (scene, import) = fixture().into_parts();
        let targets: Vec<_> = scene
            .meshes()
            .flat_map(|m| m.primitives().map(|p| p.id()))
            .collect();
        let positions: Vec<_> = scene
            .meshes()
            .flat_map(|m| m.primitives())
            .map(|p| match p.positions().unwrap().data() {
                baozi_core::AttributeData::DenseF32 { values, .. } => {
                    values.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
                }
                _ => panic!("OBJ positions"),
            })
            .collect();
        let generation = scene.generation();
        let failure = prepare_triangles(
            scene,
            &targets,
            options,
            PostProcessControl::default().with_cancellation_signal(StopAt {
                polls: Arc::new(AtomicUsize::new(0)),
                at,
            }),
        )
        .unwrap_err();
        assert_eq!(failure.input().generation(), generation);
        assert!(import.matches_scene(failure.input()));
        for (p, expected) in failure
            .input()
            .meshes()
            .flat_map(|m| m.primitives())
            .zip(&positions)
        {
            assert!(p.normals(baozi_core::AttributeDomain::Corner).is_none());
            let baozi_core::AttributeData::DenseF32 { values, .. } = p.positions().unwrap().data()
            else {
                panic!()
            };
            assert_eq!(
                values.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                *expected
            );
        }
        let PostProcessRunError::Preparation { failure } = failure.error() else {
            panic!("preparation failure")
        };
        if let Some(resources) = failure.resources() {
            assert_eq!(resources.current_live_bytes(), 0);
            assert_eq!(resources.active_reservations(), 0);
        }
    }
}
