use crate::{NormalOptions, TangentOptions};
use baozi_core::FrontFaceWinding;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostProcessBudgets {
    pub work: u64,
    pub storage_bytes: u64,
}
impl Default for PostProcessBudgets {
    fn default() -> Self {
        Self {
            work: crate::operation::runtime::MAX_WORK,
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
