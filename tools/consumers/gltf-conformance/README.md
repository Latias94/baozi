# Direct glTF selected-profile conformance consumer

This unpublished, standalone workspace demonstrates single-format use through public
`baozi-format-gltf`, `baozi-import`, and `baozi-io` APIs. It is a finite set of
selected-profile probes, not a full glTF conformance certificate, a public raw-document
library, or a gltf-rs compatibility layer. The root workspace excludes this directory;
root workspace tests do not execute it.

Production dependencies are exactly those three crates, with default features disabled.
Development dependencies are exactly `baozi-core` for naming public reader types and
`gltf = "=1.4.1"`. The reference enables `utils`, `names`, `extras`, `extensions`,
`KHR_materials_emissive_strength`, `KHR_materials_unlit`, `KHR_texture_transform`, and
`KHR_lights_punctual`. It has no `import` feature, image codec, native IO adapter, facade,
or repository test-support dependency. The main-owned `Cargo.lock` fixes both `gltf`
and its transitive `gltf-json` to 1.4.1 with the reviewed archive checksums. This
consumer does not declare a direct `gltf-json` dependency.
Fixture bytes use ordinary Rust constants and fixed container assembly; neither loader
nor its JSON schema/serialization/import helpers creates the fixtures or expectations.

## Public consumer API

| API | Purpose |
| --- | --- |
| `register()` | Register only the public glTF plugin and freeze a `RegistrySnapshot`. |
| `compile(&snapshot, selection)` | Compile forced glTF options with explicit `GltfSceneSelection` and `preserve_or_fail` loss policy. |
| `import_embedded(&request, path, bytes, read)` | Import GLB or data URI JSON without secondary asset authority. |
| `import_from_io(&request, &io, path, read)` | Import through the supplied existing `AssetIo` authority, including selected external buffers. |
| `Observation<T>` | Return caller-selected owned facts and the original detached `ImportEvidence`; the Scene is destroyed before return. |
| `selected_fixtures()` | Return a fixed six-file list of original raw assets for tests and separate official validation. |

The read callback borrows the validated Scene only during extraction. It may return the
application's own small values or a fallible extraction result. It cannot return a view
borrowing the callback's Scene. No additional Scene, importer, schema, or resolver is
introduced. Import failures retain Baozi's typed `ImportRunError`; compile failures
remain separate. This example is another user of the public extension APIs, not an
additional stability promise for those experimental APIs.

```rust
use baozi_format_gltf::GltfSceneSelection;
use baozi_gltf_conformance_consumer::{compile, import_embedded, register, selected_fixtures};
use baozi_io::AssetPath;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = register()?;
    let request = compile(&registry, GltfSceneSelection::DocumentDefault)?;
    let bytes = selected_fixtures()
        .into_iter()
        .find_map(|(name, bytes)| (name == "selected.glb").then_some(bytes))
        .ok_or("missing fixed fixture")?;
    let observation = import_embedded(
        &request,
        AssetPath::new("example/selected.glb")?,
        &bytes,
        |scene| scene.meshes().filter_map(|mesh| mesh.name().map(str::to_owned)).collect::<Vec<_>>(),
    )?;
    drop((request, registry, bytes));
    assert_eq!(observation.facts, ["Selected mesh"]);
    assert!(observation.evidence.loss_evidence().is_empty());
    Ok(())
}
```

## Finite observations and differences

Semantic expectations use independently authored literals and rational values. Neither
engine's output supplies the other's expected answer. Baozi normalized colors must match
the exact-rational nearest-F32 baseline; the pinned reference's differing results are
separately labelled characterizations. Agreement is not a conformance verdict. The reference uses validated
`gltf::Gltf::from_slice` and readers supplied with the exact fixture buffer slices; it
does not resolve resources, import images, guess selection, or construct unchecked
documents. Baozi also supplies independent typed evidence assertions after detachment.

| Probe | Independent expectation and boundary |
| --- | --- |
| GLB, data URI, external buffer | Two selected primitives retain the same positions, indices, colors, UVs and material assignments. Eight prefix bytes, unequal offsets/strides, sparse replacement of vertex one, and untouched vertices distinguish common layout errors. |
| Sparse plus normalized streams | Source float positions preserve negative zero and F32 bits; normalized U8 colors/UVs cover zero, maximum and unequal middle values. Baozi must equal the independent rational's nearest-F32 bits; three one-ULP reference differences are recorded explicitly. |
| GLB logical versus physical bytes | The first buffer has 89 logical bytes and a distinct tail byte; the BIN chunk has 92 bytes including three zero padding bytes. This valid control does not replace the format crate's malformed-padding tests. |
| Selected scene and resources | Scene zero is the default; two nodes share mesh zero and two nodes share light zero. Buffer one belongs to scene one. Default external import receives no `omitted.bin`; explicit scene one receives no `selected.bin`. Successful resource identities and asset-open counts retain the distinction. |
| Descriptive facts | Original names and exact node extras bytes survive extraction; source selection request, source scene index, selected/omitted counts and `l0`/`l1` source sites remain typed evidence. |
| Emission and Unlit | A lit material has factor `[0.125, 0.5, 0.75]`, authored strength `8`, and effective linear RGB `[1, 4, 6]`. A separate Unlit material retains its base color and absent strength. This avoids the extensions' excluded combination. |
| UV transform and image | Base UV set one is overridden by set zero, with offset `[0.25, -0.5]`, scale `[2, 0.5]`, zero rotation and exact original encoded PNG bytes. Pixel decoding and rendering are outside this task. |
| Cameras | Asymmetric perspective and orthographic values plus an infinite perspective camera distinguish finite/absent planes and authored/absent aspect. Tests use glTF meters and radians, without unit conversion. |
| Punctual lights | Source red intensity `10000` and range `8` are checked in the reference. Baozi's unit-luminance tint and intensity are separately expected: Rec.709 red luminance gives `2126`, with tint red `10000 / 2126`. One retained definition has two placements; one unselected definition is omitted. Source RGB and canonical tint are different representations. |
| No declared default | Valid two-scene source has no default in gltf-rs. Baozi returns typed `SceneSelectionRequired` until scene zero is explicitly selected. This is policy, not evidence that the source is invalid. |
| Baozi-only negatives | A missing perspective payload, invalid scene-node reference and unsupported required extension retain format codes and the completed selection phase. External bytes without asset authority retain a dependency-policy denial. These do not invoke malformed reference readers. |

Binary positions have source/retained/output width F32. U8 normalization is observed
as F32 bits, while its independent source meaning is the rational numerator divided
by `255`. `COLOR_RATIONAL_NEAREST_F32` preserves the literal nearest-F32 baseline.
`GLTF_RS_1_4_1_COLOR_BITS_OBSERVATION` preserves the pinned reference's separately
observed behavior. The test also records the exact
per-engine ULP distance from the rational baseline, without adding an acceptance
tolerance or widening the bit assertions.

| Source rational | Nearest F32 bits | Baozi contract | gltf-rs 1.4.1 characterization |
| --- | --- | --- | --- |
| `3/255` | `0x3c40c0c1` | `0x3c40c0c1`, exact | `0x3c40c0c2`, +1 ULP |
| `127/255` | `0x3efefeff` | `0x3efefeff`, exact | `0x3efeff00`, +1 ULP |
| `200/255` | `0x3f48c8c9` | `0x3f48c8c9`, exact | `0x3f48c8ca`, +1 ULP |

Baozi now uses F64 division with one final F32 rounding, shared by attribute and influence
reads under [ADR 0027](../../../docs/adr/0027-gltf-backend-ownership-and-replacement-policy.md).
This realizes the exact-rational rounding contract for the bounded integer operands.
The reference retains its F32 reciprocal products. Khronos recommends at least 32-bit
conversion precision; a one-ULP difference alone is not a proven reference conformance
violation. Endpoints remain exact; no global epsilon hides the difference.
The chosen JSON camera/material values are exactly representable at F32. Effective
emission RGB is observed at F64; the light expectation uses F64 luminance arithmetic
and checks the separately rounded retained F32 fields. There is no separate F64 scene
output or broad precision qualification here.

Skin/morph/animation evaluation, all extension parity, alternate UV rotations, image
pixels, broad invalid-input probes, limits/cancellation matrices, sanitizers, fuzzing,
and portable-target execution retain their existing owners. In particular, the main
thread's valid implicit-zero accessor and absent-animation-target fixtures are separate
core-compatibility regressions. The published reference may reject valid source there;
record that as a reference rejection/non-comparable result, not required agreement or
an excuse to change literal source meaning. Newer source is not substituted for 1.4.1.
Any future panic-prone malformed gltf-rs probe needs a bounded child process with
distinct panic/crash/timeout/unavailable outcomes; none is included here.

## Commands and qualification status

Run from the repository root after the main thread has generated this workspace's
`Cargo.lock` and fetched any missing pinned dependencies. Use the shared `target`, one
build job, and sequential invocations. These are execution instructions, not recorded
passing results.

```powershell
cargo +1.95.0 nextest run --locked --offline --manifest-path tools/consumers/gltf-conformance/Cargo.toml --target-dir target --build-jobs 1 --test selected_profile --test-threads 1
cargo +1.95.0 clippy --locked --offline --manifest-path tools/consumers/gltf-conformance/Cargo.toml --target-dir target --all-targets --jobs 1 -- -D warnings
cargo +1.95.0 fmt --manifest-path tools/consumers/gltf-conformance/Cargo.toml --all --check
python scripts/check_packages.py --offline --toolchain 1.95.0 --gltf-conformance
```

The package command preserves the existing twelve facade cases and additionally runs
this source against version-only direct format/import/IO archive roots, with the exact
reference features. `--check` proves compilation only. Neither path-based execution nor
the offline archive rehearsal proves live registry availability or authorizes release.

On the source states recorded in the
[evolution execution record](../../../docs/reviews/2026-11-02-pre-release-evolution.md),
this excluded workspace passed 8/8 source tests and strict Clippy. Four exported documents
separately passed Validator 2.0.0-dev.3.10 with zero errors; the GLB retained one warning.
A focused version-only archive consumer also passed 8/8. These receipts retain their own
source, lockfile and toolchain identities. Fresh integrated U14 qualification of the final
source remains open; root workspace or U7 results do not qualify this excluded workspace.

## Separate official Validator invocation

`selected_fixtures()` returns `selected.glb`, `selected-data.gltf`,
`selected-external.gltf`, `no-default.gltf`, `selected.bin`, and `omitted.bin`.
The ignored export test writes exactly that list. Whole-document validation must
receive both buffers, including the buffer Baozi deliberately does not open for scene
zero. First export the actual Rust fixture output:

```powershell
$env:BAOZI_GLTF_FIXTURE_DIR = 'target/review/gltf-conformance/fixtures'
cargo +1.95.0 test --locked --offline --manifest-path tools/consumers/gltf-conformance/Cargo.toml --target-dir target --test selected_profile --jobs 1 export_validator_fixtures -- --ignored --exact
```

The following separate commands use the existing official cached artifact
`gltf-validator@2.0.0-dev.3.10`, git
`bcd52cc4ba5f333b2999a58f67cc05ddf28b4fb1`. They first verify the archive SHA-256
and every extracted regular file. If the cache is absent or differs, acquire and
verify that exact artifact before continuing; do not silently use an installed latest
package. Newer source `602c54043ba66856b2e68c40fdf2bced975cdb8a` is a separate
research reference, not the executable used by these instructions.

```powershell
@'
import hashlib, json, tarfile
from pathlib import Path
cache = Path("target/quaternion-compatibility")
archive = cache / "gltf-validator.tgz"
assert hashlib.sha256(archive.read_bytes()).hexdigest() == "4e03dbdc3bc0d1342afd5d2d7ae341a7e3474502ccb872b02dbbaddaee0fdec6"
with tarfile.open(archive, "r:gz") as bundle:
    for member in bundle.getmembers():
        if member.isfile():
            assert (cache / member.name).read_bytes() == bundle.extractfile(member).read(), member.name
package = json.loads((cache / "package/package.json").read_text())
assert package["version"] == "2.0.0-dev.3.10"
print("Verified pinned Validator archive and extracted files")
'@ | python -
```

After successful identity verification, validate the four fixed documents. This keeps
the complete issue reports, supported-extension inventory, byte hashes and failed
invocations. No issues are ignored or reclassified; `maxIssues: 0` requests all issues.

```powershell
@'
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const validator = require(path.resolve('target/quaternion-compatibility/package'));
const root = path.resolve('target/review/gltf-conformance');
const fixtures = path.join(root, 'fixtures');
const reports = path.join(root, 'validator');
fs.mkdirSync(reports, {recursive: true});
const documents = ['selected.glb', 'selected-data.gltf', 'selected-external.gltf', 'no-default.gltf'];
const buffers = new Map(['selected.bin', 'omitted.bin'].map(name => [name, fs.readFileSync(path.join(fixtures, name))]));
const hashes = Object.fromEntries([...documents, ...buffers.keys()].map(name => [name, crypto.createHash('sha256').update(fs.readFileSync(path.join(fixtures, name))).digest('hex')]));
const archiveSha256 = crypto.createHash('sha256').update(fs.readFileSync('target/quaternion-compatibility/gltf-validator.tgz')).digest('hex');
fs.writeFileSync(path.join(reports, 'identity.json'), JSON.stringify({version: validator.version(), gitHead: 'bcd52cc4ba5f333b2999a58f67cc05ddf28b4fb1', archiveSha256, supportedExtensions: validator.supportedExtensions(), hashes}, null, 2));
(async () => {
    for (const name of documents) {
        try {
            const report = await validator.validateBytes(new Uint8Array(fs.readFileSync(path.join(fixtures, name))), {
                uri: name,
                maxIssues: 0,
                externalResourceFunction: async uri => {
                    if (!buffers.has(uri)) throw new Error(`Unexpected resource: ${uri}`);
                    return new Uint8Array(buffers.get(uri));
                }
            });
            fs.writeFileSync(path.join(reports, `${name}.json`), JSON.stringify(report, null, 2));
            console.log(name, JSON.stringify(report.issues));
            if (report.issues.numErrors || report.issues.truncated) process.exitCode = 1;
        } catch (error) {
            fs.writeFileSync(path.join(reports, `${name}.unavailable.json`), JSON.stringify({outcome: 'unavailable', error: String(error)}, null, 2));
            process.exitCode = 1;
        }
    }
})().catch(error => { console.error(error); process.exitCode = 1; });
'@ | node -
```

Review all warnings and the exact artifact's supported extensions separately. A warning
about an unsupported extension is a coverage limit, not evidence that its semantics
were validated. A Validator failure/unavailable run remains open; zero errors alone
does not establish semantic equality, Baozi's policy, ownership or rendering behavior.

## Pinned references

- [Khronos glTF specification](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/specification/2.0/Specification.adoc): buffers/accessors, sparse overlays, normalization, scenes and cameras.
- [Extension registry at the same pin](https://github.com/KhronosGroup/glTF/blob/59ecbdba704206bb656965ab4e202881806066f1/extensions/README.md): Emissive Strength, Unlit, Texture Transform and Punctual Lights have distinct semantics and support boundaries.
- [Published gltf-rs 1.4.1 source](https://github.com/gltf-rs/gltf/tree/0f1e94f5e2307de7336be6d3a65c38c4d34aea08): the exact cached manifest confirms the enabled feature names and exact sibling `gltf-json` pin. Archive SHA-256 values are `e3ce1918195723ce6ac74e80542c5a96a40c2b26162c1957a5cd70799b8cacf7` and `e6176f9d60a7eab0a877e8e96548605dedbde9190a7ae1e80bbcc1c9af03ab14`, respectively; retain them in the main-owned lockfile.
- [Newer gltf-rs source](https://github.com/gltf-rs/gltf/tree/50d65229477fe5f785c2c90df21eb59c93ea2261): separately researched, never substituted into the comparator. Valid zero-accessor and optional-target differences correspond to [#346](https://github.com/gltf-rs/gltf/issues/346) and [#439](https://github.com/gltf-rs/gltf/issues/439); hazardous reader classes include [#478](https://github.com/gltf-rs/gltf/issues/478) and [#479](https://github.com/gltf-rs/gltf/issues/479).
- [Official Validator source matching the cached artifact](https://github.com/KhronosGroup/glTF-Validator/tree/bcd52cc4ba5f333b2999a58f67cc05ddf28b4fb1): an independent validation tool with version-specific coverage.

The main thread's dated standards/issues research supplies the current revision
qualification. Fixtures and tests here are original; no upstream fixture or test code
is copied. Follow [ADR 0027](../../../docs/adr/0027-gltf-backend-ownership-and-replacement-policy.md),
the [selected glTF profile](../../../docs/formats/gltf.md), and R18/R19 in the
[evolution plan](../../../docs/plans/2026-11-02-0923-refactor-pre-release-evolution-plan.md).
