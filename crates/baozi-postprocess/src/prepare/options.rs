use crate::{NormalOptions, TangentOptions};
use baozi_core::FrontFaceWinding;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostProcessBudgets {
    /// Maximum attempted/consumed semantic actions under `bounded-normal-actions/v2`.
    /// Compound actions have fixed bounds; control-cost credits are not work charges.
    pub max_work_actions: u64,
    pub storage_bytes: u64,
}
impl Default for PostProcessBudgets {
    fn default() -> Self {
        Self {
            max_work_actions: crate::operation::runtime::MAX_WORK,
            storage_bytes: crate::operation::runtime::MAX_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TriangleLowering {
    /// Require existing TriangleList topology without changing its corner order.
    Preserve,
    /// Triangulate polygons and lower strips/fans to lists in exact source assembly order.
    #[default]
    Lists,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NormalOrientation {
    /// Cross product of the exact retained triangle tuple.
    #[default]
    StoredOrder,
    /// Outward normals according to the global declaration; Unknown needs an explicit assumption.
    SceneFrontFace {
        assume_unknown: Option<FrontFaceWinding>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TangentPolicy {
    /// Preserve existing tangents, rejecting an actual normal replacement that invalidates them.
    #[default]
    Preserve,
    Remove,
    Generate(TangentOptions),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrianglePreparationOptions {
    pub lowering: TriangleLowering,
    pub normals: Option<NormalOptions>,
    pub tangents: TangentPolicy,
    pub orientation: NormalOrientation,
    pub budgets: PostProcessBudgets,
}
impl TrianglePreparationOptions {
    pub(crate) fn digest(self) -> baozi_core::__scene_integration::authoring::OptionsDigest {
        let mut h = Sha256::new();
        h.update(b"baozi.prepare-triangles/v1\0");
        h.update([u8::from(self.lowering == TriangleLowering::Lists)]);
        match self.normals {
            Some(n) => {
                h.update([1]);
                h.update(n.digest().as_bytes());
            }
            None => h.update([0]),
        }
        match self.tangents {
            TangentPolicy::Preserve => h.update([0]),
            TangentPolicy::Remove => h.update([1]),
            TangentPolicy::Generate(t) => {
                h.update([2]);
                h.update(t.digest().as_bytes());
            }
        }
        h.update(match self.orientation {
            NormalOrientation::StoredOrder => [0, 0],
            NormalOrientation::SceneFrontFace { assume_unknown } => [
                1,
                match assume_unknown {
                    None => 0,
                    Some(FrontFaceWinding::Clockwise) => 1,
                    Some(FrontFaceWinding::CounterClockwise) => 2,
                    Some(_) => 3,
                },
            ],
        });
        baozi_core::__scene_integration::authoring::OptionsDigest::sha256(h.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::TrianglePreparationOptions;
    use crate::{ExistingNormals, NormalMode, NormalOptions};

    #[test]
    fn digest_matches_independent_v1_normal_composition_vectors() {
        // Literal expected hashes use independent nested normal transcripts, not NormalOptions::digest.
        // Outer framing: Lists, normal presence/hash, Preserve tangents, StoredOrder.
        for (normals, expected) in [
            (
                None,
                [
                    0xee, 0x60, 0xcb, 0x6c, 0x2b, 0x0f, 0x93, 0x05, 0xeb, 0xba, 0xd2, 0x80, 0x9d,
                    0x62, 0xbc, 0x9b, 0xba, 0xae, 0x1b, 0x30, 0x7b, 0xf1, 0x01, 0x9d, 0xc1, 0x65,
                    0x30, 0x29, 0xc8, 0x5d, 0x08, 0xd8,
                ],
            ),
            (
                Some(NormalOptions {
                    mode: NormalMode::Smooth,
                    existing: ExistingNormals::Preserve,
                }),
                [
                    0xac, 0x27, 0xeb, 0x33, 0x4b, 0xca, 0x38, 0xc7, 0xee, 0x29, 0x63, 0xcc, 0xc4,
                    0x0d, 0xb9, 0xe9, 0xa2, 0x4b, 0xf2, 0x9b, 0x4e, 0x09, 0xf2, 0x3f, 0xed, 0x7a,
                    0x3f, 0xba, 0x31, 0xed, 0xa3, 0x0c,
                ],
            ),
            (
                Some(NormalOptions {
                    mode: NormalMode::Smooth,
                    existing: ExistingNormals::Replace,
                }),
                [
                    0x7a, 0xed, 0x65, 0xa6, 0x79, 0xb1, 0x02, 0x53, 0x4c, 0x53, 0xcc, 0xba, 0xb1,
                    0xe0, 0xe0, 0x66, 0xcc, 0x72, 0x4d, 0x3f, 0x8b, 0x69, 0x20, 0x02, 0x36, 0xd3,
                    0x29, 0xca, 0x7e, 0xa9, 0x84, 0xb4,
                ],
            ),
        ] {
            let options = TrianglePreparationOptions {
                normals,
                ..TrianglePreparationOptions::default()
            };
            assert_eq!(options.digest().as_bytes(), &expected, "{normals:?}");
        }
    }
}
