cmake_minimum_required(VERSION 3.21)
function(stage source subdirectory)
    file(MAKE_DIRECTORY "${destination}/${subdirectory}")
    get_filename_component(name "${source}" NAME)
    file(COPY_FILE "${source}" "${destination}/${subdirectory}/${name}" ONLY_IF_DIFFERENT)
    list(APPEND roots "${source}")
    set(roots "${roots}" PARENT_SCOPE)
endfunction()
include("${manifest}")

# Inspect the original toolchain files so an old copy in the build directory
# cannot determine which DLLs a newly upgraded Qt plugin requires.
set(CMAKE_GET_RUNTIME_DEPENDENCIES_TOOL objdump)
set(CMAKE_GET_RUNTIME_DEPENDENCIES_COMMAND "${objdump}")
file(GET_RUNTIME_DEPENDENCIES
    LIBRARIES ${roots}
    DIRECTORIES "${qt_bin}" "${compiler_bin}"
    RESOLVED_DEPENDENCIES_VAR dependencies
    PRE_EXCLUDE_REGEXES "^api-ms-" "^ext-ms-"
    POST_EXCLUDE_REGEXES "[/\\\\][Ww][Ii][Nn][Dd][Oo][Ww][Ss][/\\\\]")
foreach(dependency IN LISTS dependencies)
    get_filename_component(name "${dependency}" NAME)
    file(COPY_FILE "${dependency}" "${destination}/${name}" ONLY_IF_DIFFERENT)
endforeach()
