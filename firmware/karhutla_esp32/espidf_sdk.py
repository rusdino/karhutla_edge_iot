"""Konfigurasi SCons PlatformIO untuk Native ESP-IDF v5.5.5 Prebuilt SDK (Tanpa Arduino Core).

Menautkan langsung header, flags, linker script, bootloader, dan pustaka .a ESP-IDF v5.5.5
tanpa mengompilasi atau menautkan FrameworkArduino.
"""
import os
from click.shell_completion import split_arg_string
from SCons.Script import Import

Import("env")

# Gunakan list kosong agar tidak memicu unduhan paket tambahan namun aman terhadap operator `in`
env["PIOFRAMEWORK"] = []


def load_flags(file_path):
    if not os.path.isfile(file_path):
        return []
    flags = []
    with open(file_path, "r", encoding="utf8") as fp:
        for line in fp.readlines():
            flags.extend([
                f.replace('"', '\\"')
                for f in split_arg_string(line)
                if f.strip()
            ])
    return flags


def configure_espidf_sdk(env, frameworks=None):
    platform = env.PioPlatform()
    board_config = env.BoardConfig()
    build_mcu = board_config.get("build.mcu", "esp32").lower()

    pkg_root = os.path.expanduser("~/.platformio/packages")
    target_libs_dir = (
        platform.get_package_dir(f"framework-arduinoespressif32-{build_mcu}-libs")
        or os.path.join(pkg_root, f"framework-arduinoespressif32-{build_mcu}-libs")
    )
    framework_dir = (
        platform.get_package_dir("framework-arduinoespressif32")
        or os.path.join(pkg_root, "framework-arduinoespressif32")
    )

    inc_file = os.path.join(target_libs_dir, "flags", "includes")
    inc_paths = [
        os.path.join(target_libs_dir, "include", f)
        for f in load_flags(inc_file)
        if not f.startswith("-iwithprefixbefore")
    ]

    user_defines = list(env.get("CPPDEFINES", []))
    # Hapus define duplikat dari _bare.py
    user_defines = [
        d for d in user_defines
        if d not in ("ESP32", "ESP_PLATFORM", "HAVE_CONFIG_H")
        and not (isinstance(d, tuple) and d[0] in ("F_CPU", "MBEDTLS_CONFIG_FILE"))
    ]

    env.Replace(
        ASFLAGS=[],
        ASPPFLAGS=[
            "-x",
            "assembler-with-cpp",
            load_flags(os.path.join(target_libs_dir, "flags", "S_flags")),
        ],
        CFLAGS=[load_flags(os.path.join(target_libs_dir, "flags", "c_flags"))],
        CCFLAGS=[
            "-Wall",
            "-Wextra",
            "-Werror=return-type",
        ],
        CXXFLAGS=[load_flags(os.path.join(target_libs_dir, "flags", "cpp_flags"))],
        CPPPATH=[
            os.path.join("$PROJECT_DIR", "include"),
            os.path.join(target_libs_dir, "${__get_board_memory_type(__env__)}", "include"),
        ] + inc_paths,
        CPPDEFINES=user_defines + [
            ("F_CPU", "$BOARD_F_CPU"),
            ("ESP32", "ESP32"),
        ] + [
            d[2:]
            for d in load_flags(os.path.join(target_libs_dir, "flags", "defines"))
        ],
        LIBPATH=[
            os.path.join(target_libs_dir, "lib"),
            os.path.join(target_libs_dir, "ld"),
            os.path.join(target_libs_dir, "${__get_board_memory_type(__env__)}"),
        ],
        LINKFLAGS=[
            "-Wl,--Map",
            os.path.join(env.subst("$BUILD_DIR"), "${PROGNAME}.map"),
            load_flags(os.path.join(target_libs_dir, "flags", "ld_flags")),
            load_flags(os.path.join(target_libs_dir, "flags", "ld_scripts")),
            "-Wl,-EL",
        ],
        LIBS=[
            lib[2:]
            for lib in load_flags(os.path.join(target_libs_dir, "flags", "ld_libs"))
        ],
    )

    bootloader_elf = os.path.join(
        target_libs_dir,
        "bin",
        "bootloader_${__get_board_boot_mode(__env__)}_${__get_board_f_flash(__env__)}.elf",
    )
    bootloader_cmd = env.Command(
        os.path.join("$BUILD_DIR", "bootloader.bin"),
        bootloader_elf,
        env.VerboseAction(
            " ".join([
                '"$OBJCOPY"',
                "--chip", build_mcu,
                "elf2image",
                "--flash-mode", "${__get_board_flash_mode(__env__)}",
                "--flash-freq", "${__get_board_f_flash(__env__)}",
                "--flash-size", board_config.get("upload.flash_size", "4MB"),
                "-o", "$TARGET", "$SOURCES",
            ]),
            "Building $TARGET",
        ),
    )
    env.Depends("$BUILD_DIR/$PROGNAME$PROGSUFFIX", bootloader_cmd)

    partitions_csv = os.path.join(framework_dir, "tools", "partitions", "default.csv")
    env.Replace(PARTITIONS_TABLE_CSV=partitions_csv)
    partition_table = env.Command(
        os.path.join("$BUILD_DIR", "partitions.bin"),
        "$PARTITIONS_TABLE_CSV",
        env.VerboseAction(
            '"$PYTHONEXE" "%s" -q $SOURCE $TARGET'
            % os.path.join(framework_dir, "tools", "gen_esp32part.py"),
            "Generating partitions $TARGET",
        ),
    )
    env.Depends("$BUILD_DIR/$PROGNAME$PROGSUFFIX", partition_table)

    env.Append(
        ELF2BINFLAGS=["--elf-sha256-offset", "0xb0"],
        FLASH_EXTRA_IMAGES=[
            ("0x1000", str(bootloader_cmd[0])),
            ("0x8000", os.path.join(env.subst("$BUILD_DIR"), "partitions.bin")),
            ("0xe000", os.path.join(framework_dir, "tools", "partitions", "boot_app0.bin")),
        ],
    )


env.AddMethod(configure_espidf_sdk, "BuildFrameworks")
