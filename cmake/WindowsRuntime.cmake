# The GUI, its child CLI and the Windows test executables share one runtime directory.
set(runtime_manifest "")
set(runtime_inputs "")
foreach(module IN ITEMS Core Gui Widgets Test)
    list(APPEND runtime_inputs "$<TARGET_FILE:Qt6::${module}>")
    string(APPEND runtime_manifest "stage([[$<TARGET_FILE:Qt6::${module}>]] .)\n")
endforeach()
foreach(plugin IN ITEMS QWindowsIntegrationPlugin QOffscreenIntegrationPlugin
        QICOPlugin QSvgIconPlugin QSvgPlugin QModernWindowsStylePlugin)
    if(NOT TARGET Qt6::${plugin})
        if(plugin STREQUAL "QModernWindowsStylePlugin")
            continue()
        endif()
        message(FATAL_ERROR "Missing Qt runtime plugin: ${plugin}")
    endif()
    get_target_property(plugin_type Qt6::${plugin} QT_PLUGIN_TYPE)
    list(APPEND runtime_inputs "$<TARGET_FILE:Qt6::${plugin}>")
    string(APPEND runtime_manifest "stage([[$<TARGET_FILE:Qt6::${plugin}>]] ${plugin_type})\n")
endforeach()
set(manifest "${CMAKE_CURRENT_BINARY_DIR}/windows-runtime-$<CONFIG>.cmake")
file(GENERATE OUTPUT "${manifest}" CONTENT "${runtime_manifest}")
get_filename_component(compiler_bin "${CMAKE_CXX_COMPILER}" DIRECTORY)
set(stamp "${CMAKE_CURRENT_BINARY_DIR}/windows-runtime-$<CONFIG>.stamp")
add_custom_command(OUTPUT "${stamp}"
    COMMAND "${CMAKE_COMMAND}"
        "-Dmanifest=${manifest}" "-Ddestination=$<TARGET_FILE_DIR:litzsim>"
        "-Dqt_bin=$<TARGET_FILE_DIR:Qt6::Core>" "-Dcompiler_bin=${compiler_bin}"
        "-Dobjdump=${CMAKE_OBJDUMP}" -P "${CMAKE_CURRENT_LIST_DIR}/StageMinGWRuntime.cmake"
    COMMAND "${CMAKE_COMMAND}" -E touch "${stamp}"
    DEPENDS "${manifest}" "${CMAKE_CURRENT_LIST_DIR}/StageMinGWRuntime.cmake" ${runtime_inputs}
    COMMENT "Deploying Qt plugins and their MinGW runtime dependencies"
    VERBATIM)
add_custom_target(litzsim-windows-runtime DEPENDS "${stamp}")
add_dependencies(litzcore litzsim-windows-runtime)
add_dependencies(litzsim litzsim-cli)

add_test(NAME windows-runtime COMMAND pfc-studio-test "${CMAKE_CURRENT_SOURCE_DIR}/examples/pfc.litzsim")
set_tests_properties(windows-runtime PROPERTIES TIMEOUT 60
    ENVIRONMENT "PATH=$ENV{SystemRoot}/System32;QT_PLUGIN_PATH=;QT_QPA_PLATFORM_PLUGIN_PATH=;QT_QPA_PLATFORM=windows")
