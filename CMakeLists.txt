# SPDX-License-Identifier: GPL-2.0-or-later
cmake_minimum_required(VERSION 3.22)
project(control-surface VERSION 0.1.0 LANGUAGES C CXX)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

include(GNUInstallDirs)
include(CTest)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Network DBus)
find_package(PkgConfig REQUIRED)
pkg_check_modules(UDEV REQUIRED IMPORTED_TARGET libudev)

if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    add_compile_options(-Wall -Wextra -Wpedantic)
endif()

# Pad protocol (no Qt)
add_library(cs_proto STATIC
    src/proto/ch552proto.cpp
    src/proto/scheme.cpp
    src/proto/hidrawdev.cpp
)
target_include_directories(cs_proto PUBLIC src/proto)

add_executable(ch552-padprog src/padprog/main.cpp)
target_link_libraries(ch552-padprog PRIVATE cs_proto Qt6::Core)

# Daemon core (QtCore + QtNetwork + QtDBus)
add_library(cs_core STATIC
    src/daemon/keynames.cpp
    src/daemon/hardwaremap.cpp
    src/daemon/decoder.cpp
    src/daemon/config.cpp
    src/daemon/configwatcher.cpp
    src/daemon/capabilities.cpp
    src/daemon/coalescer.cpp
    src/daemon/engine.cpp
    src/daemon/keysink.cpp
    src/daemon/uinputsink.cpp
    src/daemon/paddevice.cpp
    src/daemon/windowtracker.cpp
    src/daemon/hyprlandtracker.cpp
    src/daemon/kdenliveclient.cpp
    src/daemon/kdenlivedbusclient.cpp
    src/daemon/learn.cpp
)
target_include_directories(cs_core PUBLIC src/daemon src/proto)
target_link_libraries(cs_core PUBLIC cs_proto Qt6::Core Qt6::Network Qt6::DBus PkgConfig::UDEV)

add_executable(control-surfaced src/daemon/main.cpp)
target_link_libraries(control-surfaced PRIVATE cs_core)
qt_add_resources(control-surfaced "defaults" PREFIX "/control-surface" BASE data FILES data/config.example.jsonc)

# Mock Kdenlive implementing docs/kdenlive-api-contract.md
add_library(cs_mock STATIC src/mock/mockkdenlive.cpp)
target_include_directories(cs_mock PUBLIC src/mock src/daemon)
target_link_libraries(cs_mock PUBLIC Qt6::Core Qt6::DBus)

add_executable(mock-kdenlive src/mock/main.cpp)
target_link_libraries(mock-kdenlive PRIVATE cs_mock)

install(TARGETS control-surfaced ch552-padprog RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})

if(BUILD_TESTING)
    find_package(Qt6 REQUIRED COMPONENTS Test)
    foreach(t proto decoder hardwaremap config coalescer engine hyprland kdenlive_dbus kdenlive_bus uinput paddevice)
        add_executable(tst_${t} tests/tst_${t}.cpp)
        target_link_libraries(tst_${t} PRIVATE cs_core cs_mock Qt6::Test)
        target_compile_definitions(tst_${t} PRIVATE CS_SOURCE_DIR="${CMAKE_SOURCE_DIR}")
        if(NOT t STREQUAL "kdenlive_bus")
            add_test(NAME ${t} COMMAND tst_${t})
        endif()
    endforeach()
    # Pad firmware logic (firmware/src/padlogic.c), built for the host. Plain C,
    # no Qt: the same file is compiled by SDCC for the CH552.
    add_executable(tst_fw_logic tests/tst_fw_logic.c firmware/src/padlogic.c)
    target_include_directories(tst_fw_logic PRIVATE firmware/src)
    add_test(NAME fw_logic COMMAND tst_fw_logic)
    add_executable(tst_fw_store tests/tst_fw_store.c firmware/src/padstore.c firmware/src/padlogic.c)
    target_include_directories(tst_fw_store PRIVATE firmware/src)
    add_test(NAME fw_store COMMAND tst_fw_store)
    target_sources(tst_kdenlive_dbus PRIVATE tests/rawclient.h)
    target_sources(tst_kdenlive_bus PRIVATE tests/rawclient.h)
    # The pacing test runs the mock as a separate process (its own event loop).
    target_compile_definitions(tst_kdenlive_bus PRIVATE CS_MOCK_BINARY="$<TARGET_FILE:mock-kdenlive>")
    add_dependencies(tst_kdenlive_bus mock-kdenlive)
    # Multi-subscriber and ownership tests need a real bus: always a private one.
    find_program(DBUS_RUN_SESSION dbus-run-session REQUIRED)
    add_test(NAME kdenlive_bus COMMAND ${CMAKE_COMMAND} -E env CS_PRIVATE_BUS=1 ${DBUS_RUN_SESSION} -- $<TARGET_FILE:tst_kdenlive_bus>)
endif()
