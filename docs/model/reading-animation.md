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

## Private name storage revision

Animation schema revision 2 adds a shared UTF-8 clip-name leaf. Directory fields
19–21 contain its byte count, presence, and reference; clip fields 9–12 contain
name presence, byte offset, byte count, and reference. An absent name and an
authored empty name both use zero byte count and a zero reference sentinel, with
distinct presence values. Nonempty name ranges partition the shared leaf in
clip order. Construction accounts for name bytes, references, payload blocks,
and validation work. Scene edits inherit this leaf with the other animation
payloads.

The snapshot transcript appends optional tag `223` after a clip's fixed fields
when its name is present. The tag is followed by an eight-byte little-endian
UTF-8 byte length and exactly that many borrowed bytes. This framing distinguishes
an absent name, an authored empty name, and different UTF-8 contents. It also
keeps embedded tag bytes unambiguous. Unnamed clips and non-animation transcript
encodings are unchanged. Snapshot name hashing copies no intermediate string.
