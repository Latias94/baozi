#pragma once

#include <assimp/Importer.hpp>
#include <assimp/scene.h>

#include <array>
#include <cstdint>
#include <filesystem>
#include <iosfwd>
#include <memory>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>

namespace comparison {

enum class Stage { Import, Prepare, Extract, Pipeline };
enum class IoMode { File, Memory };
enum class Fields { Position, PositionNormal, PositionNormalUv0 };
enum class NormalPolicy { Preserve, Missing };
enum class Status { Success, PolicyRejected, Unsupported, SemanticMismatch, Timeout, Crash };
enum class Topology { Triangles, Points };

struct Error : std::runtime_error {
    Status status;
    bool nonfinite;
    std::string failure_stage;
    Error(Status status, const std::string &message, bool nonfinite = false);
};

struct Request {
    std::filesystem::path root;
    std::filesystem::path relative;
    Stage stage = Stage::Import;
    IoMode io_mode = IoMode::File;
    Fields fields = Fields::Position;
    NormalPolicy normals = NormalPolicy::Preserve;
    std::uint64_t max_work_units = 0;
};

struct Input {
    std::string path;
    std::string extension;
    std::vector<char> memory;
    bool gltf = false;
};

using Position = std::array<double, 3>;
using Uv = std::array<double, 2>;
using Matrix = std::array<double, 16>;

struct Draw {
    Topology topology = Topology::Triangles;
    std::vector<Position> positions;
    std::vector<Position> normals;
    std::vector<Uv> uv0;
};

struct Placement {
    std::uint32_t draw;
    Matrix world;
};

struct Output {
    std::vector<Draw> draws;
    std::vector<Placement> placements;
    std::uint64_t logical_bytes() const;
    std::uint64_t capacity_bytes() const;
};

template <std::size_t N>
struct Bounds {
    std::array<double, N> min;
    std::array<double, N> max;
};

struct Observation {
    std::uint64_t draws = 0;
    std::uint64_t placements = 0;
    std::uint64_t tuples = 0;
    std::optional<Bounds<3>> bounds;
    double area = 0;
    std::optional<Bounds<3>> normal_bounds;
    std::optional<Bounds<2>> uv0_bounds;
    double translation_x_sum = 0;
};

struct Report {
    Status status = Status::Success;
    std::optional<std::uint64_t> stage_ns;
    std::optional<Observation> observation;
    std::optional<Output> output;
    std::optional<bool> finite;
    std::string message;
    std::string failure_stage;
};

// This state is private to the probe and its behavior tests, not an asset API.
class Native {
public:
    Native(const Request &request, const Input &input);
    ~Native();
    Native(const Native &) = delete;
    Native &operator=(const Native &) = delete;
    void import();
    void prepare();
    Output extract() const;
    Observation observe(bool prepared) const;
    const aiScene &scene() const;
    Assimp::IOSystem &io() const;
    void free_scene();

private:
    const Request &request_;
    const Input &input_;
    std::unique_ptr<Assimp::Importer> importer_;
    const aiScene *scene_ = nullptr;
    bool sidecar_attempted_ = false;
    bool prepared_ = false;
};

void check_runtime();
void parse_request(int argc, const char *const *argv, Request &request);
Input load_input(const Request &request);
Observation observe_output(const Output &output, Fields fields);
Report run(const Request &request);
void write_report(std::ostream &stream, const Request &request, const Report &report);
const char *name(Status value);

} // namespace comparison
