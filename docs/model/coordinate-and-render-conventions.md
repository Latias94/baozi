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
- meters
- column-vector transform convention
- local-to-parent node transforms
- counter-clockwise front faces unless target options say otherwise

Normalization is a post-process decision, not a parser side effect.

## UV Origin

Raw scenes record source UV origin when known.

Texture coordinates are not flipped during import. Use a `FlipUvs` or coordinate target post-process step when a renderer expects a different origin.

## Winding

Raw scenes preserve source winding when possible.

Coordinate conversion and handedness conversion must update winding or emit diagnostics when winding cannot be preserved safely.

## Tangents and Normal Maps

Generated tangents store handedness in tangent `w`.

Normal maps are data textures:

- no sRGB conversion
- no implicit green-channel flip
- normal map convention metadata should be recorded when known

Coordinate conversion must update tangent basis or invalidate tangents with diagnostics.

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
