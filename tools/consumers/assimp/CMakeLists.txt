cmake_minimum_required(VERSION 3.22)
project(baozi_assimp_comparison LANGUAGES C CXX)

set(BAOZI_ASSIMP_SOURCE_DIR "" CACHE PATH "Existing, unmodified Assimp 6.0.5 source; never downloaded")
set(BAOZI_ASSIMP_SOURCE_REVISION "392a658f9c271be965271f45e7521a1b80ea4392"
    CACHE STRING "Caller-declared source revision; archive provenance must be retained externally")
set(BAOZI_ASSIMP_PRECISION "f32" CACHE STRING "Assimp retained geometry precision: f32 or f64")
set_property(CACHE BAOZI_ASSIMP_PRECISION PROPERTY STRINGS f32 f64)
set(BAOZI_ASSIMP_CORPUS_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../../target/assimp-comparison/corpus"
    CACHE PATH "Previously generated comparison corpus for semantic tests")
set(assimp_release_revision "392a658f9c271be965271f45e7521a1b80ea4392")
if(NOT BAOZI_ASSIMP_SOURCE_REVISION STREQUAL assimp_release_revision)
    message(FATAL_ERROR "BAOZI_ASSIMP_SOURCE_REVISION must be ${assimp_release_revision}")
endif()

if(NOT EXISTS "${BAOZI_ASSIMP_SOURCE_DIR}/include/assimp/Importer.hpp")
    message(FATAL_ERROR "Set BAOZI_ASSIMP_SOURCE_DIR to existing Assimp 6.0.5 source")
endif()
if(NOT EXISTS "${BAOZI_ASSIMP_SOURCE_DIR}/assimp.pc.in")
    message(FATAL_ERROR "Release source archive must include root assimp.pc.in; upstream configures it even with ASSIMP_INSTALL=OFF")
endif()
if(NOT BAOZI_ASSIMP_PRECISION MATCHES "^(f32|f64)$")
    message(FATAL_ERROR "BAOZI_ASSIMP_PRECISION must be f32 or f64")
endif()
file(REAL_PATH "${BAOZI_ASSIMP_SOURCE_DIR}" assimp_source)
file(READ "${assimp_source}/CMakeLists.txt" assimp_cmake)
if(NOT assimp_cmake MATCHES "PROJECT\\(Assimp VERSION 6\\.0\\.5[ \r\n]")
    message(FATAL_ERROR "The comparator requires the Assimp 6.0.5 release source")
endif()

set(assimp_source_identity "archive-declared-pin")
find_package(Git QUIET)
if(Git_FOUND)
    execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${assimp_source}" rev-parse --show-toplevel
        OUTPUT_VARIABLE assimp_git_root OUTPUT_STRIP_TRAILING_WHITESPACE ERROR_QUIET)
    if(assimp_git_root)
        file(REAL_PATH "${assimp_git_root}" assimp_git_root)
    endif()
    if(assimp_git_root STREQUAL assimp_source)
        execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${assimp_source}" rev-parse HEAD
            OUTPUT_VARIABLE assimp_revision OUTPUT_STRIP_TRAILING_WHITESPACE COMMAND_ERROR_IS_FATAL ANY)
        execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${assimp_source}" status --porcelain --untracked-files=no
            OUTPUT_VARIABLE assimp_changes OUTPUT_STRIP_TRAILING_WHITESPACE COMMAND_ERROR_IS_FATAL ANY)
        if(NOT assimp_revision STREQUAL assimp_release_revision OR assimp_changes)
            message(FATAL_ERROR "Assimp source must be clean at ${assimp_release_revision}; do not alter the reference checkout")
        endif()
        set(assimp_source_identity "git-verified")
    endif()
endif()
if(assimp_source_identity STREQUAL "archive-declared-pin")
    message(WARNING "Archive source: declared pin ${BAOZI_ASSIMP_SOURCE_REVISION}; version checked, contents not Git-verified. Retain archive/source digests before admitting ratios.")
endif()

option(BUILD_SHARED_LIBS "Build Assimp as a shared library" OFF)
option(USE_STATIC_CRT "Use the static MSVC runtime for both probe and Assimp" OFF)
if(MSVC)
    if(USE_STATIC_CRT)
        set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
    else()
        set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL")
    endif()
endif()

set(ASSIMP_BUILD_ALL_IMPORTERS_BY_DEFAULT OFF CACHE BOOL "" FORCE)
set(ASSIMP_BUILD_ALL_EXPORTERS_BY_DEFAULT OFF CACHE BOOL "" FORCE)
set(ASSIMP_NO_EXPORT ON CACHE BOOL "" FORCE)
get_cmake_property(probe_cache_variables CACHE_VARIABLES)
foreach(variable IN LISTS probe_cache_variables)
    if(variable MATCHES "^ASSIMP_BUILD_.*_(IMPORTER|EXPORTER)$")
        set(${variable} OFF CACHE BOOL "" FORCE)
    endif()
endforeach()
foreach(option_name IN ITEMS ASSIMP_BUILD_ASSIMP_TOOLS ASSIMP_BUILD_SAMPLES ASSIMP_BUILD_TESTS
        ASSIMP_BUILD_DOCS ASSIMP_INSTALL ASSIMP_OPT_BUILD_PACKAGES ASSIMP_HUNTER_ENABLED
        ASSIMP_BUILD_DRACO ASSIMP_BUILD_DRACO_STATIC ASSIMP_BUILD_USD_IMPORTER
        ASSIMP_BUILD_M3D_IMPORTER ASSIMP_BUILD_VRML_IMPORTER ASSIMP_ANDROID_JNIIOSYSTEM
        ASSIMP_BUILD_FRAMEWORK ASSIMP_COVERALLS ASSIMP_ASAN ASSIMP_UBSAN)
    set(${option_name} OFF CACHE BOOL "" FORCE)
endforeach()
set(ASSIMP_BUILD_ZLIB ON CACHE BOOL "Use only bundled local zlib" FORCE)
set(ASSIMP_IGNORE_GIT_HASH ON CACHE BOOL "Avoid attributing an archive to its enclosing repository" FORCE)
set(GIT_COMMIT_HASH 0)
if(assimp_source_identity STREQUAL "git-verified")
    set(GIT_COMMIT_HASH 392a658f)
endif()

set(assimp_float_importers ON)
set(assimp_scalar_bytes 4)
set(assimp_double OFF)
if(BAOZI_ASSIMP_PRECISION STREQUAL "f64")
    set(assimp_float_importers OFF)
    set(assimp_scalar_bytes 8)
    set(assimp_double ON)
endif()
set(ASSIMP_DOUBLE_PRECISION ${assimp_double} CACHE BOOL "" FORCE)
foreach(format IN ITEMS OBJ STL GLTF)
    set(ASSIMP_BUILD_${format}_IMPORTER ${assimp_float_importers} CACHE BOOL "" FORCE)
endforeach()
set(ASSIMP_BUILD_PLY_IMPORTER ON CACHE BOOL "" FORCE)

add_subdirectory("${assimp_source}" "${CMAKE_CURRENT_BINARY_DIR}/assimp" EXCLUDE_FROM_ALL)
include(CTest)

function(configure_probe target)
    target_compile_features(${target} PRIVATE cxx_std_17)
    target_link_libraries(${target} PRIVATE assimp)
    # The release sets this in its directory, rather than exporting it to consumers.
    if(assimp_double)
        target_compile_definitions(${target} PRIVATE ASSIMP_DOUBLE_PRECISION)
    endif()
    if(WIN32 AND BUILD_SHARED_LIBS)
        target_compile_definitions(${target} PRIVATE ASSIMP_DLL)
    endif()
    target_compile_definitions(${target} PRIVATE
        BAOZI_ASSIMP_SCALAR_BYTES=${assimp_scalar_bytes}
        BAOZI_ASSIMP_SOURCE_IDENTITY="${assimp_source_identity}")
    if(MSVC)
        target_compile_options(${target} PRIVATE /W4 /permissive-)
    else()
        target_compile_options(${target} PRIVATE -Wall -Wextra -Wpedantic)
    endif()
endfunction()

add_executable(baozi-assimp-probe src/main.cpp)
configure_probe(baozi-assimp-probe)

if(BUILD_TESTING)
    add_executable(baozi-assimp-semantic tests/semantic.cpp src/main.cpp)
    target_compile_definitions(baozi-assimp-semantic PRIVATE BAOZI_ASSIMP_NO_MAIN)
    configure_probe(baozi-assimp-semantic)
    set(semantic_scenarios runtime precision ownership rejection json points formats)
    if(BAOZI_ASSIMP_PRECISION STREQUAL "f32")
        list(APPEND semantic_scenarios grid normals shared)
    endif()
    foreach(scenario IN LISTS semantic_scenarios)
        add_test(NAME assimp-${scenario}
            COMMAND baozi-assimp-semantic "${scenario}" "${BAOZI_ASSIMP_CORPUS_DIR}")
    endforeach()
endif()

file(GENERATE OUTPUT "${CMAKE_CURRENT_BINARY_DIR}/comparison-build-$<CONFIG>.txt" CONTENT
"expected_assimp_revision=${assimp_release_revision}
declared_source_revision=${BAOZI_ASSIMP_SOURCE_REVISION}
source=${assimp_source}
source_identity=${assimp_source_identity}
precision=${BAOZI_ASSIMP_PRECISION}
compiler=${CMAKE_CXX_COMPILER}
compiler_id=${CMAKE_CXX_COMPILER_ID}
compiler_version=${CMAKE_CXX_COMPILER_VERSION}
configuration=$<CONFIG>
generator=${CMAKE_GENERATOR}
system=${CMAKE_SYSTEM_NAME}/${CMAKE_SYSTEM_PROCESSOR}
shared=${BUILD_SHARED_LIBS}
msvc_runtime=${CMAKE_MSVC_RUNTIME_LIBRARY}
cxx_flags=${CMAKE_CXX_FLAGS}
release_flags=${CMAKE_CXX_FLAGS_RELEASE}
relwithdebinfo_flags=${CMAKE_CXX_FLAGS_RELWITHDEBINFO}
link_flags=${CMAKE_EXE_LINKER_FLAGS}
allocator=default C++ new / platform runtime; not intercepted
timing=optimized uninstrumented only; caller must record CPU flags, stdlib/CRT and host
")
