# Reading retained animation

`ValidatedScene::animation_clips()` preserves retained source order. Each `AnimationClipRef`
has a checked, generation-qualified `AnimationClipId`, an optional borrowed name,
an `f64` duration, and the authored seconds, frames, or ticks time domain. Frames
retain their rate; ticks retain either a known rate or an unknown rate.

`clip.channels()` and `clip.channel(index)` read the same ordered channel range.
Each channel exposes its typed `NodeId` or `MeshInstanceId` target, interpolation,
pre/post extrapolation, `&[f64]` key times, and key-major `&[f32]` values. The
explicit `value_width()` gives the scalar count per key. Cubic in/out tangents
remain separate optional slices with the same shape. These views borrow the
validated canonical Scene and allocate no memory. Independent iterators and
random reads use the row's recorded offsets without traversing preceding rows.

The API exposes authored data for application-side use; it does not evaluate
animation curves or convert time domains.

## Application CPU reference

The independent [asset consumer](../../tools/consumers/assets/README.md) copies nodes, instances,
curves, morphs, influences and skin palettes through these public readers. Its
[`evaluate`](../../tools/consumers/assets/src/evaluation.rs) function accepts an owned
`ApplicationAsset`, optional clip index and `SampleTime::{Declared, Seconds}`. Copied data and
evaluated results remain usable after the imported Scene, IO and import evidence are dropped.
The module belongs to the application example; core retains the source contracts.

The reference evaluates Step, Linear and CubicSpline TRS/morph channels. Linear rotations use
shortest-path normalized slerp. Cubic rotations use component Hermite interpolation with the
actual key interval, then normalize the result; quaternion derivatives and individual key signs
are retained. Time and rate arithmetic uses F64. A sample in seconds requires a known rate for
frame/tick clips; a sample in the declared domain also supports ticks with an unknown rate.

Extrapolation remains explicit: `Constant` clamps to the curve endpoint, whereas `SourceDefault`
uses the authored TRS component or effective initial morph weights. The glTF profile declares
Constant. Unsupported extrapolation and duplicate channel targets return named errors.
Mesh morph defaults apply when an instance has no override; an authored override and a sampled
channel retain their distinct precedence. Matrix-authored nodes remain matrices and are not
implicitly decomposed for TRS channels.

Hierarchy composition produces parent-world times local. Morph deltas apply before skinning.
Skin positions use the instance-specific palette and inverse binds, retaining every influence
and its authored weight. The weighted homogeneous component participates in world/local
conversion, so a weight sum other than one is preserved rather than silently normalized.
Unsupported skin-space/correction contracts and an instance transform whose inverse cannot be
established return typed errors. The reference does not blend clips, retarget, schedule playback,
or skin shading frames.

The [analytic tests](../../tools/consumers/assets/tests/evaluation.rs) use independent numeric
expectations for curves, morph precedence, two-joint skins and reflected centimeter conversion.
Executed source/target qualification is recorded separately in the
[maturation review](../reviews/2026-11-02-first-release-maturation.md).

## Typed storage and borrowed lifetimes

Private animation storage has two typed row tables, for clips and channels, and
five contiguous pools: f64 times, f32 values, f32 input/output tangents, and UTF-8
names. Rows hold checked ranges into those pools. A clip's optional name range
distinguishes an absent name from an authored empty name; `clip.name()` returns
`None` or `Some("")` accordingly.

One retained owner binds these row tables and pools to their allocation charges.
Central validation borrows the actual rows and payloads. An edit that leaves
animation unchanged shares that owner, so its names and numeric pools remain
live after the source Scene is dropped. The final owner destroys storage before
releasing its charge.

Public clip/channel views, names, and scalar slices borrow the particular Scene
used to obtain them. They remain usable after a temporary view or iterator is
dropped, but cannot outlive that Scene. Construction and validation account for
the actual rows, pool storage, and traversal work; reading does not build a
replacement model.

## Snapshot name framing

The snapshot transcript appends optional tag `223` after a clip's fixed fields
when its name is present. The tag is followed by an eight-byte little-endian
UTF-8 byte length and exactly that many borrowed bytes. This framing distinguishes
an absent name, an authored empty name, and different UTF-8 contents. It also
keeps embedded tag bytes unambiguous. Unnamed clips and non-animation transcript
encodings are unchanged. Snapshot name hashing copies no intermediate string.
