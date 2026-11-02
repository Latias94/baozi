#include "probe.hpp"

#include <assimp/DefaultIOSystem.h>
#include <assimp/IOStream.hpp>
#include <assimp/IOSystem.hpp>
#include <assimp/config.h>
#include <assimp/material.h>
#include <assimp/postprocess.h>
#include <assimp/version.h>

#include <algorithm>
#include <charconv>
#include <chrono>
#include <cmath>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <locale>
#include <new>
#include <string_view>
#include <utility>

#include <assimp/MemoryIOWrapper.h>

static_assert(sizeof(ai_real) == BAOZI_ASSIMP_SCALAR_BYTES,
        "Assimp headers disagree with the configured geometry width");
static_assert(sizeof(double) == 8 && std::numeric_limits<double>::is_iec559,
        "The comparison output requires IEEE binary64");

namespace comparison {
namespace {

using Clock = std::chrono::steady_clock;
static_assert(Clock::is_steady, "Stage clocks must be monotonic");

bool wants_normals(Fields fields) { return fields != Fields::Position; }
bool wants_uv(Fields fields) { return fields == Fields::PositionNormalUv0; }

std::string format_extension(const std::filesystem::path &path) {
    auto extension = path.extension().u8string();
    std::transform(extension.begin(), extension.end(), extension.begin(), [](unsigned char c) {
        return static_cast<char>(c >= 'A' && c <= 'Z' ? c + ('a' - 'A') : c);
    });
    return extension;
}

const char *name(Stage value) {
    switch (value) {
    case Stage::Import: return "import";
    case Stage::Prepare: return "prepare";
    case Stage::Extract: return "extract";
    case Stage::Pipeline: return "pipeline";
    }
    return "unknown";
}

const char *name(IoMode value) { return value == IoMode::File ? "file" : "memory"; }
const char *name(NormalPolicy value) { return value == NormalPolicy::Missing ? "missing" : "preserve"; }
const char *name(Fields value) {
    switch (value) {
    case Fields::Position: return "position";
    case Fields::PositionNormal: return "position-normal";
    case Fields::PositionNormalUv0: return "position-normal-uv0";
    }
    return "unknown";
}

void require(bool condition, Status status, const char *message) {
    if (!condition) {
        throw Error(status, message);
    }
}

void require_finite(double value) {
    if (!std::isfinite(value)) {
        throw Error(Status::SemanticMismatch, "Selected geometry, transform or observation is nonfinite", true);
    }
}

std::uint64_t checked_add(std::uint64_t a, std::uint64_t b) {
    require(b <= std::numeric_limits<std::uint64_t>::max() - a,
            Status::PolicyRejected, "Output size overflow");
    return a + b;
}

std::uint64_t checked_bytes(std::uint64_t count, std::uint64_t width) {
    require(count <= std::numeric_limits<std::uint64_t>::max() / width,
            Status::PolicyRejected, "Output size overflow");
    return count * width;
}

class ComparisonIO final : public Assimp::IOSystem {
public:
    ComparisonIO(const Input &input, IoMode mode, bool &sidecar_attempted)
        : input_(input), memory_(mode == IoMode::Memory), sidecar_attempted_(sidecar_attempted) {}

    bool Exists(const char *file) const override {
        // Availability discovery alone does not establish an authored dependency.
        return memory_ ? is_primary(file) : filesystem_.Exists(file);
    }
    char getOsSeparator() const override { return filesystem_.getOsSeparator(); }
    Assimp::IOStream *Open(const char *file, const char *mode = "rb") override {
        const bool primary = is_primary(file);
        if (!primary) { sidecar_attempted_ = true; }
        if (memory_) {
            if (!primary) { return nullptr; }
            return new Assimp::MemoryIOStream(reinterpret_cast<const std::uint8_t *>(input_.memory.data()), input_.memory.size());
        }
        return filesystem_.Open(file, mode);
    }
    bool ComparePaths(const char *one, const char *other) const override {
        return memory_ ? std::strcmp(one, other) == 0 : filesystem_.ComparePaths(one, other);
    }
    void Close(Assimp::IOStream *stream) override { delete stream; }

private:
    bool is_primary(const char *file) const { return file && input_.path == file; }

    const Input &input_;
    bool memory_;
    bool &sidecar_attempted_;
    Assimp::DefaultIOSystem filesystem_;
};

Topology topology(const aiMesh &mesh) {
    require(mesh.mNumVertices != 0 && mesh.mVertices, Status::Unsupported, "Empty position stream");
    if (mesh.mPrimitiveTypes == aiPrimitiveType_POINT) {
        if (mesh.mNumFaces != 0) {
            require(mesh.mFaces != nullptr, Status::SemanticMismatch, "Missing point faces");
            for (unsigned int i = 0; i < mesh.mNumFaces; ++i) {
                require(mesh.mFaces[i].mNumIndices == 1 && mesh.mFaces[i].mIndices,
                        Status::Unsupported, "Only point lists and triangle lists are admitted");
            }
        }
        return Topology::Points;
    }
    require(mesh.mPrimitiveTypes == aiPrimitiveType_TRIANGLE,
            Status::Unsupported, "Only point lists and triangle lists are admitted; no triangulation is implied");
    require(mesh.mNumFaces != 0 && mesh.mFaces, Status::Unsupported, "Missing triangle faces");
    for (unsigned int i = 0; i < mesh.mNumFaces; ++i) {
        require(mesh.mFaces[i].mNumIndices == 3 && mesh.mFaces[i].mIndices,
                Status::Unsupported, "Only authored triangle lists are admitted");
    }
    return Topology::Triangles;
}

void require_profile(const aiScene &scene) {
    require(scene.mNumMeshes != 0 && scene.mMeshes && scene.mRootNode,
            Status::Unsupported, "Scene has no drawable geometry");
    require(scene.mNumAnimations == 0, Status::Unsupported, "Animation evaluation is outside the fixed geometry task");
    require(scene.mNumTextures == 0, Status::Unsupported, "Image tasks are not equivalent to this image-free comparison");
    for (unsigned int i = 0; i < scene.mNumMaterials; ++i) {
        const aiMaterial &material = *scene.mMaterials[i];
        for (unsigned int j = 0; j < material.mNumProperties; ++j) {
            require(std::strcmp(material.mProperties[j]->mKey.C_Str(), _AI_MATKEY_TEXTURE_BASE) != 0,
                    Status::Unsupported, "Texture/image tasks require a separately matched capability case");
        }
    }
    for (unsigned int i = 0; i < scene.mNumMeshes; ++i) {
        require(scene.mMeshes[i] != nullptr, Status::SemanticMismatch, "Null primitive mesh");
        const aiMesh &mesh = *scene.mMeshes[i];
        require(mesh.mNumBones == 0 && mesh.mNumAnimMeshes == 0,
                Status::Unsupported, "Skin and morph evaluation are outside the fixed geometry task");
    }
}

std::size_t tuple_count(const aiMesh &mesh, Topology kind) {
    if (kind == Topology::Points && mesh.mNumFaces == 0) {
        return mesh.mNumVertices;
    }
    const auto count = checked_bytes(mesh.mNumFaces, kind == Topology::Triangles ? 3 : 1);
    require(count <= std::numeric_limits<std::size_t>::max(), Status::PolicyRejected, "Tuple count overflow");
    return static_cast<std::size_t>(count);
}

template <class Visitor>
void visit_tuples(const aiMesh &mesh, Topology kind, Visitor &&visitor) {
    if (kind == Topology::Points && mesh.mNumFaces == 0) {
        for (unsigned int i = 0; i < mesh.mNumVertices; ++i) {
            visitor(i);
        }
        return;
    }
    for (unsigned int face = 0; face < mesh.mNumFaces; ++face) {
        for (unsigned int corner = 0; corner < mesh.mFaces[face].mNumIndices; ++corner) {
            const auto index = mesh.mFaces[face].mIndices[corner];
            require(index < mesh.mNumVertices, Status::SemanticMismatch, "Face index outside the position stream");
            visitor(index);
        }
    }
}

Position position(const aiVector3D &value) {
    return {static_cast<double>(value.x), static_cast<double>(value.y), static_cast<double>(value.z)};
}

Uv uv(const aiVector3D &value) {
    return {static_cast<double>(value.x), static_cast<double>(value.y)};
}

Matrix matrix(const aiMatrix4x4 &value) {
    return {value.a1, value.b1, value.c1, value.d1,
            value.a2, value.b2, value.c2, value.d2,
            value.a3, value.b3, value.c3, value.d3,
            value.a4, value.b4, value.c4, value.d4};
}

template <class Visitor>
void visit_placements(const aiScene &scene, const aiNode &node,
        const aiMatrix4x4 &parent, Visitor &visitor) {
    const aiMatrix4x4 world = parent * node.mTransformation;
    for (unsigned int i = 0; i < node.mNumMeshes; ++i) {
        require(node.mMeshes && node.mMeshes[i] < scene.mNumMeshes,
                Status::SemanticMismatch, "Placement references an invalid draw");
        visitor(static_cast<std::uint32_t>(node.mMeshes[i]), matrix(world));
    }
    for (unsigned int i = 0; i < node.mNumChildren; ++i) {
        require(node.mChildren && node.mChildren[i], Status::SemanticMismatch, "Null child node");
        visit_placements(scene, *node.mChildren[i], world, visitor);
    }
}

template <std::size_t N>
void extend(std::optional<Bounds<N>> &bounds, const std::array<double, N> &value) {
    for (double component : value) {
        require_finite(component);
    }
    if (!bounds) {
        bounds = Bounds<N>{value, value};
    } else {
        for (std::size_t i = 0; i < N; ++i) {
            bounds->min[i] = std::min(bounds->min[i], value[i]);
            bounds->max[i] = std::max(bounds->max[i], value[i]);
        }
    }
}

double triangle_area(const Position &a, const Position &b, const Position &c) {
    const Position ab{b[0] - a[0], b[1] - a[1], b[2] - a[2]};
    const Position ac{c[0] - a[0], c[1] - a[1], c[2] - a[2]};
    return 0.5 * std::hypot(ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2], ab[0] * ac[1] - ab[1] * ac[0]);
}

struct Accumulator {
    Observation value;
    std::array<Position, 3> triangle{};
    unsigned int corner = 0;
    bool normals_complete = true;
    bool uv_complete = true;

    void tuple(const Position &p, Topology kind, const std::optional<Position> &n,
            const std::optional<Uv> &t) {
        extend(value.bounds, p);
        if (n) { extend(value.normal_bounds, *n); }
        else { normals_complete = false; }
        if (t) { extend(value.uv0_bounds, *t); }
        else { uv_complete = false; }
        value.tuples = checked_add(value.tuples, 1);
        if (kind == Topology::Triangles) {
            triangle[corner++] = p;
            if (corner == 3) {
                value.area += triangle_area(triangle[0], triangle[1], triangle[2]);
                corner = 0;
            }
        }
    }

    void placement(const Matrix &world) {
        for (double component : world) { require_finite(component); }
        require(world[3] == 0 && world[7] == 0 && world[11] == 0 && world[15] == 1,
                Status::Unsupported, "Only affine placement transforms are admitted");
        value.placements = checked_add(value.placements, 1);
        value.translation_x_sum += world[12];
    }

    Observation finish() {
        require(corner == 0 && value.draws != 0 && value.placements != 0 && value.tuples != 0,
                Status::SemanticMismatch, "Incomplete or empty selected geometry");
        require_finite(value.area);
        require_finite(value.translation_x_sum);
        if (!normals_complete) { value.normal_bounds.reset(); }
        if (!uv_complete) { value.uv0_bounds.reset(); }
        return value;
    }
};

void require_fields(const aiMesh &mesh, Fields fields) {
    require(!wants_normals(fields) || mesh.mNormals, Status::Unsupported, "Requested normals are absent");
    require(!wants_uv(fields) || (mesh.mTextureCoords[0] && mesh.mNumUVComponents[0] == 2),
            Status::Unsupported, "Requested two-component UV0 is absent");
}

template <class Action>
std::uint64_t timed(Action &&action) {
    const auto start = Clock::now();
    action();
    const auto stop = Clock::now();
    return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(stop - start).count());
}

void quote(std::ostream &stream, std::string_view value) {
    constexpr char hex[] = "0123456789abcdef";
    stream.put('"');
    for (unsigned char byte : value) {
        switch (byte) {
        case '"': stream << "\\\""; break;
        case '\\': stream << "\\\\"; break;
        case '\b': stream << "\\b"; break;
        case '\f': stream << "\\f"; break;
        case '\n': stream << "\\n"; break;
        case '\r': stream << "\\r"; break;
        case '\t': stream << "\\t"; break;
        default:
            if (byte < 0x20 || byte >= 0x80) {
                stream << "\\u00" << hex[byte >> 4] << hex[byte & 15];
            } else {
                stream.put(static_cast<char>(byte));
            }
        }
    }
    stream.put('"');
}

void number(std::ostream &stream, double value) {
    require_finite(value);
    stream << value;
}

template <std::size_t N>
void write_bounds(std::ostream &stream, const std::optional<Bounds<N>> &bounds) {
    if (!bounds) { stream << "null"; return; }
    stream << '[';
    for (unsigned int side = 0; side < 2; ++side) {
        if (side != 0) { stream << ','; }
        stream << '[';
        const auto &components = side == 0 ? bounds->min : bounds->max;
        for (std::size_t i = 0; i < N; ++i) {
            if (i != 0) { stream << ','; }
            number(stream, components[i]);
        }
        stream << ']';
    }
    stream << ']';
}

} // namespace

Error::Error(Status value, const std::string &message, bool is_nonfinite)
    : std::runtime_error(message), status(value), nonfinite(is_nonfinite) {}

const char *name(Status value) {
    switch (value) {
    case Status::Success: return "success";
    case Status::PolicyRejected: return "policy_rejected";
    case Status::Unsupported: return "unsupported";
    case Status::SemanticMismatch: return "semantic_mismatch";
    case Status::Timeout: return "timeout";
    case Status::Crash: return "crash";
    }
    return "crash";
}

void check_runtime() {
    const auto library_width = (aiGetCompileFlags() & ASSIMP_CFLAGS_DOUBLE_SUPPORT) != 0 ? 8u : 4u;
    require(library_width == sizeof(ai_real), Status::SemanticMismatch,
            "Assimp header/library scalar widths disagree");
    require(aiGetVersionMajor() == 6 && aiGetVersionMinor() == 0 && aiGetVersionPatch() == 5,
            Status::Unsupported, "The comparison requires Assimp 6.0.5");
    if (std::strcmp(BAOZI_ASSIMP_SOURCE_IDENTITY, "git-verified") == 0) {
        require(aiGetVersionRevision() == 0x392a658fu, Status::SemanticMismatch,
                "Assimp runtime revision disagrees with the verified release source");
    }
}

void parse_request(int argc, const char *const *argv, Request &request) {
    require(argc >= 3, Status::PolicyRejected,
            "usage: PROBE ROOT RELATIVE_ASSET --comparison STAGE --io-mode MODE --fields FIELDS --normals POLICY --max-work-units N");
    request.root = std::filesystem::u8path(argv[1]);
    request.relative = std::filesystem::u8path(argv[2]);
    unsigned int seen = 0;
    for (int i = 3; i < argc; i += 2) {
        require(i + 1 < argc, Status::PolicyRejected, "Missing option value");
        const std::string_view option(argv[i]);
        const std::string_view value(argv[i + 1]);
        unsigned int bit = 0;
        if (option == "--comparison") {
            bit = 1;
            if (value == "import") { request.stage = Stage::Import; }
            else if (value == "prepare") { request.stage = Stage::Prepare; }
            else if (value == "extract") { request.stage = Stage::Extract; }
            else if (value == "pipeline") { request.stage = Stage::Pipeline; }
            else { throw Error(Status::Unsupported, "Unknown comparison endpoint"); }
        } else if (option == "--io-mode") {
            bit = 2;
            if (value == "file") { request.io_mode = IoMode::File; }
            else if (value == "memory") { request.io_mode = IoMode::Memory; }
            else { throw Error(Status::Unsupported, "Unknown IO mode"); }
        } else if (option == "--fields") {
            bit = 4;
            if (value == "position") { request.fields = Fields::Position; }
            else if (value == "position-normal") { request.fields = Fields::PositionNormal; }
            else if (value == "position-normal-uv0") { request.fields = Fields::PositionNormalUv0; }
            else { throw Error(Status::Unsupported, "Only the fixed geometry selections are supported; conversion/image tasks are unmatched"); }
        } else if (option == "--normals") {
            bit = 8;
            if (value == "preserve") { request.normals = NormalPolicy::Preserve; }
            else if (value == "missing") { request.normals = NormalPolicy::Missing; }
            else { throw Error(Status::Unsupported, "Unknown normal policy"); }
        } else if (option == "--max-work-units") {
            bit = 16;
            const auto parsed = std::from_chars(value.data(), value.data() + value.size(), request.max_work_units);
            require(parsed.ec == std::errc{} && parsed.ptr == value.data() + value.size(),
                    Status::PolicyRejected, "max-work-units must be an unsigned 64-bit integer");
        } else {
            throw Error(Status::Unsupported, "Unknown option: " + std::string(option));
        }
        require((seen & bit) == 0, Status::PolicyRejected, "Duplicate comparison option");
        seen |= bit;
    }
    require(seen == 31, Status::PolicyRejected, "All five comparison options are required");
}

Input load_input(const Request &request) {
    require(!request.relative.empty() && request.relative.is_relative() && !request.relative.has_root_path(),
            Status::PolicyRejected, "Asset must be relative to ROOT");
    for (const auto &part : request.relative) {
        require(part != "..", Status::PolicyRejected, "Parent traversal is not permitted in the root asset path");
    }
    std::error_code error;
    const auto root = std::filesystem::canonical(request.root, error);
    require(!error, Status::PolicyRejected, "Cannot resolve ROOT");
    const auto path = std::filesystem::canonical(root / request.relative, error);
    require(!error, Status::PolicyRejected, "Cannot resolve the root asset");
    const auto relative = path.lexically_relative(root);
    require(!relative.empty() && relative.is_relative() && *relative.begin() != "..",
            Status::PolicyRejected, "Root asset resolves outside ROOT");
    require(std::filesystem::is_regular_file(path, error) && !error, Status::PolicyRejected, "Asset is not a regular file");
    Input input;
    input.path = path.u8string();
    input.extension = format_extension(path);
    require(input.extension == ".obj" || input.extension == ".stl" || input.extension == ".ply"
                    || input.extension == ".gltf" || input.extension == ".glb",
            Status::Unsupported, "Only OBJ, STL, PLY and glTF are in this comparison");
    if (sizeof(ai_real) == 8) {
        require(input.extension == ".ply", Status::Unsupported, "The separately admitted F64 comparator supports PLY only");
    }
    input.gltf = input.extension == ".gltf" || input.extension == ".glb";
    if (request.io_mode == IoMode::Memory) {
        const auto size = std::filesystem::file_size(path, error);
        require(!error && size != 0 && size <= std::numeric_limits<std::size_t>::max()
                        && size <= static_cast<std::uintmax_t>(std::numeric_limits<std::streamsize>::max()),
                Status::PolicyRejected, "Cannot preload a complete nonempty single-file asset");
        input.memory.resize(static_cast<std::size_t>(size));
        std::ifstream stream(path, std::ios::binary);
        stream.read(input.memory.data(), static_cast<std::streamsize>(size));
        require(stream && stream.gcount() == static_cast<std::streamsize>(size),
                Status::PolicyRejected, "Failed to preload the entire asset");
    }
    return input;
}

Native::Native(const Request &request, const Input &input)
    : request_(request), input_(input), importer_(std::make_unique<Assimp::Importer>()) {
    importer_->SetPropertyBool(AI_CONFIG_GLOB_MEASURE_TIME, false);
    importer_->SetPropertyFloat(AI_CONFIG_PP_GSN_MAX_SMOOTHING_ANGLE, static_cast<ai_real>(175));
    importer_->SetIOHandler(new ComparisonIO(input, request.io_mode, sidecar_attempted_));
}

Native::~Native() = default;

void Native::import() {
    require(scene_ == nullptr && !importer_->GetScene(), Status::SemanticMismatch, "Import endpoint requires a fresh importer");
    scene_ = importer_->ReadFile(input_.path.c_str(), 0);
    if (sidecar_attempted_ && request_.io_mode == IoMode::Memory) {
        throw Error(Status::PolicyRejected, "Memory IO permits only exact primary-file opens; sidecar reads are denied without fallback");
    }
    // Release OBJ non-primary opens are explicit mtllib reads or their fallback.
    if (sidecar_attempted_ && input_.extension == ".obj") {
        throw Error(Status::Unsupported, "OBJ material-library reads are outside the matched single-file geometry task");
    }
    if (!scene_) {
        throw Error(Status::Unsupported, "Assimp import failed: " + std::string(importer_->GetErrorString()));
    }
}

const aiScene &Native::scene() const {
    require(scene_ != nullptr, Status::SemanticMismatch, "Native Scene is unavailable");
    return *scene_;
}

Assimp::IOSystem &Native::io() const {
    return *importer_->GetIOHandler();
}

void Native::free_scene() {
    importer_->FreeScene();
    scene_ = nullptr;
}

void Native::prepare() {
    require(!prepared_, Status::SemanticMismatch, "Preparation must run exactly once");
    require_profile(scene());
    bool point_only = true;
    for (unsigned int i = 0; i < scene().mNumMeshes; ++i) {
        point_only &= scene().mMeshes[i]->mPrimitiveTypes == aiPrimitiveType_POINT;
    }
    if (point_only) {
        require(request_.stage != Stage::Prepare, Status::Unsupported, "Point-only cases have no preparation endpoint");
        require(input_.extension == ".ply" && request_.fields == Fields::Position,
                Status::Unsupported, "The admitted point task is position-only PLY without preparation");
        prepared_ = true;
        return;
    }
    std::vector<aiMesh *> targets;
    targets.reserve(scene().mNumMeshes);
    bool missing_normals = false;
    bool has_triangles = false;
    for (unsigned int i = 0; i < scene().mNumMeshes; ++i) {
        aiMesh *mesh = scene().mMeshes[i];
        const auto kind = topology(*mesh);
        targets.push_back(mesh);
        has_triangles |= kind == Topology::Triangles;
        if (wants_normals(request_.fields) && !mesh->mNormals) {
            require(kind == Topology::Triangles && request_.normals == NormalPolicy::Missing,
                    Status::Unsupported, "Requested normals are missing and cannot be prepared under this policy/topology");
            missing_normals = true;
        }
    }
    require(request_.stage != Stage::Prepare || has_triangles, Status::Unsupported, "Point-only cases have no preparation endpoint");
    if (missing_normals) {
        scene_ = importer_->ApplyPostProcessing(aiProcess_GenSmoothNormals);
        require(scene_ != nullptr, Status::Unsupported, "Assimp missing-normal generation failed");
    }
    for (auto *target : targets) {
        require_fields(*target, request_.fields);
        if (input_.gltf && wants_uv(request_.fields)) {
            for (unsigned int i = 0; i < target->mNumVertices; ++i) {
                target->mTextureCoords[0][i].y = static_cast<ai_real>(1) - target->mTextureCoords[0][i].y;
            }
        }
    }
    prepared_ = true;
    // Local target storage and Assimp postprocess intermediates are gone before the stop clock.
}

Output Native::extract() const {
    require(prepared_, Status::SemanticMismatch, "Extraction requires completed preparation");
    require_profile(scene());
    Output output;
    output.draws.reserve(scene().mNumMeshes);
    for (unsigned int i = 0; i < scene().mNumMeshes; ++i) {
        const aiMesh &mesh = *scene().mMeshes[i];
        const auto kind = topology(mesh);
        require_fields(mesh, request_.fields);
        Draw draw;
        draw.topology = kind;
        const auto count = tuple_count(mesh, kind);
        draw.positions.reserve(count);
        if (wants_normals(request_.fields)) { draw.normals.reserve(count); }
        if (wants_uv(request_.fields)) { draw.uv0.reserve(count); }
        visit_tuples(mesh, kind, [&](unsigned int index) {
            draw.positions.push_back(position(mesh.mVertices[index]));
            if (wants_normals(request_.fields)) { draw.normals.push_back(position(mesh.mNormals[index])); }
            if (wants_uv(request_.fields)) { draw.uv0.push_back(uv(mesh.mTextureCoords[0][index])); }
        });
        output.draws.push_back(std::move(draw));
    }
    auto append = [&](std::uint32_t draw, const Matrix &world) { output.placements.push_back({draw, world}); };
    visit_placements(scene(), *scene().mRootNode, aiMatrix4x4{}, append);
    return output;
}

Observation Native::observe(bool prepared) const {
    require_profile(scene());
    Accumulator accumulator;
    accumulator.value.draws = scene().mNumMeshes;
    for (unsigned int i = 0; i < scene().mNumMeshes; ++i) {
        const aiMesh &mesh = *scene().mMeshes[i];
        const auto kind = topology(mesh);
        if (prepared) { require_fields(mesh, request_.fields); }
        if (wants_uv(request_.fields) && mesh.mTextureCoords[0]) {
            require(mesh.mNumUVComponents[0] == 2, Status::Unsupported, "UV0 must have two components");
        }
        visit_tuples(mesh, kind, [&](unsigned int index) {
            std::optional<Position> normal;
            std::optional<Uv> texcoord;
            if (wants_normals(request_.fields) && mesh.mNormals) { normal = position(mesh.mNormals[index]); }
            if (wants_uv(request_.fields) && mesh.mTextureCoords[0]) { texcoord = uv(mesh.mTextureCoords[0][index]); }
            accumulator.tuple(position(mesh.mVertices[index]), kind, normal, texcoord);
        });
    }
    auto observe = [&](std::uint32_t, const Matrix &world) { accumulator.placement(world); };
    visit_placements(scene(), *scene().mRootNode, aiMatrix4x4{}, observe);
    return accumulator.finish();
}

Observation observe_output(const Output &output, Fields fields) {
    Accumulator accumulator;
    accumulator.value.draws = output.draws.size();
    for (const Draw &draw : output.draws) {
        require(!draw.positions.empty() && (draw.topology != Topology::Triangles || draw.positions.size() % 3 == 0),
                Status::SemanticMismatch, "Detached draw has incomplete tuples");
        require(wants_normals(fields) ? draw.normals.size() == draw.positions.size() : draw.normals.empty(),
                Status::SemanticMismatch, "Detached normal selection/count mismatch");
        require(wants_uv(fields) ? draw.uv0.size() == draw.positions.size() : draw.uv0.empty(),
                Status::SemanticMismatch, "Detached UV0 selection/count mismatch");
        for (std::size_t i = 0; i < draw.positions.size(); ++i) {
            std::optional<Position> normal;
            std::optional<Uv> texcoord;
            if (wants_normals(fields)) { normal = draw.normals[i]; }
            if (wants_uv(fields)) { texcoord = draw.uv0[i]; }
            accumulator.tuple(draw.positions[i], draw.topology, normal, texcoord);
        }
    }
    for (const auto &placement : output.placements) {
        require(placement.draw < output.draws.size(), Status::SemanticMismatch, "Detached placement references an invalid draw");
        accumulator.placement(placement.world);
    }
    return accumulator.finish();
}

std::uint64_t Output::logical_bytes() const {
    auto bytes = checked_bytes(placements.size(), 16 * sizeof(double) + sizeof(std::uint32_t));
    for (const auto &draw : draws) {
        bytes = checked_add(bytes, checked_bytes(draw.positions.size(), sizeof(Position)));
        bytes = checked_add(bytes, checked_bytes(draw.normals.size(), sizeof(Position)));
        bytes = checked_add(bytes, checked_bytes(draw.uv0.size(), sizeof(Uv)));
    }
    return bytes;
}

std::uint64_t Output::capacity_bytes() const {
    auto bytes = checked_add(checked_bytes(draws.capacity(), sizeof(Draw)), checked_bytes(placements.capacity(), sizeof(Placement)));
    for (const auto &draw : draws) {
        bytes = checked_add(bytes, checked_bytes(draw.positions.capacity(), sizeof(Position)));
        bytes = checked_add(bytes, checked_bytes(draw.normals.capacity(), sizeof(Position)));
        bytes = checked_add(bytes, checked_bytes(draw.uv0.capacity(), sizeof(Uv)));
    }
    return bytes;
}

Report run(const Request &request) {
    const char *operation = "runtime";
    try {
        check_runtime();
        std::optional<Input> preload;
        if (request.io_mode == IoMode::Memory) {
            operation = "input";
            preload.emplace(load_input(request));
        }
        Report report;
        if (request.stage == Stage::Pipeline) {
            Output detached;
            report.stage_ns = timed([&] {
                std::optional<Input> file_input;
                if (!preload) {
                    operation = "input";
                    file_input.emplace(load_input(request));
                }
                operation = "setup";
                Native native(request, preload ? *preload : *file_input);
                operation = "import";
                native.import();
                operation = "prepare";
                native.prepare();
                operation = "extract";
                detached = native.extract();
                operation = "cleanup";
                native.free_scene();
            });
            report.output.emplace(std::move(detached));
            operation = "observe";
            report.observation = observe_output(*report.output, request.fields);
        } else {
            std::optional<Input> file_input;
            if (!preload) {
                operation = "input";
                file_input.emplace(load_input(request));
            }
            operation = "setup";
            Native native(request, preload ? *preload : *file_input);
            if (request.stage == Stage::Import) {
                operation = "import";
                report.stage_ns = timed([&] { native.import(); });
                operation = "observe";
                report.observation = native.observe(false);
            } else {
                operation = "import";
                native.import();
                operation = "prepare";
                if (request.stage == Stage::Prepare) {
                    report.stage_ns = timed([&] { native.prepare(); });
                    operation = "observe";
                    report.observation = native.observe(true);
                } else {
                    native.prepare();
                    operation = "extract";
                    report.stage_ns = timed([&] { report.output.emplace(native.extract()); });
                    operation = "observe";
                    report.observation = observe_output(*report.output, request.fields);
                }
            }
        }
        report.finite = true;
        return report;
    } catch (Error &error) {
        error.failure_stage = operation;
        throw;
    } catch (const std::bad_alloc &) {
        Error error(Status::PolicyRejected, "Native or selected-output allocation failed");
        error.failure_stage = operation;
        throw error;
    } catch (const std::exception &caught) {
        Error error(Status::Crash, caught.what());
        error.failure_stage = operation;
        throw error;
    } catch (...) {
        Error error(Status::Crash, "Unknown native exception");
        error.failure_stage = operation;
        throw error;
    }
}

void write_report(std::ostream &stream, const Request &request, const Report &report) {
    stream.imbue(std::locale::classic());
    stream << std::setprecision(std::numeric_limits<double>::max_digits10);
    stream << "{\"schema\":\"baozi-assimp-observation/v1\",\"engine\":\"assimp\",\"status\":";
    quote(stream, name(report.status));
    stream << ",\"stage\":"; quote(stream, name(request.stage));
    stream << ",\"io_mode\":"; quote(stream, name(request.io_mode));
    stream << ",\"fields\":"; quote(stream, name(request.fields));
    stream << ",\"normal_policy\":"; quote(stream, name(request.normals));
    stream << ",\"scalar_bytes\":" << sizeof(ai_real)
           << ",\"scalar_basis\":\"retained Assimp ai_real geometry; not destination width or source accessor provenance\""
           << ",\"computation_scalar_bytes\":" << sizeof(ai_real)
           << ",\"observation_scalar_bytes\":8,\"output_scalar_bytes\":8"
           << ",\"library_scalar_bytes\":" << ((aiGetCompileFlags() & ASSIMP_CFLAGS_DOUBLE_SUPPORT) != 0 ? 8 : 4)
           << ",\"assimp_version\":\"" << aiGetVersionMajor() << '.' << aiGetVersionMinor() << '.' << aiGetVersionPatch() << '"'
           << ",\"assimp_runtime_revision\":" << aiGetVersionRevision()
           << ",\"assimp_expected_revision\":\"392a658f9c271be965271f45e7521a1b80ea4392\",\"source_identity\":";
    quote(stream, BAOZI_ASSIMP_SOURCE_IDENTITY);
    stream << ",\"max_work_units\":" << request.max_work_units
           << ",\"max_work_units_enforced\":false,\"work_budget_note\":\"Baozi-specific request record; Assimp has no equivalent enforcement\""
           << ",\"uv0_convention\":";
    const auto extension = format_extension(request.relative);
    const bool gltf = extension == ".gltf" || extension == ".glb";
    quote(stream, request.stage == Stage::Import && gltf ? "assimp-native-one-minus-source-v" : "source");
    stream << ",\"io_backend\":";
    quote(stream, request.io_mode == IoMode::Memory ? "ReadFile + exact-primary preloaded IOSystem" : "ReadFile + observed Assimp default filesystem IOSystem");
    stream << ",\"io_policy\":";
    quote(stream, request.io_mode == IoMode::Memory ? "single preloaded file; exact primary identity only; non-primary Open denied; Exists is discovery"
                                                 : "root asset confined to ROOT; native filesystem forwarding; OBJ sidecar Open tasks unsupported; Exists is discovery");
    stream << ",\"finite\":";
    if (report.finite) { stream << (*report.finite ? "true" : "false"); }
    else { stream << "null"; }
    stream << ",\"stage_ns\":";
    if (report.stage_ns) { stream << *report.stage_ns; }
    else { stream << "null"; }
    stream << ",\"observation\":";
    if (report.observation) {
        const auto &value = *report.observation;
        stream << "{\"draws\":" << value.draws << ",\"placements\":" << value.placements
               << ",\"tuples\":" << value.tuples << ",\"bounds\":";
        write_bounds(stream, value.bounds);
        stream << ",\"area\":"; number(stream, value.area);
        stream << ",\"normal_bounds\":"; write_bounds(stream, value.normal_bounds);
        stream << ",\"uv0_bounds\":"; write_bounds(stream, value.uv0_bounds);
        stream << ",\"translation_x_sum\":"; number(stream, value.translation_x_sum);
        stream << '}';
    } else { stream << "null"; }
    stream << ",\"output\":";
    if (report.output) {
        stream << "{\"logical_bytes\":" << report.output->logical_bytes()
               << ",\"capacity_bytes\":" << report.output->capacity_bytes() << '}';
    } else { stream << "null"; }
    stream << ",\"error\":";
    if (report.message.empty()) { stream << "null"; }
    else { quote(stream, report.message); }
    stream << ",\"failure_stage\":";
    if (report.failure_stage.empty()) { stream << "null"; }
    else { quote(stream, report.failure_stage); }
    stream << "}\n";
}

} // namespace comparison

#ifndef BAOZI_ASSIMP_NO_MAIN
int main(int argc, char **argv) {
    comparison::Request request;
    comparison::Report report;
    try {
        comparison::parse_request(argc, argv, request);
        report = comparison::run(request);
    } catch (const comparison::Error &error) {
        report.status = error.status;
        report.message = error.what();
        report.failure_stage = error.failure_stage.empty() ? "arguments" : error.failure_stage;
        if (error.nonfinite) { report.finite = false; }
    } catch (const std::bad_alloc &) {
        report.status = comparison::Status::PolicyRejected;
        report.message = "Native or selected-output allocation failed";
        report.failure_stage = "arguments";
    } catch (const std::exception &error) {
        report.status = comparison::Status::Crash;
        report.message = error.what();
        report.failure_stage = "arguments";
    } catch (...) {
        report.status = comparison::Status::Crash;
        report.message = "Unknown native exception";
        report.failure_stage = "unknown";
    }
    if (report.status != comparison::Status::Success) {
        std::cerr << comparison::name(report.status) << ": " << report.message << '\n';
    }
    comparison::write_report(std::cout, request, report);
    return report.status == comparison::Status::Success ? 0 : 1;
}
#endif
