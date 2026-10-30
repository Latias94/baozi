use super::*;
use crate::canonical_scene::store::typed_storage::{StoredNodeIndex, TableRange, TextRange};
use crate::{Axis, FrontFaceWinding, Handedness, Mat4, Quat, SceneSpace, TargetSpace, Vec3};

#[derive(Default)]
struct Control {
    work: u64,
    stop: Option<u64>,
    omit: Option<u64>,
}

impl Control {
    fn run<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
        if self.stop == Some(self.work) {
            return Err(self.work);
        }
        let result = action();
        self.work += 1;
        Ok(result)
    }
}

impl TypedStorageBuildControl for Control {
    type Error = u64;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
        self.run(action)
    }
    fn run_string_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<R, u64> {
        self.run(action)
    }
}

impl TypedStorageValidationControl for Control {
    type Error = u64;
    fn run_unit<R>(&mut self, action: impl FnOnce() -> R) -> Result<Option<R>, u64> {
        if self.omit == Some(self.work) {
            return Ok(None);
        }
        self.run(action).map(Some)
    }
}

fn conversion() -> SpaceConversion {
    SpaceConversion::try_new(
        &SceneSpace::UNKNOWN,
        TargetSpace::try_new(
            Handedness::Left,
            Axis::PositiveY,
            Axis::NegativeZ,
            0.01,
            FrontFaceWinding::CounterClockwise,
        )
        .unwrap(),
        Some(TargetSpace::STANDARD),
    )
    .unwrap()
}

fn nodes() -> NodeStorage {
    NodeStorage {
        transform_scalar_count: 26,
        nodes: vec![
            StoredNode {
                children: TableRange::new(0, 1),
                transform: StoredTransform::Synthetic,
                name: None,
                camera: None,
                light: None,
            },
            StoredNode {
                children: TableRange::new(1, 1),
                transform: StoredTransform::Matrix(Mat4 {
                    cols: [
                        [1.0, 0.0, 6.0, 0.0],
                        [2.0, 0.0, 0.0, 0.0],
                        [0.0, 3.0, 1.0, 0.0],
                        [4.0, 5.0, 7.0, 1.0],
                    ],
                }),
                name: Some(TextRange(TableRange::new(0, 6))),
                camera: Some(0),
                light: None,
            },
            StoredNode {
                children: TableRange::new(2, 0),
                transform: StoredTransform::Trs {
                    translation: Vec3::new(1.0, -2.0, 3.0),
                    rotation: Quat::IDENTITY,
                    scale: Vec3::new(-2.0, 0.0, 4.0),
                },
                name: Some(TextRange(TableRange::new(6, 5))),
                camera: None,
                light: Some(1),
            },
        ],
        children: vec![StoredNodeIndex(1), StoredNodeIndex(2)],
        names: b"parentchild".to_vec(),
    }
}

#[test]
fn matrix_and_trs_preserve_authored_form_hierarchy_and_singular_scale() {
    let source = nodes();
    let plan = NodeSpacePlan::try_new(&source, conversion(), &mut Control::default()).unwrap();
    let mut build = Control::default();
    let actual = plan.materialize(&mut build).unwrap();
    assert_eq!(build.work, plan.work());
    assert_eq!(actual.transform_scalar_count, 26);
    assert_eq!(actual.children, source.children);
    assert_eq!(actual.names, b"parentchild");
    let StoredTransform::Matrix(matrix) = actual.nodes[1].transform else {
        panic!("authored matrix");
    };
    assert_eq!(
        matrix,
        Mat4 {
            cols: [
                [1.0, 0.0, -6.0, 0.0],
                [-2.0, 0.0, 0.0, 0.0],
                [0.0, 3.0, 1.0, 0.0],
                [-400.0, 500.0, 700.0, 1.0],
            ]
        }
    );
    let StoredTransform::Trs {
        translation,
        rotation,
        scale,
    } = actual.nodes[2].transform
    else {
        panic!("authored TRS");
    };
    assert_eq!(translation, Vec3::new(-100.0, -200.0, 300.0));
    assert_eq!(rotation, Quat::IDENTITY);
    assert_eq!(scale, Vec3::new(-2.0, 0.0, 4.0));
    let mut validation = Control::default();
    assert!(
        plan.validate_correspondence(&actual, &mut validation)
            .unwrap()
    );
    assert_eq!(validation.work, plan.correspondence_work());
}

#[test]
fn correspondence_rejects_changed_structure_or_authored_bits_and_requires_each_action() {
    let source = nodes();
    let plan = NodeSpacePlan::try_new(&source, conversion(), &mut Control::default()).unwrap();
    let mut actual = plan.materialize(&mut Control::default()).unwrap();
    actual.nodes[1].camera = None;
    assert!(
        !plan
            .validate_correspondence(&actual, &mut Control::default())
            .unwrap()
    );
    actual.nodes[1].camera = Some(0);
    actual.names[0] = b'P';
    assert!(
        !plan
            .validate_correspondence(&actual, &mut Control::default())
            .unwrap()
    );
    actual.names[0] = b'p';
    actual.nodes[2].transform = StoredTransform::Matrix(Mat4::IDENTITY);
    assert!(
        !plan
            .validate_correspondence(&actual, &mut Control::default())
            .unwrap()
    );
    let actual = plan.materialize(&mut Control::default()).unwrap();
    let mut omitted = Control {
        omit: Some(plan.correspondence_work() - 1),
        ..Control::default()
    };
    assert!(!plan.validate_correspondence(&actual, &mut omitted).unwrap());
}

#[test]
fn numeric_error_identifies_the_public_node_before_materialization() {
    let mut source = nodes();
    source.nodes[2].transform = StoredTransform::Trs {
        translation: Vec3::new(f32::MAX, 0.0, 0.0),
        rotation: Quat::IDENTITY,
        scale: Vec3::new(1.0, 1.0, 1.0),
    };
    assert!(matches!(
        NodeSpacePlan::try_new(&source, conversion(), &mut Control::default()),
        Err(NodeSpacePlanError::Numeric {
            node: 1,
            source: NumericConversionError::NonFiniteOutput
        })
    ));
    let mut control = Control {
        stop: Some(1),
        ..Control::default()
    };
    assert!(matches!(
        NodeSpacePlan::try_new(&source, conversion(), &mut control),
        Err(NodeSpacePlanError::Control(1))
    ));
}

#[test]
fn every_materialization_action_can_stop_without_mutating_the_source() {
    let source = nodes();
    let plan = NodeSpacePlan::try_new(&source, conversion(), &mut Control::default()).unwrap();
    for stop in 0..plan.work() {
        let mut control = Control {
            stop: Some(stop),
            ..Control::default()
        };
        assert!(
            matches!(plan.materialize(&mut control), Err(TypedStorageBuildError::Control(value)) if value == stop)
        );
        assert_eq!(source.names, b"parentchild");
        assert_eq!(
            source.nodes[1]
                .transform
                .authored()
                .unwrap()
                .authored_matrix()
                .unwrap()
                .cols[3][0],
            4.0
        );
    }
}
