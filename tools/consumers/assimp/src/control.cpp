#include "probe.hpp"

#include <assimp/DefaultIOSystem.h>
#include <assimp/IOStream.hpp>
#include <assimp/ProgressHandler.hpp>
#include <assimp/config.h>
#include <assimp/postprocess.h>
#include <assimp/version.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <locale>
#include <memory>
#include <string_view>
#include <utility>

#include <assimp/MemoryIOWrapper.h>

namespace {
namespace fs = std::filesystem;

constexpr const char *release_revision = "392a658f9c271be965271f45e7521a1b80ea4392";
constexpr std::size_t large_obj_bytes = 33 * 1024 * 1024 + 512 * 1024;
constexpr std::size_t event_limit = 256;
constexpr std::size_t text_limit = 4096;
constexpr std::array<std::array<float, 3>, 3> triangle{{{0, 0, 0}, {1, 0, 0}, {0, 1, 0}}};
static_assert(sizeof(ai_real) == BAOZI_ASSIMP_SCALAR_BYTES, "Assimp header width mismatch");
static_assert(sizeof(float) == 4 && std::numeric_limits<float>::is_iec559, "Fixtures require IEEE binary32");

void require(bool condition, const char *message) {
    if (!condition) { throw std::runtime_error(message); }
}

std::string bounded(std::string_view value) { return std::string(value.substr(0, text_limit)); }

void little(std::string &bytes, std::uint32_t value, unsigned int width) {
    for (unsigned int i = 0; i < width; ++i) {
        bytes.push_back(static_cast<char>((value >> (8 * i)) & 255u));
    }
}

void binary32(std::string &bytes, float value) {
    std::uint32_t bits;
    std::memcpy(&bits, &value, sizeof(bits));
    little(bytes, bits, 4);
}

std::string base64(const std::string &bytes) {
    constexpr char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    std::string result;
    for (std::size_t i = 0; i < bytes.size(); i += 3) {
        const auto remaining = bytes.size() - i;
        std::uint32_t value = static_cast<unsigned char>(bytes[i]) << 16;
        if (remaining > 1) { value |= static_cast<unsigned char>(bytes[i + 1]) << 8; }
        if (remaining > 2) { value |= static_cast<unsigned char>(bytes[i + 2]); }
        result += alphabet[(value >> 18) & 63];
        result += alphabet[(value >> 12) & 63];
        result += remaining > 1 ? alphabet[(value >> 6) & 63] : '=';
        result += remaining > 2 ? alphabet[value & 63] : '=';
    }
    return result;
}

// A content fingerprint for these generated inputs, not a provenance digest.
std::uint64_t fingerprint(std::string_view bytes) {
    std::uint64_t value = UINT64_C(14695981039346656037);
    for (unsigned char byte : bytes) { value = (value ^ byte) * UINT64_C(1099511628211); }
    return value;
}

struct Fixture {
    const char *name;
    const char *format;
    bool authored_normals;
    std::uint64_t bytes = 0;
    std::uint64_t fnv1a64 = 0;
};

std::string fixture_bytes(std::size_t index) {
    const std::string obj = "# Original P6 triangle, no normals or material library\no p6\n"
                            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    if (index == 0) { return obj; }
    if (index == 1) {
        std::string bytes = obj;
        bytes.reserve(large_obj_bytes);
        const std::string comment = "#" + std::string(62, 'c') + "\n";
        while (bytes.size() + comment.size() <= large_obj_bytes) { bytes += comment; }
        const auto remaining = large_obj_bytes - bytes.size();
        if (remaining > 1) { bytes += "#" + std::string(remaining - 2, 'c') + "\n"; }
        else if (remaining == 1) { bytes += '\n'; }
        return bytes;
    }
    if (index == 2) {
        return "solid p6\nfacet normal 0 0 1\nouter loop\n"
               "vertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid p6\n";
    }
    if (index == 3) {
        std::string bytes = "Original P6 binary triangle";
        bytes.resize(80, '\0');
        little(bytes, 1, 4);
        for (float value : {0.f, 0.f, 1.f}) { binary32(bytes, value); }
        for (const auto &point : triangle) {
            for (float value : point) { binary32(bytes, value); }
        }
        little(bytes, 0, 2);
        return bytes;
    }
    if (index == 4 || index == 5) {
        std::string bytes = index == 4 ? "ply\nformat ascii 1.0\n" : "ply\nformat binary_little_endian 1.0\n";
        bytes += "comment Original P6 triangle\nelement vertex 3\nproperty float x\nproperty float y\n"
                 "property float z\nelement face 1\nproperty list uchar uint vertex_indices\nend_header\n";
        if (index == 4) { return bytes + "0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n"; }
        for (const auto &point : triangle) {
            for (float value : point) { binary32(bytes, value); }
        }
        little(bytes, 3, 1);
        for (unsigned int i = 0; i < 3; ++i) { little(bytes, i, 4); }
        return bytes;
    }
    require(index == 6 || index == 7, "Unknown fixed fixture");
    std::string payload;
    for (const auto &point : triangle) {
        for (float value : point) { binary32(payload, value); }
    }
    for (unsigned int i = 0; i < 3; ++i) { little(payload, i, 2); }
    std::string document = R"({"asset":{"version":"2.0","generator":"Original Baozi P6 control fixture v1"},
"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"mode":4}]}],
"buffers":[{"byteLength":42)";
    if (index == 6) { document += ",\"uri\":\"data:application/octet-stream;base64," + base64(payload) + "\""; }
    document += R"(}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36,"target":34962},
{"buffer":0,"byteOffset":36,"byteLength":6,"target":34963}],"accessors":[
{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}]})";
    if (index == 6) { return document + '\n'; }
    while (document.size() % 4 != 0) { document += ' '; }
    while (payload.size() % 4 != 0) { payload += '\0'; }
    std::string bytes;
    little(bytes, 0x46546c67, 4);
    little(bytes, 2, 4);
    little(bytes, static_cast<std::uint32_t>(28 + document.size() + payload.size()), 4);
    little(bytes, static_cast<std::uint32_t>(document.size()), 4);
    little(bytes, 0x4e4f534a, 4);
    bytes += document;
    little(bytes, static_cast<std::uint32_t>(payload.size()), 4);
    little(bytes, 0x004e4942, 4);
    return bytes + payload;
}

std::array<Fixture, 8> generate_fixtures(const fs::path &directory) {
    std::array<Fixture, 8> fixtures{{
            {"short.obj", "obj", false}, {"cache-boundaries.obj", "obj", false},
            {"ascii.stl", "stl-ascii", true}, {"binary.stl", "stl-binary", true},
            {"ascii.ply", "ply-ascii", false}, {"binary-le.ply", "ply-binary-le", false},
            {"embedded.gltf", "gltf2", false}, {"embedded.glb", "glb2", false}}};
    require(fs::create_directory(directory), "Fixture directory already exists");
    for (std::size_t i = 0; i < fixtures.size(); ++i) {
        const auto bytes = fixture_bytes(i);
        auto &fixture = fixtures[i];
        fixture.bytes = bytes.size();
        fixture.fnv1a64 = fingerprint(bytes);
        std::ofstream file(directory / fixture.name, std::ios::binary | std::ios::out);
        file.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
        file.close();
        require(static_cast<bool>(file), "Cannot write a generated fixture");
    }
    return fixtures;
}

struct IoCounts {
    std::uint64_t operations = 0, exists = 0, opens = 0, open_failures = 0;
    std::uint64_t nonprimary_opens = 0, reads = 0, bytes_read = 0, seeks = 0, streams_closed = 0;
};

class CountedStream final : public Assimp::IOStream {
public:
    CountedStream(std::unique_ptr<Assimp::IOStream> stream, IoCounts &counts)
        : stream_(std::move(stream)), counts_(counts) {}
    ~CountedStream() override { ++counts_.operations; ++counts_.streams_closed; }
    std::size_t Read(void *buffer, std::size_t size, std::size_t count) override {
        ++counts_.operations;
        ++counts_.reads;
        const auto result = stream_->Read(buffer, size, count);
        counts_.bytes_read += static_cast<std::uint64_t>(result) * size;
        return result;
    }
    std::size_t Write(const void *buffer, std::size_t size, std::size_t count) override {
        ++counts_.operations;
        return stream_->Write(buffer, size, count);
    }
    aiReturn Seek(std::size_t offset, aiOrigin origin) override {
        ++counts_.operations;
        ++counts_.seeks;
        return stream_->Seek(offset, origin);
    }
    std::size_t Tell() const override { ++counts_.operations; return stream_->Tell(); }
    std::size_t FileSize() const override { ++counts_.operations; return stream_->FileSize(); }
    void Flush() override { ++counts_.operations; stream_->Flush(); }

private:
    std::unique_ptr<Assimp::IOStream> stream_;
    IoCounts &counts_;
};

class PrimaryIO final : public Assimp::IOSystem {
public:
    PrimaryIO(const std::string &path, const std::vector<std::uint8_t> *memory, IoCounts &counts)
        : path_(path), memory_(memory), counts_(counts) {}
    bool Exists(const char *file) const override {
        ++counts_.operations;
        ++counts_.exists;
        return memory_ ? is_primary(file) : filesystem_.Exists(file);
    }
    char getOsSeparator() const override { return filesystem_.getOsSeparator(); }
    bool ComparePaths(const char *one, const char *other) const override {
        ++counts_.operations;
        return memory_ ? std::strcmp(one, other) == 0 : filesystem_.ComparePaths(one, other);
    }
    Assimp::IOStream *Open(const char *file, const char *mode = "rb") override {
        ++counts_.operations;
        ++counts_.opens;
        const bool primary = is_primary(file);
        if (!primary) { ++counts_.nonprimary_opens; }
        std::unique_ptr<Assimp::IOStream> stream;
        if (memory_) {
            if (primary) { stream.reset(new Assimp::MemoryIOStream(memory_->data(), memory_->size())); }
        } else {
            stream.reset(filesystem_.Open(file, mode));
        }
        if (!stream) { ++counts_.open_failures; return nullptr; }
        return new CountedStream(std::move(stream), counts_);
    }
    void Close(Assimp::IOStream *stream) override { delete stream; }

private:
    bool is_primary(const char *file) const { return file && path_ == file; }
    const std::string &path_;
    const std::vector<std::uint8_t> *memory_;
    IoCounts &counts_;
    Assimp::DefaultIOSystem filesystem_;
};

enum class Policy { I0, I1, I2, I3, P0, P1, P2, P3, U0, U1, U2 };
enum class Kind { Direct, FileRead, PostProcess };
const char *name(Policy policy) {
    constexpr const char *names[]{"I0", "I1", "I2", "I3", "P0", "P1", "P2", "P3", "U0", "U1", "U2"};
    return names[static_cast<unsigned int>(policy)];
}
const char *name(Kind kind) {
    constexpr const char *names[]{"direct_update", "file_read", "postprocess"};
    return names[static_cast<unsigned int>(kind)];
}
bool postprocess(Policy policy) { return policy >= Policy::P0 && policy <= Policy::P3; }
bool requests_false(Policy policy) {
    return policy == Policy::I1 || policy == Policy::I2 || policy == Policy::I3 ||
           policy == Policy::P1 || policy == Policy::P2 || policy == Policy::P3;
}

struct Event {
    std::uint64_t ordinal = 0, kind_ordinal = 0;
    Kind kind = Kind::Direct;
    int current = 0, total = 0;
    float value = 0;
    bool returned = true;
    IoCounts io;
};

struct Trace {
    std::array<Event, event_limit> events{};
    std::array<std::uint64_t, 3> kind_counts{};
    std::uint64_t count = 0, first_false = 0;
    IoCounts io, io_at_first_false;
    bool overflow = false, handler_destroyed = false;
    std::size_t stored() const { return static_cast<std::size_t>(std::min<std::uint64_t>(count, event_limit)); }
};

class Observer final : public Assimp::ProgressHandler {
public:
    Observer(Trace &trace, Policy policy, std::uint64_t final_file_ordinal = 0)
        : trace_(trace), policy_(policy), final_file_ordinal_(final_file_ordinal) {}
    ~Observer() override { trace_.handler_destroyed = true; }
    bool Update(float value = -1.f) noexcept override {
        const auto ordinal = ++trace_.count;
        const auto kind_ordinal = ++trace_.kind_counts[static_cast<unsigned int>(kind_)];
        const bool interior = current_ > 0 && current_ < total_;
        const bool hit =
                (policy_ == Policy::I1 && kind_ == Kind::FileRead && kind_ordinal == 1) ||
                (policy_ == Policy::I2 && kind_ == Kind::FileRead && interior) ||
                (policy_ == Policy::I3 && kind_ == Kind::FileRead && kind_ordinal == final_file_ordinal_) ||
                (policy_ == Policy::P1 && kind_ == Kind::PostProcess && kind_ordinal == 1) ||
                (policy_ == Policy::P2 && kind_ == Kind::PostProcess && interior) ||
                (policy_ == Policy::P3 && kind_ == Kind::PostProcess && total_ > 0 && current_ == total_);
        if (hit && trace_.first_false == 0) {
            trace_.first_false = ordinal;
            trace_.io_at_first_false = trace_.io;
        }
        const bool returned = trace_.first_false == 0;
        if (ordinal <= event_limit) {
            trace_.events[static_cast<std::size_t>(ordinal - 1)] =
                    Event{ordinal, kind_ordinal, kind_, current_, total_, value, returned, trace_.io};
        } else {
            trace_.overflow = true;
        }
        return returned;
    }
    void UpdateFileRead(int current, int total) noexcept override {
        kind_ = Kind::FileRead; current_ = current; total_ = total;
        Assimp::ProgressHandler::UpdateFileRead(current, total);
        kind_ = Kind::Direct;
    }
    void UpdatePostProcess(int current, int total) noexcept override {
        kind_ = Kind::PostProcess; current_ = current; total_ = total;
        Assimp::ProgressHandler::UpdatePostProcess(current, total);
        kind_ = Kind::Direct;
    }

private:
    Trace &trace_;
    Policy policy_;
    std::uint64_t final_file_ordinal_;
    Kind kind_ = Kind::Direct;
    int current_ = 0, total_ = 0;
};

struct Geometry {
    unsigned int meshes = 0, vertices = 0, faces = 0, indices = 0, normals = 0;
    bool root = false, triangle_matches = false, normals_match = false;
};

Geometry geometry(const aiScene *scene) {
    Geometry result;
    if (!scene) { return result; }
    result.root = scene->mRootNode != nullptr;
    result.meshes = scene->mNumMeshes;
    if (result.meshes != 1 || !scene->mMeshes || !scene->mMeshes[0]) { return result; }
    const auto &mesh = *scene->mMeshes[0];
    result.vertices = mesh.mNumVertices;
    result.faces = mesh.mNumFaces;
    result.normals = mesh.mNormals ? mesh.mNumVertices : 0;
    if (result.faces != 1 || !mesh.mFaces) { return result; }
    const auto &face = mesh.mFaces[0];
    result.indices = face.mNumIndices;
    if (result.vertices != 3 || result.indices != 3 || !face.mIndices || !mesh.mVertices ||
            mesh.mPrimitiveTypes != aiPrimitiveType_TRIANGLE) { return result; }
    result.triangle_matches = true;
    result.normals_match = mesh.mNormals != nullptr;
    for (unsigned int i = 0; i < 3; ++i) {
        const auto index = face.mIndices[i];
        if (index >= 3) { result.triangle_matches = false; result.normals_match = false; break; }
        const auto &p = mesh.mVertices[index];
        result.triangle_matches = result.triangle_matches &&
                p.x == triangle[i][0] && p.y == triangle[i][1] && p.z == triangle[i][2];
        if (mesh.mNormals) {
            const auto &n = mesh.mNormals[index];
            result.normals_match = result.normals_match && std::isfinite(n.x) && std::isfinite(n.y) &&
                    std::isfinite(n.z) && std::abs(n.x) < 1e-6f && std::abs(n.y) < 1e-6f &&
                    std::abs(n.z - 1) < 1e-6f;
        }
    }
    return result;
}

bool expected(const Geometry &value, bool normals) {
    return value.root && value.meshes == 1 && value.vertices == 3 && value.faces == 1 &&
           value.indices == 3 && value.triangle_matches && value.normals == (normals ? 3u : 0u) &&
           (!normals || value.normals_match);
}

bool same_geometry(const Geometry &a, const Geometry &b) {
    return a.meshes == b.meshes && a.vertices == b.vertices && a.faces == b.faces &&
           a.indices == b.indices && a.normals == b.normals && a.root == b.root &&
           a.triangle_matches == b.triangle_matches && a.normals_match == b.normals_match;
}

bool same_route(const Trace &a, const Trace &b) {
    if (a.overflow || b.overflow || a.count != b.count) { return false; }
    for (std::size_t i = 0; i < a.stored(); ++i) {
        const auto &x = a.events[i];
        const auto &y = b.events[i];
        if (x.kind != y.kind || x.current != y.current || x.total != y.total || x.value != y.value) { return false; }
    }
    return true;
}

bool expected_trace(const Trace &trace, const Fixture &fixture, bool with_postprocess) {
    if (trace.overflow || trace.count < 2 || trace.kind_counts[0] != 0) { return false; }
    const auto files = trace.kind_counts[1];
    if (files < 2 || files > trace.stored()) { return false; }
    const auto &entry = trace.events[0];
    const auto &final = trace.events[static_cast<std::size_t>(files - 1)];
    if (entry.kind != Kind::FileRead || entry.current != 0 ||
            entry.total != static_cast<int>(fixture.bytes) || final.kind != Kind::FileRead ||
            final.current != entry.total || final.total != entry.total) { return false; }
    unsigned int interior = 0;
    int previous_post = -1, post_total = 0;
    for (std::size_t i = 0; i < trace.stored(); ++i) {
        const auto &event = trace.events[i];
        if (!std::isfinite(event.value) || event.kind == Kind::Direct ||
                event.current < 0 || event.current > event.total) { return false; }
        const float fraction = event.total ? event.current / static_cast<float>(event.total) : 1.f;
        const float mapped = fraction * 0.5f + (event.kind == Kind::PostProcess ? 0.5f : 0.f);
        if (event.value != mapped) { return false; }
        if (event.kind == Kind::FileRead) {
            if (event.total != entry.total || i >= files) { return false; }
            if (event.current > 0 && event.current < event.total) { ++interior; }
        } else {
            if (!with_postprocess || event.total <= 0 || event.current != previous_post + 1 ||
                    (previous_post >= 0 && post_total != event.total)) { return false; }
            previous_post = event.current;
            post_total = event.total;
        }
    }
    if (fixture.bytes == large_obj_bytes) {
        if (interior < 2) { return false; }
    } else if (interior != 0) { return false; }
    if (std::strcmp(fixture.format, "obj") == 0) {
        if (files < 3) { return false; }
    } else if (files != 2) { return false; }
    return with_postprocess ? previous_post == post_total && post_total > 0 &&
                    trace.kind_counts[2] == static_cast<std::uint64_t>(post_total) + 1
                            : trace.kind_counts[2] == 0;
}

struct Record {
    Policy policy = Policy::I0;
    const Fixture *fixture = nullptr;
    bool memory = false, import_called = false, postprocess_called = false;
    bool import_scene = false, scene = false, library_exception = false, io_return_available = false;
    Trace trace;
    IoCounts io_at_return;
    Geometry imported, final;
    std::string import_error, error, exception;
    std::uint64_t target_file_ordinal = 0;
    bool geometry_ok = false, import_geometry_ok = false, baseline_ok = false;
    bool matches_geometry = false, matches_route = false, trace_ok = false, trigger_ok = false, passed = false;
    const char *result = "unrelated_failure";
};

Record run_sequence(const fs::path &directory, const Fixture &fixture, bool memory,
        Policy policy, const Record *baseline) {
    Record record;
    record.policy = policy; record.fixture = &fixture; record.memory = memory;
    record.target_file_ordinal = policy == Policy::I3 && baseline && baseline->passed ?
            baseline->trace.kind_counts[1] : 0;
    const auto path = (directory / fixture.name).u8string();
    // These buffers and traces outlive the importer and its owned handlers/streams.
    std::vector<std::uint8_t> bytes;
    try {
        if (memory) {
            bytes.resize(static_cast<std::size_t>(fixture.bytes));
            std::ifstream file(fs::u8path(path), std::ios::binary);
            file.read(reinterpret_cast<char *>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
            require(static_cast<bool>(file) && file.peek() == std::char_traits<char>::eof(), "Cannot preload exact fixture bytes");
            require(fingerprint(std::string_view(reinterpret_cast<const char *>(bytes.data()), bytes.size())) == fixture.fnv1a64,
                    "Generated fixture changed before memory preload");
        }
        Assimp::Importer importer;
        importer.SetIOHandler(new PrimaryIO(path, memory ? &bytes : nullptr, record.trace.io));
        importer.SetProgressHandler(new Observer(record.trace, policy, record.target_file_ordinal));
        importer.SetPropertyFloat(AI_CONFIG_PP_GSN_MAX_SMOOTHING_ANGLE, 175.f);
        record.import_called = true;
        const aiScene *scene = importer.ReadFile(path, 0);
        record.import_scene = scene != nullptr;
        record.imported = geometry(scene);
        record.import_error = bounded(importer.GetErrorString());
        record.library_exception = static_cast<bool>(importer.GetException());
        if (postprocess(policy)) {
            record.postprocess_called = true;
            scene = importer.ApplyPostProcessing(aiProcess_GenSmoothNormals);
        }
        record.scene = scene != nullptr;
        record.final = geometry(scene);
        record.error = bounded(importer.GetErrorString());
        record.library_exception = record.library_exception || static_cast<bool>(importer.GetException());
        record.io_at_return = record.trace.io;
        record.io_return_available = true;
    } catch (const std::exception &error) {
        record.exception = bounded(error.what());
        record.scene = false;
    } catch (...) {
        record.exception = "Non-standard exception outside the progress observer";
        record.scene = false;
    }
    record.geometry_ok = record.scene && expected(record.final, fixture.authored_normals || postprocess(policy));
    record.import_geometry_ok = record.import_scene && expected(record.imported, fixture.authored_normals);
    record.baseline_ok = !baseline || baseline->passed;
    record.matches_geometry = !baseline || same_geometry(record.final, baseline->final);
    record.matches_route = !baseline || same_route(record.trace, baseline->trace);
    record.trace_ok = expected_trace(record.trace, fixture, postprocess(policy));
    const bool expect_trigger = requests_false(policy) && (policy != Policy::I2 || fixture.bytes == large_obj_bytes);
    record.trigger_ok = (record.trace.first_false != 0) == expect_trigger;
    const bool operation_ok = record.geometry_ok && record.import_geometry_ok && record.exception.empty() &&
            !record.library_exception && record.trace.io.open_failures == 0 && record.trace.io.nonprimary_opens == 0;
    if (operation_ok && !record.trace.overflow) {
        record.result = record.trace.first_false ? "false_return_ignored" :
                requests_false(policy) ? "trigger_not_reached" : "accepted";
    }
    // A null scene, even after false, supplies no cancellation attribution here.
    record.passed = operation_ok && record.baseline_ok && record.matches_geometry && record.matches_route &&
            record.trace_ok && record.trigger_ok && record.trace.handler_destroyed;
    return record;
}

void quote(std::ostream &out, std::string_view text) {
    constexpr char hex[] = "0123456789abcdef";
    out << '"';
    for (unsigned char byte : text) {
        if (byte == '"') { out << "\\\""; }
        else if (byte == '\\') { out << "\\\\"; }
        else if (byte < 0x20 || byte >= 0x80) { out << "\\u00" << hex[byte >> 4] << hex[byte & 15]; }
        else { out.put(static_cast<char>(byte)); }
    }
    out << '"';
}

void write_io(std::ostream &out, const IoCounts &io) {
    out << "{\"operations\":" << io.operations << ",\"exists\":" << io.exists << ",\"opens\":" << io.opens
        << ",\"open_failures\":" << io.open_failures << ",\"nonprimary_opens\":" << io.nonprimary_opens
        << ",\"reads\":" << io.reads << ",\"bytes_read\":" << io.bytes_read << ",\"seeks\":" << io.seeks
        << ",\"streams_closed\":" << io.streams_closed << '}';
}

void write_geometry(std::ostream &out, const Geometry &value, bool present) {
    if (!present) { out << "null"; return; }
    out << "{\"meshes\":" << value.meshes << ",\"vertices\":" << value.vertices << ",\"faces\":" << value.faces
        << ",\"indices\":" << value.indices << ",\"normal_vectors\":" << value.normals << ",\"root\":" << value.root
        << ",\"analytic_triangle_matches\":" << value.triangle_matches << ",\"positive_z_normals_match\":" << value.normals_match << '}';
}

void write_events(std::ostream &out, const Trace &trace) {
    out << '[';
    for (std::size_t i = 0; i < trace.stored(); ++i) {
        if (i) { out << ','; }
        const auto &event = trace.events[i];
        out << "{\"event_ordinal\":" << event.ordinal << ",\"kind_ordinal\":" << event.kind_ordinal << ",\"kind\":";
        quote(out, name(event.kind));
        out << ",\"current\":";
        if (event.kind == Kind::Direct) { out << "null"; } else { out << event.current; }
        out << ",\"total\":";
        if (event.kind == Kind::Direct) { out << "null"; } else { out << event.total; }
        out << ",\"value\":";
        if (std::isfinite(event.value)) { out << event.value; } else { out << "null"; }
        out << ",\"value_finite\":" << std::isfinite(event.value) << ",\"returned\":" << event.returned << ",\"io\":";
        write_io(out, event.io);
        out << '}';
    }
    out << ']';
}

void write_record(std::ostream &out, const Record &record, unsigned int ordinal) {
    out << "{\"record_ordinal\":" << ordinal << ",\"engine\":\"assimp\",\"io\":";
    quote(out, record.memory ? "memory" : "file");
    out << ",\"fixture\":"; quote(out, record.fixture->name);
    out << ",\"trigger\":{\"case\":"; quote(out, name(record.policy));
    out << ",\"requested\":" << requests_false(record.policy) << ",\"trigger_reached\":" << (record.trace.first_false != 0)
        << ",\"baseline_file_final_ordinal\":";
    if (record.policy == Policy::I3 && record.target_file_ordinal) { out << record.target_file_ordinal; }
    else { out << "null"; }
    out << ",\"first_false_event_ordinal\":";
    if (record.trace.first_false) { out << record.trace.first_false; } else { out << "null"; }
    out << "},\"scene_kind\":"; quote(out, !record.exception.empty() ? "exception" : record.scene ? "native_scene" : "null");
    out << ",\"result\":"; quote(out, record.result);
    out << ",\"error\":"; quote(out, record.error);
    out << ",\"exception\":"; quote(out, record.exception);
    out << ",\"library_exception_present\":" << record.library_exception
        << ",\"null_after_false_unattributed\":" << (!record.scene && record.trace.first_false != 0)
        << ",\"import_called\":" << record.import_called << ",\"postprocess_called\":" << record.postprocess_called
        << ",\"import_scene_present\":" << record.import_scene << ",\"import_error\":";
    quote(out, record.import_error);
    out << ",\"import_geometry\":"; write_geometry(out, record.imported, record.import_scene);
    out << ",\"geometry\":"; write_geometry(out, record.final, record.scene);
    out << ",\"checks\":{\"geometry_expected\":" << record.geometry_ok << ",\"import_geometry_expected\":" << record.import_geometry_ok
        << ",\"baseline_qualified\":" << record.baseline_ok << ",\"geometry_matches_baseline\":" << record.matches_geometry
        << ",\"callback_route_matches_baseline\":" << record.matches_route << ",\"trace_shape_expected\":" << record.trace_ok
        << ",\"trigger_presence_expected\":" << record.trigger_ok << ",\"trace_complete\":" << !record.trace.overflow
        << ",\"importer_destroyed_handler\":" << record.trace.handler_destroyed << ",\"passed\":" << record.passed << '}'
        << ",\"callback_count\":" << record.trace.count << ",\"file_callback_count\":" << record.trace.kind_counts[1]
        << ",\"postprocess_callback_count\":" << record.trace.kind_counts[2] << ",\"callbacks_after_false\":";
    if (record.trace.first_false) { out << record.trace.count - record.trace.first_false; } else { out << "null"; }
    out << ",\"io_at_first_false\":";
    if (record.trace.first_false) { write_io(out, record.trace.io_at_first_false); } else { out << "null"; }
    out << ",\"io_at_return\":";
    if (record.io_return_available) { write_io(out, record.io_at_return); } else { out << "null"; }
    out << ",\"io_after_importer_destruction\":"; write_io(out, record.trace.io);
    out << ",\"io_operations_after_false_before_return\":";
    if (record.trace.first_false && record.io_return_available) {
        out << record.io_at_return.operations - record.trace.io_at_first_false.operations;
    } else { out << "null"; }
    out << ",\"bytes_read_after_false_before_return\":";
    if (record.trace.first_false && record.io_return_available) {
        out << record.io_at_return.bytes_read - record.trace.io_at_first_false.bytes_read;
    } else { out << "null"; }
    out << ",\"events\":"; write_events(out, record.trace);
    out << '}';
}

bool write_adapter(std::ostream &out, Policy policy, unsigned int ordinal) {
    Trace trace;
    {
        Observer observer(trace, policy);
        Assimp::ProgressHandler &public_interface = observer;
        if (policy == Policy::U0) { public_interface.Update(); }
        else if (policy == Policy::U1) { public_interface.UpdateFileRead(0, 0); }
        else { public_interface.UpdatePostProcess(0, 0); }
    }
    const float expected_value = policy == Policy::U0 ? -1.f : policy == Policy::U1 ? 0.5f : 1.f;
    const bool passed = trace.count == 1 && !trace.overflow && trace.events[0].returned && trace.events[0].value == expected_value;
    out << "{\"record_ordinal\":" << ordinal << ",\"engine\":\"assimp\",\"io\":\"none\",\"fixture\":null,\"trigger\":{\"case\":";
    quote(out, name(policy));
    out << ",\"requested\":false,\"trigger_reached\":false,\"first_false_event_ordinal\":null},"
           "\"scene_kind\":\"no_import\",\"result\":";
    quote(out, passed ? "adapter_observed" : "unrelated_failure");
    out << ",\"error\":\"\",\"geometry\":null,\"checks\":{\"expected_value\":" << expected_value
        << ",\"passed\":" << passed << "},\"events\":";
    write_events(out, trace);
    out << '}';
    return passed;
}

void write_identity(std::ostream &out) {
    out << "\"identity\":{\"expected_source_revision\":"; quote(out, release_revision);
    out << ",\"configured_source_identity\":"; quote(out, BAOZI_ASSIMP_SOURCE_IDENTITY);
    out << ",\"runtime_version\":[" << aiGetVersionMajor() << ',' << aiGetVersionMinor() << ',' << aiGetVersionPatch()
        << "],\"runtime_revision\":" << aiGetVersionRevision() << ",\"runtime_compile_flags\":" << aiGetCompileFlags()
        << ",\"scalar_bytes\":" << sizeof(ai_real) << ",\"pointer_bytes\":" << sizeof(void *)
        << ",\"cxx_standard\":201703,\"compiler\":";
#if defined(__clang__)
    quote(out, "clang " __clang_version__);
#elif defined(_MSC_VER)
    quote(out, "msvc"); out << ",\"msvc_full_version\":" << _MSC_FULL_VER;
#elif defined(__GNUC__)
    quote(out, "gcc " __VERSION__);
#else
    quote(out, "unknown");
#endif
#if defined(_WIN32)
    out << ",\"platform\":\"windows\"";
#elif defined(__linux__)
    out << ",\"platform\":\"linux\"";
#else
    out << ",\"platform\":\"other\"";
#endif
#ifdef NDEBUG
    out << ",\"ndebug\":true";
#else
    out << ",\"ndebug\":false";
#endif
    out << ",\"external_source_build_binary_receipts_required\":true}";
}

int execute(int argc, char **argv) {
    require(argc == 3, "usage: baozi-assimp-control ROOT REPORT.JSON (new ROOT; report directly inside ROOT)");
    const auto root = fs::absolute(fs::u8path(argv[1])).lexically_normal();
    const auto report_argument = fs::u8path(argv[2]);
    const auto report = (report_argument.has_parent_path() ? fs::absolute(report_argument) : root / report_argument).lexically_normal();
    require(root.has_filename() && report.has_filename() && report.parent_path() == root && report.filename() != "fixtures",
            "Report must be a file directly inside the new task root, separate from fixtures");
    require(root.u8string().size() <= text_limit && report.u8string().size() <= text_limit, "Artifact path exceeds report text bound");
    require(fs::is_directory(root.parent_path()), "Task root parent must already exist");
    require(!fs::exists(fs::symlink_status(root)) && !fs::exists(fs::symlink_status(report)), "Refusing existing artifacts");
    comparison::check_runtime();
    require(sizeof(ai_real) == 4, "The fixed eight-fixture control matrix requires the f32 build");
    {
        Assimp::Importer importer;
        for (const char *extension : {".obj", ".stl", ".ply", ".gltf", ".glb"}) {
            require(importer.IsExtensionSupported(extension), "Required reader is not enabled");
        }
    }
    require(fs::create_directory(root), "Task root already exists; use a fresh root");
    const auto directory = root / "fixtures";
    const auto fixtures = generate_fixtures(directory);
    std::ofstream out(report, std::ios::binary | std::ios::out);
    require(static_cast<bool>(out), "Cannot create control report");
    out.imbue(std::locale::classic());
    out << std::boolalpha << std::setprecision(std::numeric_limits<float>::max_digits10)
        << "{\"schema\":\"baozi-assimp-control/v1\",\"engine\":\"assimp\",\"timing_collected\":false,"
           "\"fixture_generation\":\"original-p6-triangle/v1\",\"event_limit_per_record\":" << event_limit
        << ",\"text_limit_bytes\":" << text_limit << ",\"task_root\":";
    quote(out, root.u8string());
    out << ','; write_identity(out);
    out << ",\"fixtures\":[";
    for (std::size_t i = 0; i < fixtures.size(); ++i) {
        if (i) { out << ','; }
        const auto &fixture = fixtures[i];
        out << "{\"file\":"; quote(out, (fs::path("fixtures") / fixture.name).generic_u8string());
        out << ",\"format\":"; quote(out, fixture.format);
        out << ",\"bytes\":" << fixture.bytes << ",\"fnv1a64_decimal\":\"" << fixture.fnv1a64
            << "\",\"expected_meshes\":1,\"expected_vertices\":3,\"expected_faces\":1,\"authored_normals\":" << fixture.authored_normals << '}';
    }
    out << "],\"records\":[\n";
    unsigned int records = 0, sequences = 0, readfile_calls = 0, postprocess_calls = 0, failures = 0;
    unsigned int ignored = 0, missed = 0, unrelated = 0;
    auto emit = [&](const Record &record) {
        if (records) { out << ",\n"; }
        write_record(out, record, ++records);
        ++sequences;
        readfile_calls += record.import_called ? 1u : 0u;
        postprocess_calls += record.postprocess_called ? 1u : 0u;
        failures += record.passed ? 0u : 1u;
        ignored += std::strcmp(record.result, "false_return_ignored") == 0 ? 1u : 0u;
        missed += std::strcmp(record.result, "trigger_not_reached") == 0 ? 1u : 0u;
        unrelated += std::strcmp(record.result, "unrelated_failure") == 0 ? 1u : 0u;
    };
    for (bool memory : {false, true}) {
        for (const auto &fixture : fixtures) {
            const auto baseline = run_sequence(directory, fixture, memory, Policy::I0, nullptr);
            emit(baseline);
            for (Policy policy : {Policy::I1, Policy::I2, Policy::I3}) {
                emit(run_sequence(directory, fixture, memory, policy, &baseline));
            }
        }
        const auto baseline = run_sequence(directory, fixtures[0], memory, Policy::P0, nullptr);
        emit(baseline);
        for (Policy policy : {Policy::P1, Policy::P2, Policy::P3}) {
            emit(run_sequence(directory, fixtures[0], memory, policy, &baseline));
        }
    }
    for (Policy policy : {Policy::U0, Policy::U1, Policy::U2}) {
        out << ",\n";
        failures += write_adapter(out, policy, ++records) ? 0u : 1u;
    }
    const bool complete = records == 75 && sequences == 72 && readfile_calls == 72 && postprocess_calls == 8;
    const bool passed = complete && failures == 0;
    out << "\n],\"summary\":{\"expected_import_sequences\":72,\"import_sequences\":" << sequences
        << ",\"readfile_calls\":" << readfile_calls << ",\"postprocess_calls\":" << postprocess_calls
        << ",\"adapter_calls\":3,\"records\":" << records << ",\"failed_records\":" << failures
        << ",\"false_return_ignored\":" << ignored << ",\"trigger_not_reached\":" << missed
        << ",\"unrelated_failure\":" << unrelated << ",\"cancelled_after_false\":0,\"matrix_complete\":" << complete
        << ",\"checks_passed\":" << passed << "}}\n";
    out.close();
    require(static_cast<bool>(out), "Cannot finish control report; retained file may be incomplete");
    std::cout << (passed ? "Control observations complete: " : "Control observations require investigation: ") << report.u8string() << '\n';
    return passed ? 0 : 1;
}

} // namespace

int main(int argc, char **argv) {
    try { return execute(argc, argv); }
    catch (const std::exception &error) { std::cerr << "Assimp control setup/report failure: " << bounded(error.what()) << '\n'; }
    catch (...) { std::cerr << "Assimp control setup/report failure: non-standard exception\n"; }
    return 2;
}
