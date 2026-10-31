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
