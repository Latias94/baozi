#include "../src/probe.hpp"

#include <assimp/IOStream.hpp>
#include <assimp/IOSystem.hpp>
#include <assimp/version.h>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstring>
#include <fstream>
#include <iostream>
#include <limits>
#include <sstream>
#include <string_view>
#include <utility>

namespace {

using namespace comparison;

void expect(bool condition, const std::string &message) {
    if (!condition) { throw std::runtime_error(message); }
}

void close(double actual, double expected, const char *message, double tolerance = 1e-12) {
    expect(std::isfinite(actual) && std::abs(actual - expected) <= tolerance, message);
}

template <std::size_t N>
void equal(const std::array<double, N> &actual, const std::array<double, N> &expected, const char *message) {
    for (std::size_t i = 0; i < N; ++i) { close(actual[i], expected[i], message); }
}

template <class Action>
void rejects(Status expected, Action &&action, const char *failure_stage = nullptr) {
    try {
        action();
    } catch (const Error &error) {
        expect(error.status == expected, "Wrong typed failure: " + std::string(error.what()));
        if (failure_stage) { expect(error.failure_stage == failure_stage, "Failure identifies its actual operation"); }
        return;
    }
    throw std::runtime_error("Expected a typed rejection");
}

Request request(const std::filesystem::path &root, const char *file, Fields fields = Fields::Position,
        NormalPolicy normals = NormalPolicy::Preserve, Stage stage = Stage::Pipeline, IoMode io = IoMode::Memory) {
    Request result;
    result.root = root;
    result.relative = file;
    result.fields = fields;
    result.normals = normals;
    result.stage = stage;
    result.io_mode = io;
    // A zero Baozi budget must not turn into a fictitious Assimp enforcement limit.
    result.max_work_units = 0;
    return result;
}

Matrix identity(double x = 0, double y = 0) {
    return {1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, x, y, 0, 1};
}

class Fixtures {
public:
    Fixtures() {
        const auto stamp = std::chrono::steady_clock::now().time_since_epoch().count();
        const auto parent = std::filesystem::temp_directory_path();
        for (unsigned int attempt = 0; attempt < 100; ++attempt) {
            path = parent / ("baozi-assimp-semantic-" + std::to_string(stamp) + "-" + std::to_string(attempt));
            std::error_code error;
            if (std::filesystem::create_directory(path, error)) { return; }
            expect(!error, "Cannot create isolated semantic fixture directory");
        }
        throw std::runtime_error("Cannot obtain a unique semantic fixture directory");
    }
    ~Fixtures() {
        std::error_code error;
        std::filesystem::remove_all(path, error);
    }
    Fixtures(const Fixtures &) = delete;
    Fixtures &operator=(const Fixtures &) = delete;

    void write(const char *name, const std::string &data) const {
        std::ofstream stream(path / name, std::ios::binary);
        stream.write(data.data(), static_cast<std::streamsize>(data.size()));
        expect(static_cast<bool>(stream), "Cannot write semantic fixture");
    }

    std::filesystem::path path;
};

void little(std::string &bytes, std::uint64_t value, unsigned int width) {
    for (unsigned int i = 0; i < width; ++i) { bytes.push_back(static_cast<char>((value >> (8 * i)) & 255)); }
}

void binary64(std::string &bytes, double value) {
    std::uint64_t bits;
    std::memcpy(&bits, &value, sizeof(bits));
    little(bytes, bits, 8);
}

void binary32(std::string &bytes, float value) {
    std::uint32_t bits;
    std::memcpy(&bits, &value, sizeof(bits));
    little(bytes, bits, 4);
}

std::string neighbor_ply(double next) {
    std::string bytes = "ply\nformat binary_little_endian 1.0\nelement vertex 3\n"
                        "property double x\nproperty double y\nproperty double z\n"
                        "element face 1\nproperty list uchar uint vertex_indices\nend_header\n";
    const std::array<Position, 3> points{{{1, -0.0, 0}, {next, 0, 0}, {1, 1, 0}}};
    for (const auto &point : points) {
        for (double component : point) { binary64(bytes, component); }
    }
    little(bytes, 3, 1);
    for (unsigned int i = 0; i < 3; ++i) { little(bytes, i, 4); }
    return bytes;
}

// An independent nested transform/UV fixture, with an embedded buffer and no sidecars.
std::string nested_glb() {
    std::string payload;
    const std::array<float, 24> values{{0, 0, 0, 1, 0, 0, 0, 1, 0,
            0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0.25f, 1, 0.5f, 0.25f, 0.75f}};
    for (float value : values) { binary32(payload, value); }
    for (unsigned int i = 0; i < 3; ++i) { little(payload, i, 2); }
    std::string document = R"({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
"nodes":[{"matrix":[0,1,0,0,-1,0,0,0,0,0,1,0,3,0,0,1],"children":[1,2]},
{"mesh":0,"translation":[2,0,0],"scale":[2,3,1]},{"mesh":0,"translation":[-1,4,0]}],
"meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"indices":3}]}],
"buffers":[{"byteLength":102}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},
{"buffer":0,"byteOffset":36,"byteLength":36},{"buffer":0,"byteOffset":72,"byteLength":24},
{"buffer":0,"byteOffset":96,"byteLength":6}],"accessors":[
{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
{"bufferView":1,"componentType":5126,"count":3,"type":"VEC3"},
{"bufferView":2,"componentType":5126,"count":3,"type":"VEC2"},
{"bufferView":3,"componentType":5123,"count":3,"type":"SCALAR"}]})";
    while (document.size() % 4 != 0) { document.push_back(' '); }
    while (payload.size() % 4 != 0) { payload.push_back('\0'); }
    std::string bytes;
    little(bytes, 0x46546c67, 4);
    little(bytes, 2, 4);
    little(bytes, 28 + document.size() + payload.size(), 4);
    little(bytes, document.size(), 4);
    little(bytes, 0x4e4f534a, 4);
    bytes += document;
    little(bytes, payload.size(), 4);
    little(bytes, 0x004e4942, 4);
    return bytes + payload;
}

void check_triangle(const Draw &draw, double first = 0, double second = 1) {
    expect(draw.topology == Topology::Triangles && draw.positions.size() == 3, "Expected one expanded triangle");
    equal(draw.positions[0], Position{first, 0, 0}, "Triangle corner 0");
    equal(draw.positions[1], Position{second, 0, 0}, "Triangle corner 1");
    equal(draw.positions[2], Position{first, 1, 0}, "Triangle corner 2");
}

void check_grid(const Output &output) {
    expect(output.draws.size() == 1 && output.placements.size() == 1, "Small grid draw/placement counts");
    const Draw &draw = output.draws[0];
    expect(draw.positions.size() == 24 && draw.normals.size() == 24 && draw.uv0.size() == 24,
            "Small grid selected tuple counts");
    // Each cell covers its square with the lower-left to upper-right diagonal, counterclockwise.
    constexpr unsigned int offsets[6][2]{{0, 0}, {1, 0}, {1, 1}, {0, 0}, {1, 1}, {0, 1}};
    std::size_t index = 0;
    for (unsigned int y = 0; y < 2; ++y) {
        for (unsigned int x = 0; x < 2; ++x) {
            for (const auto &offset : offsets) {
                const double px = x + offset[0];
                const double py = y + offset[1];
                equal(draw.positions[index], Position{px, py, 0}, "Every grid position/winding/coverage tuple");
                equal(draw.normals[index], Position{0, 0, 1}, "Every grid authored normal");
                equal(draw.uv0[index], Uv{px / 2, py / 2}, "Every grid source UV tuple");
                ++index;
            }
        }
    }
    expect(output.placements[0].draw == 0, "Small grid placement reference");
    equal(output.placements[0].world, identity(), "Small grid identity world transform");
    expect(output.logical_bytes() == 24 * 8 * 8 + 132, "Selected arrays plus F64 placement and u32 reference");
    expect(output.capacity_bytes() >= output.logical_bytes(), "Actual capacities include selected payload");
}

void test_grid(const std::filesystem::path &corpus) {
    for (IoMode io : {IoMode::File, IoMode::Memory}) {
        for (Stage stage : {Stage::Import, Stage::Prepare, Stage::Extract, Stage::Pipeline}) {
            const auto report = run(request(corpus, "obj-grid-2.obj", Fields::PositionNormalUv0,
                    NormalPolicy::Preserve, stage, io));
            expect(report.stage_ns.has_value() && report.finite == true && report.observation.has_value(), "Stage has finite observed evidence");
            const auto &value = *report.observation;
            expect(value.draws == 1 && value.placements == 1 && value.tuples == 24, "Small grid observation counts");
            close(value.area, 4, "Small grid source-local area");
            equal(value.bounds->min, Position{0, 0, 0}, "Small grid min bounds");
            equal(value.bounds->max, Position{2, 2, 0}, "Small grid max bounds");
            if (stage == Stage::Import || stage == Stage::Prepare) {
                expect(!report.output, "Native-only endpoints must not construct copied output");
            } else { check_grid(*report.output); }
        }
    }
    const auto selected = run(request(corpus, "obj-grid-2.obj"));
    expect(selected.output->draws[0].normals.empty() && selected.output->draws[0].uv0.empty(), "Unrequested fields are not extracted");
    expect(!selected.observation->normal_bounds && !selected.observation->uv0_bounds, "Unrequested fields have null observations");
}

void test_normals(const std::filesystem::path &corpus) {
    for (IoMode io : {IoMode::File, IoMode::Memory}) {
        auto options = request(corpus, "obj-repeated-1.obj", Fields::PositionNormal, NormalPolicy::Missing, Stage::Import, io);
        auto imported = run(options);
        expect(!imported.observation->normal_bounds && !imported.output, "Import must leave missing normals absent");
        for (Stage stage : {Stage::Prepare, Stage::Extract, Stage::Pipeline}) {
            options.stage = stage;
            const auto report = run(options);
            equal(report.observation->normal_bounds->min, Position{0, 0, 1}, "Generated planar smooth normals min");
            equal(report.observation->normal_bounds->max, Position{0, 0, 1}, "Generated planar smooth normals max");
            if (report.output) {
                for (const auto &normal : report.output->draws[0].normals) {
                    equal(normal, Position{0, 0, 1}, "Every generated planar normal");
                }
            }
        }
    }
    Fixtures fixtures;
    fixtures.write("mixed.obj", "o authored\ns 1\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 -1\n"
                                "f 1//1 2//1 3//1\no absent\nv 2 0 0\nv 3 0 0\nv 2 1 0\nf 4 5 6\n");
    for (IoMode io : {IoMode::File, IoMode::Memory}) {
        const auto report = run(request(fixtures.path, "mixed.obj", Fields::PositionNormal, NormalPolicy::Missing, Stage::Pipeline, io));
        expect(report.output->draws.size() == 2, "Independent authored/missing normal meshes");
        for (const auto &draw : report.output->draws) {
            expect(draw.positions.size() == 3 && draw.normals.size() == 3, "Mixed normal tuple counts");
            const auto expected = draw.positions[0][0] == 0 ? Position{0, 0, -1} : Position{0, 0, 1};
            for (const auto &normal : draw.normals) { equal(normal, expected, "Authored normals survive missing-only generation"); }
        }
    }
    auto preserve = request(corpus, "obj-repeated-1.obj", Fields::PositionNormal);
    rejects(Status::Unsupported, [&] { run(preserve); });
}

void test_shared(const std::filesystem::path &corpus) {
    constexpr std::array<Uv, 3> source_uv{{{0, 0}, {1, 0}, {0, 1}}};
    for (IoMode io : {IoMode::File, IoMode::Memory}) {
        const auto options = request(corpus, "gltf-shared-8.glb", Fields::PositionNormalUv0, NormalPolicy::Preserve, Stage::Extract, io);
        const auto input = load_input(options);
        Native native(options, input);
        native.import();
        expect(native.scene().mNumMeshes == 8, "One native draw per glTF primitive definition");
        for (unsigned int i = 0; i < native.scene().mNumMeshes; ++i) {
            const auto &mesh = *native.scene().mMeshes[i];
            expect(mesh.mNumVertices == 3, "Shared primitive vertex count");
            for (unsigned int j = 0; j < 3; ++j) {
                equal(Uv{mesh.mTextureCoords[0][j].x, mesh.mTextureCoords[0][j].y},
                        Uv{source_uv[j][0], 1 - source_uv[j][1]}, "Native glTF import uses one-minus-source V");
            }
        }
        const auto imported = native.observe(false);
        expect(imported.draws == 8 && imported.placements == 16 && imported.tuples == 24, "Shared native observation counts unique draw tuples");
        std::ostringstream import_json;
        auto import_options = options;
        import_options.stage = Stage::Import;
        Report import_report;
        import_report.observation = imported;
        import_report.finite = true;
        write_report(import_json, import_options, import_report);
        expect(import_json.str().find("\"uv0_convention\":\"assimp-native-one-minus-source-v\"") != std::string::npos,
                "Import labels the native UV convention");
        native.prepare();
        const auto output = native.extract();
        expect(output.draws.size() == 8 && output.placements.size() == 16, "Shared definitions are not multiplied into placement payload");
        for (const auto &draw : output.draws) {
            check_triangle(draw);
            expect(draw.normals.size() == 3 && draw.uv0.size() == 3, "Shared selected field counts");
            for (std::size_t j = 0; j < 3; ++j) {
                equal(draw.uv0[j], source_uv[j], "Preparation restores every glTF UV0 tuple");
                equal(draw.normals[j], Position{0, 0, 1}, "Shared authored normal");
            }
        }
        std::array<unsigned int, 8> at_zero{};
        std::array<unsigned int, 8> at_two{};
        for (const auto &placement : output.placements) {
            expect(placement.draw < 8, "Shared placement reference");
            const auto x = placement.world[12];
            expect(x == 0 || x == 2, "Shared source translation");
            equal(placement.world, identity(x), "Every shared world matrix component");
            (x == 0 ? at_zero : at_two)[placement.draw]++;
        }
        for (unsigned int i = 0; i < 8; ++i) {
            expect(at_zero[i] == 1 && at_two[i] == 1, "Each shared draw has both independently expected placements");
        }
        const auto observed = observe_output(output, options.fields);
        close(observed.area, 4, "Shared draw area is not multiplied by instances");
        close(observed.translation_x_sum, 16, "Shared draw-placement translation sum");
    }
    Fixtures fixtures;
    fixtures.write("nested.glb", nested_glb());
    const auto report = run(request(fixtures.path, "nested.glb", Fields::PositionNormalUv0));
    expect(report.output->draws.size() == 1 && report.output->placements.size() == 2, "Nested shared placement counts");
    check_triangle(report.output->draws[0]);
    constexpr std::array<Matrix, 2> worlds{{
            {0, 2, 0, 0, -3, 0, 0, 0, 0, 0, 1, 0, 3, 2, 0, 1},
            {0, 1, 0, 0, -1, 0, 0, 0, 0, 0, 1, 0, -1, -1, 0, 1}}};
    bool found[2]{false, false};
    for (const auto &placement : report.output->placements) {
        const unsigned int which = placement.world[12] == 3 ? 0 : 1;
        expect(!found[which] && placement.draw == 0, "Nested draw reference and placement uniqueness");
        equal(placement.world, worlds[which], "Analytic parent-rotation/child-scale composition");
        found[which] = true;
    }
    expect(found[0] && found[1], "Both nested transforms retained");
    constexpr std::array<Uv, 3> nested_uv{{{0, 0.25}, {1, 0.5}, {0.25, 0.75}}};
    for (unsigned int i = 0; i < 3; ++i) { equal(report.output->draws[0].uv0[i], nested_uv[i], "Asymmetric UV source values"); }
    close(report.observation->area, 0.5, "Nested transforms do not bake or scale the mesh-local area");
    close(report.observation->translation_x_sum, 2, "Nested placement X translation sum");
}

void test_precision(const std::filesystem::path &corpus) {
    Fixtures fixtures;
    const double adjacent = std::nextafter(1.0, 2.0);
    expect(adjacent > 1 && static_cast<float>(adjacent) == 1, "Independently selected adjacent binary64 values collide in binary32");
    fixtures.write("adjacent.ply", neighbor_ply(adjacent));
    for (IoMode io : {IoMode::File, IoMode::Memory}) {
        for (const auto &entry : {std::make_pair(corpus, "ply-f64-neighbors.ply"), std::make_pair(fixtures.path, "adjacent.ply")}) {
            const double next = entry.first == corpus ? 1 + std::ldexp(1.0, -30) : adjacent;
            for (Stage stage : {Stage::Import, Stage::Prepare, Stage::Extract, Stage::Pipeline}) {
                const auto report = run(request(entry.first, entry.second, Fields::Position, NormalPolicy::Preserve, stage, io));
                const double retained_next = sizeof(ai_real) == 8 ? next : 1;
                close(report.observation->bounds->max[0], retained_next, "Width-sensitive PLY bound", 0);
                close(report.observation->area, (retained_next - 1) / 2, "Width-sensitive PLY area", 0);
                if (report.output) {
                    const auto &draw = report.output->draws[0];
                    expect(draw.positions.size() == 3 && draw.normals.empty() && draw.uv0.empty(), "Precision case selects positions only");
                    equal(draw.positions[0], Position{1, 0, 0}, "Precision tuple 0");
                    equal(draw.positions[1], Position{retained_next, 0, 0}, "Precision tuple 1");
                    close(draw.positions[1][0], retained_next, "Exact second X value", 0);
                    equal(draw.positions[2], Position{1, 1, 0}, "Precision tuple 2");
                    expect(std::signbit(draw.positions[0][1]), "Signed source zero survives extraction");
                    if (sizeof(ai_real) == 8) {
                        expect(draw.positions[1][0] != draw.positions[0][0], "F64 neighbors remain distinct after cleanup");
                    } else {
                        expect(draw.positions[1][0] == draw.positions[0][0], "F32 honestly characterizes its precision limit");
                    }
                } else { expect(!report.output, "Precision native endpoint has no output"); }
            }
        }
    }
}

void test_ownership(const std::filesystem::path &corpus) {
    Output detached;
    Fields fields;
    {
        const auto options = sizeof(ai_real) == 8
                ? request(corpus, "ply-f64-neighbors.ply")
                : request(corpus, "obj-grid-2.obj", Fields::PositionNormalUv0);
        fields = options.fields;
        auto input = load_input(options);
        {
            Native native(options, input);
            native.import();
            native.prepare();
            detached = native.extract();
            native.free_scene();
            expect(observe_output(detached, fields).tuples != 0, "Output is usable immediately after FreeScene");
        }
        std::fill(input.memory.begin(), input.memory.end(), '\x7f');
    }
    const std::vector<Position> reuse(4096, Position{-9, -8, -7});
    expect(reuse[0][0] == -9, "Heap reuse exercise");
    expect(observe_output(detached, fields).tuples != 0, "Output survives importer/IO/input destruction");
    if (sizeof(ai_real) == 8) {
        close(detached.draws[0].positions[1][0], 1 + std::ldexp(1.0, -30), "Detached F64 value after all native destruction", 0);
        equal(detached.placements[0].world, identity(), "Detached placement after all native destruction");
    } else {
        check_grid(detached);
        const auto pipeline = run(request(corpus, "gltf-shared-1.glb", Fields::PositionNormalUv0));
        check_triangle(pipeline.output->draws[0]);
        equal(pipeline.output->draws[0].normals[0], Position{0, 0, 1}, "Pipeline detached authored normal");
        equal(pipeline.output->draws[0].uv0[0], Uv{0, 0}, "Pipeline detached source UV");
        expect(pipeline.output->placements.size() == 2, "Pipeline detached shared placements");
    }
}

void test_points(const std::filesystem::path &corpus) {
    for (IoMode io : {IoMode::File, IoMode::Memory}) {
        for (Stage stage : {Stage::Import, Stage::Extract, Stage::Pipeline}) {
            const auto report = run(request(corpus, "ply-points-1024.ply", Fields::Position, NormalPolicy::Preserve, stage, io));
            expect(report.observation->tuples == 1024 && report.observation->draws == 1 && report.observation->placements == 1,
                    "Point PLY counts without synthesized faces");
            close(report.observation->area, 0, "Points have zero triangle area");
            if (report.output) {
                const auto &draw = report.output->draws[0];
                expect(draw.topology == Topology::Points && draw.positions.size() == 1024, "Detached point list");
                for (unsigned int i = 0; i < 1024; ++i) { equal(draw.positions[i], Position{static_cast<double>(i), 0, 0}, "Every source point tuple"); }
            }
        }
        rejects(Status::Unsupported, [&] { run(request(corpus, "ply-points-1024.ply", Fields::Position, NormalPolicy::Preserve, Stage::Prepare, io)); });
    }
}

void test_formats(const std::filesystem::path &corpus) {
    for (IoMode io : {IoMode::File, IoMode::Memory}) {
        for (const char *file : {"ply-triangle-1.ply", "stl-repeated-1.stl"}) {
            if (sizeof(ai_real) == 8 && std::string_view(file) != "ply-triangle-1.ply") { continue; }
            for (Stage stage : {Stage::Import, Stage::Prepare, Stage::Extract, Stage::Pipeline}) {
                const auto report = run(request(corpus, file, Fields::Position, NormalPolicy::Preserve, stage, io));
                expect(report.observation->draws == 1 && report.observation->placements == 1 && report.observation->tuples == 3, "Triangle family counts");
                close(report.observation->area, 0.5, "Triangle family analytic area");
                if (report.output) { check_triangle(report.output->draws[0]); }
            }
        }
    }
}

void test_rejection(const std::filesystem::path &corpus) {
    Fixtures fixtures;
    fixtures.write("quad.ply", "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\n"
                               "element face 1\nproperty list uchar uint vertex_indices\nend_header\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n4 0 1 2 3\n");
    rejects(Status::Unsupported, [&] { run(request(fixtures.path, "quad.ply")); }, "prepare");
    auto missing = request(corpus, "ply-triangle-1.ply", Fields::PositionNormalUv0, NormalPolicy::Missing);
    rejects(Status::Unsupported, [&] { run(missing); }, "prepare");
    auto traversal = request(corpus, "../outside.ply");
    rejects(Status::PolicyRejected, [&] { run(traversal); }, "input");
    auto absent = request(corpus, "absent.ply");
    rejects(Status::PolicyRejected, [&] { run(absent); }, "input");
    for (IoMode mode : {IoMode::File, IoMode::Memory}) {
        const auto primary = sizeof(ai_real) == 4 ? "obj-repeated-1.obj" : "ply-triangle-1.ply";
        const std::string_view expected_prefix = sizeof(ai_real) == 4 ? "o re" : "ply\n";
        const auto options = request(corpus, primary, Fields::Position, NormalPolicy::Preserve, Stage::Import, mode);
        const auto input = load_input(options);
        Native native(options, input);
        expect(!native.io().Exists((input.path + ".not-requested").c_str()), "Absent optional discovery is not an authored read");
        for (unsigned int attempt = 0; attempt < 2; ++attempt) {
            std::unique_ptr<Assimp::IOStream> stream(native.io().Open(input.path.c_str()));
            char prefix[4]{};
            expect(stream && stream->Read(prefix, 1, sizeof(prefix)) == sizeof(prefix)
                            && std::string_view(prefix, sizeof(prefix)) == expected_prefix,
                    "Repeated primary opens return independent streams at byte zero");
        }
        native.import();
        expect(native.observe(false).tuples == 3, "Optional Exists and primary reopens do not reject single-file import");
    }
    if (sizeof(ai_real) == 4) {
        fixtures.write("textured.obj", "mtllib authored.mtl\no image-task\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl textured\nf 1 2 3\n");
        fixtures.write("authored.mtl", "newmtl textured\nKd 1 1 1\nmap_Kd deliberately-unavailable.png\n");
        rejects(Status::PolicyRejected, [&] { run(request(fixtures.path, "textured.obj")); }, "import");
        rejects(Status::Unsupported, [&] { run(request(fixtures.path, "textured.obj", Fields::Position, NormalPolicy::Preserve, Stage::Pipeline, IoMode::File)); }, "import");

        fixtures.write("missing-explicit-mtl.obj", "mtllib missing.mtl\no triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
        fixtures.write("magic-prefix-sidecar.obj", "mtllib $$$___magic___$$$.mtl\no triangle\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
        struct IoRegression { const char *file; IoMode io; Status status; };
        const IoRegression regressions[]{
                {"missing-explicit-mtl.obj", IoMode::File, Status::Unsupported},
                {"magic-prefix-sidecar.obj", IoMode::Memory, Status::PolicyRejected}};
        std::string failures;
        for (const auto &regression : regressions) {
            for (Stage stage : {Stage::Import, Stage::Pipeline}) {
                try {
                    rejects(regression.status, [&] {
                        run(request(fixtures.path, regression.file, Fields::Position, NormalPolicy::Preserve, stage, regression.io));
                    }, "import");
                } catch (const std::exception &error) {
                    failures += std::string(regression.file) + "/" + (stage == Stage::Import ? "import" : "pipeline")
                            + ": " + error.what() + "\n";
                }
            }
        }
        expect(failures.empty(), "IO contract regressions:\n" + failures);
    } else {
        rejects(Status::Unsupported, [&] { run(request(corpus, "gltf-shared-1.glb")); });
    }
    Output nonfinite;
    Draw draw;
    draw.topology = Topology::Points;
    draw.positions.push_back({std::numeric_limits<double>::quiet_NaN(), 0, 0});
    nonfinite.draws.push_back(std::move(draw));
    nonfinite.placements.push_back({0, identity()});
    try {
        observe_output(nonfinite, Fields::Position);
        throw std::runtime_error("Nonfinite selected output was accepted");
    } catch (const Error &error) {
        expect(error.status == Status::SemanticMismatch && error.nonfinite, "Nonfinite output has a typed finite-status failure");
    }
}

void test_json(const std::filesystem::path &corpus) {
    auto options = request(corpus, "ply-triangle-1.ply", Fields::Position, NormalPolicy::Preserve, Stage::Import);
    const auto success = run(options);
    std::ostringstream stream;
    write_report(stream, options, success);
    const auto text = stream.str();
    for (const char *required : {"\"schema\":\"baozi-assimp-observation/v1\"", "\"engine\":\"assimp\"",
                 "\"status\":\"success\"", "\"stage\":\"import\"", "\"io_mode\":\"memory\"",
                 "\"fields\":\"position\"", "\"normal_policy\":\"preserve\"", "\"output\":null",
                 "\"io_backend\":\"ReadFile + exact-primary preloaded IOSystem\"",
                 "\"max_work_units\":0", "\"max_work_units_enforced\":false", "\"finite\":true",
                 "\"output_scalar_bytes\":8", "\"observation_scalar_bytes\":8", "\"normal_bounds\":null",
                 "\"uv0_bounds\":null", "\"tuples\":3", "\"placements\":1", "\"draws\":1"}) {
        expect(text.find(required) != std::string::npos, "Missing success JSON field: " + std::string(required));
    }
    expect(std::count(text.begin(), text.end(), '\n') == 1 && text.back() == '\n', "One stdout JSON object");
    expect(text.find("\"scalar_bytes\":" + std::to_string(sizeof(ai_real))) != std::string::npos, "Retained scalar width in JSON");
    expect(text.find("\"computation_scalar_bytes\":" + std::to_string(sizeof(ai_real))) != std::string::npos, "Computation width in JSON");
    const std::pair<Status, const char *> failure_statuses[]{
            {Status::PolicyRejected, "policy_rejected"},
            {Status::Unsupported, "unsupported"},
            {Status::SemanticMismatch, "semantic_mismatch"},
            {Status::Timeout, "timeout"},
            {Status::Crash, "crash"}};
    for (const auto &[status, wire_status] : failure_statuses) {
        Report failure;
        failure.status = status;
        failure.message = "quoted \"field\"\\path\nline\r\t";
        failure.message.push_back('\0');
        failure.message.push_back('\x01');
        failure.finite = false;
        failure.failure_stage = "observe";
        std::ostringstream emitted;
        write_report(emitted, options, failure);
        const auto json = emitted.str();
        expect(json.find("\"status\":\"" + std::string(wire_status) + "\"") != std::string::npos, "Finite failure status vocabulary");
        expect(json.find("\"observation\":null") != std::string::npos && json.find("\"stage_ns\":null") != std::string::npos,
                "Failure does not fabricate observations/timing");
        expect(json.find("\"failure_stage\":\"observe\"") != std::string::npos, "Failure serializes its actual operation");
        expect(json.find("quoted \\\"field\\\"\\\\path\\nline\\r\\t\\u0000\\u0001") != std::string::npos, "JSON diagnostic escaping");
        expect(std::count(json.begin(), json.end(), '\n') == 1, "Diagnostic newlines remain inside one escaped JSON object");
    }
    const char *valid[]{"probe", ".", "asset.ply", "--comparison", "pipeline", "--io-mode", "file",
            "--fields", "position", "--normals", "missing", "--max-work-units", "18446744073709551615"};
    Request parsed;
    parse_request(13, valid, parsed);
    expect(parsed.max_work_units == std::numeric_limits<std::uint64_t>::max() && parsed.stage == Stage::Pipeline,
            "Fixed CLI parses the complete unsigned work-budget record");
    const char *conversion[]{"probe", ".", "asset.ply", "--fields", "position-world-baked"};
    rejects(Status::Unsupported, [&] { parse_request(5, conversion, parsed); });
    const char *image[]{"probe", ".", "asset.ply", "--fields", "image"};
    rejects(Status::Unsupported, [&] { parse_request(5, image, parsed); });
    const char *missing_options[]{"probe", ".", "asset.ply", "--comparison", "import"};
    rejects(Status::PolicyRejected, [&] { parse_request(5, missing_options, parsed); });
    // Keep every other required option valid so an unrelated missing-option guard cannot mask these checks.
    const char *bad_budget[]{"probe", ".", "asset.ply", "--comparison", "pipeline", "--io-mode", "file",
            "--fields", "position", "--normals", "missing", "--max-work-units", "18446744073709551616"};
    rejects(Status::PolicyRejected, [&] { parse_request(13, bad_budget, parsed); });
    const char *duplicate[]{"probe", ".", "asset.ply", "--comparison", "import", "--comparison", "extract",
            "--io-mode", "file", "--fields", "position", "--normals", "missing", "--max-work-units", "1"};
    rejects(Status::PolicyRejected, [&] { parse_request(15, duplicate, parsed); });
}

void test_runtime() {
    check_runtime();
    expect(((aiGetCompileFlags() & ASSIMP_CFLAGS_DOUBLE_SUPPORT) != 0 ? 8u : 4u) == sizeof(ai_real), "Header/runtime precision agreement");
    expect(sizeof(ai_real) == BAOZI_ASSIMP_SCALAR_BYTES, "Explicit configure/header precision agreement");
    Assimp::Importer importer;
    expect(importer.GetScene() == nullptr, "Fresh importer has no prior Scene");
    expect(importer.IsExtensionSupported(".ply"), "PLY reader is enabled");
    for (const char *extension : {".obj", ".stl", ".gltf", ".glb"}) {
        expect(importer.IsExtensionSupported(extension) == (sizeof(ai_real) == 4), "Only admitted readers are enabled per precision variant");
    }
    expect(!importer.IsExtensionSupported(".fbx") && !importer.IsExtensionSupported(".dae"), "Unused importers are disabled");
}

} // namespace

int main(int argc, char **argv) {
    try {
        expect(argc == 3, "usage: baozi-assimp-semantic SCENARIO CORPUS_ROOT");
        const std::string_view scenario(argv[1]);
        const auto corpus = std::filesystem::u8path(argv[2]);
        if (scenario == "runtime") { test_runtime(); }
        else if (scenario == "grid") { test_grid(corpus); }
        else if (scenario == "normals") { test_normals(corpus); }
        else if (scenario == "shared") { test_shared(corpus); }
        else if (scenario == "precision") { test_precision(corpus); }
        else if (scenario == "ownership") { test_ownership(corpus); }
        else if (scenario == "points") { test_points(corpus); }
        else if (scenario == "formats") { test_formats(corpus); }
        else if (scenario == "rejection") { test_rejection(corpus); }
        else if (scenario == "json") { test_json(corpus); }
        else { throw std::runtime_error("Unknown semantic test scenario"); }
        std::cout << scenario << ": semantic evidence passed\n";
        return 0;
    } catch (const std::exception &error) {
        std::cerr << "semantic failure: " << error.what() << '\n';
        return 1;
    }
}
