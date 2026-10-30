# Coordinate and Render Conventions

This document summarizes the implementation-facing conventions from ADR 0008, ADR 0012, and ADR 0013.

## Raw Import

Baozi preserves source coordinates by default.

Importers should record source metadata when known:

- handedness
- up axis
- front axis
- unit scale to meters
- UV origin
- winding convention
- tangent basis convention

Importers must not silently convert coordinate systems, flip UVs, flip winding, or change units for renderer convenience.

## Normalized Target

The default normalized target for explicit post-processing is:

- right-handed coordinates
- Y-up
- negative Z front
- meters
- counter-clockwise front faces unless target options say otherwise

Matrices always use column vectors, and authored node transforms always map local coordinates to
the parent. TRS means `T * R * S`. These are fixed API semantics rather than conversion options;
other matrix domains define their own source and destination spaces.

`TargetSpace::try_new` validates all five target facts. `convert_scene_space` performs the explicit
conversion. Unknown source facts require `SpaceConversionOptions::source_assumptions`; assumptions
cannot override declared facts. The report records fields actually filled from assumptions.
Normalization remains a post-process decision rather than a parser side effect.

## UV Origin

Raw scenes record source UV origin when known.

Texture coordinates are not flipped during import. Coordinate conversion preserves the UV origin.
A caller needing another origin must perform an explicit separately supported conversion; no public
`FlipUvs` operation is currently provided.

## Winding

Raw scenes preserve source winding when possible.

Coordinate conversion reverses triangle index lanes and all Corner attribute lanes together when
the basis determinant and independent winding convention require it. Vertex and Face identities
retain their order. A nonempty polygon/strip/fan requiring reversal must first be lowered through
an implemented operation; conversion returns a typed capability error otherwise.

## Tangents and Normal Maps

Generated tangents store handedness in tangent `w`.

Normal maps are data textures:

- no sRGB conversion
- no implicit green-channel flip
- normal map convention metadata should be recorded when known

Coordinate conversion transports normals and tangent directions by the signed basis permutation.
It multiplies tangent `w` by the basis determinant when the encoding is `SignInW`; it does not
normalize authored vectors or alter the tangent-generation declaration. A reflection with an
unhandled present tangent encoding returns a typed capability error.

## Units

Raw scenes preserve source unit metadata when known.

The normalized unit target is meters. Unitless formats such as STL and OBJ remain unitless unless the caller supplies an option or a format sidecar provides units.

## Cameras, Lights, and Animation

Coordinate normalization must apply consistently to:

- mesh positions and normals
- node transforms
- cameras
- lights
- skeleton bind poses
- animation channels
- morph targets

If a post-process step cannot update a feature correctly, it must diagnose or reject that scene rather than silently producing inconsistent output.

Skin inverse binds and optional corrections use `H M H^-1`, with `H = diag(sQ, 1)`.
Dense and sparse Position morph deltas use `sQ`; Normal and Tangent deltas use `Q`.
Sparse omissions, explicit entries, target identities, influence rows, and morph weights remain unchanged.

Animation translation values and cubic derivatives use `sQ`. Scale values and derivatives
permute axes without sign changes. Quaternion samples and arbitrary cubic derivatives use
`(det(Q) Q xyz, w)`. This linear map preserves `q/-q`, zero/nonunit derivatives, and scalar bits
except for the declared permutation, sign, and unit arithmetic. No normalization or quaternion
sign canonicalization occurs. Names, source order, f64 key times/time domains, and interpolation
and extrapolation declarations remain unchanged.

Camera and light definitions carry complete intrinsic `LocalAxes`. With reference-to-node map
`B = [right, up, -forward]`, conversion applies `B' = Q B`, preserving all 48 signed permutations
and leaving the node hierarchy and attachments unchanged. Camera clipping distances and orthographic
half-extents scale by `s`; point/spot ranges do likewise. FOV, aspect, cone angles, tint, and
photometric intensity keep their bits. Projection ordering must remain valid after F32 rounding.

Conversion still rejects numeric conversion of custom attributes, spatial texture-coordinate
sources, or nonzero Scene-unit material values
it cannot convert. Winding-only conversion can retain otherwise unaffected spatial domains.
These typed rejections preserve the original Scene. Unrepresentable numeric output identifies the
skin correction, inverse-bind matrix, morph delta/tuple, animation channel/stream/key, or
camera/light definition before
retained output admission. Winding-only and declaration-only edits share deformation and animation storage.
