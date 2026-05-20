#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Iterable


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Audit feature parity between rs3-cache (Java) and rs3-cache-rs (Rust)."
    )
    parser.add_argument(
        "--java-root",
        type=Path,
        default=Path("../rs3-cache"),
        help="Path to Java repo root",
    )
    parser.add_argument(
        "--rust-root",
        type=Path,
        default=Path("."),
        help="Path to Rust repo root",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit JSON only",
    )
    return parser.parse_args()


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def find_between(lines: Iterable[str], needle: str, marker: str) -> list[str]:
    out: list[str] = []
    for line in lines:
        if needle not in line:
            continue
        start = line.find(marker)
        if start == -1:
            continue
        start += len(marker)
        end = line.find('"', start)
        if end == -1:
            continue
        out.append(line[start:end])
    return out


def java_dump_targets(unpack_java: str) -> list[str]:
    return sorted(
        {
            target
            for target in find_between(
                unpack_java.splitlines(),
                "root.resolve(",
                'root.resolve("',
            )
            if target.startswith("config/dump.")
        }
    )


def java_root_targets(unpack_java: str) -> list[str]:
    return sorted(
        set(
            find_between(
                unpack_java.splitlines(),
                "root.resolve(",
                'root.resolve("',
            )
        )
    )


def java_config_unpackers(java_root: Path) -> list[str]:
    config_dir = java_root / "src/main/java/rs3/unpack/config"
    unpackers = []
    for file in sorted(config_dir.glob("*Unpacker.java")):
        unpackers.append(file.stem.removesuffix("Unpacker").lower())
    return unpackers


def rust_command_names(cli_rs: str) -> list[str]:
    lines = cli_rs.splitlines()
    out: list[str] = []
    in_enum = False
    brace_depth = 0
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("pub enum Command"):
            in_enum = True
            brace_depth = 0
            continue
        if not in_enum:
            continue
        brace_depth += line.count("{")
        brace_depth -= line.count("}")
        if stripped and stripped[0].isupper() and stripped.endswith("{"):
            out.append(stripped.split("{", 1)[0].strip())
        if brace_depth < 0:
            break
    return sorted(set(out))


def rust_supported_configs() -> dict[str, str]:
    return {
        "param": "parse_param",
        "enum": "parse_enum",
        "dbtable": "parse_dbtable",
        "dbrow": "parse_dbrow",
        "inv": "parse_inv",
        "cursor": "parse_cursor",
        "controller": "parse_controller",
        "category": "parse_category",
        "seqgroup": "parse_seqgroup",
        "struct": "parse_struct",
        "area": "parse_area",
        "achievement": "parse_achievement",
        "bas": "parse_bas",
        "hunt": "parse_hunt",
        "idk": "parse_idk",
        "mesanim": "parse_mesanim",
        "itemcode": "parse_itemcode",
        "loc": "parse_loc",
        "material": "parse_material",
        "mel": "parse_mel",
        "npc": "parse_npc",
        "obj": "parse_obj",
        "quest": "parse_quest",
        "seq": "parse_seq",
        "spot": "parse_spot",
        "water": "parse_water",
        "gamelogevent": "parse_gamelogevent",
        "bugtemplate": "parse_bugtemplate",
        "varcstr": "parse_var_client_string",
        "varnbit": "parse_var_npc_bit",
        "vars": "parse_var_shared",
        "varsstr": "parse_var_shared_string",
        "flu": "parse_underlay",
        "flo": "parse_overlay",
        "msi": "parse_msi",
        "skybox": "parse_skybox",
        "worldarea": "parse_worldarea",
        "quickchatcat": "parse_quickchatcat",
        "headbar": "parse_headbar",
        "hitmark": "parse_hitmark",
        "light": "parse_light",
        "quickchatphrase": "parse_quickchatphrase",
        "billboard": "parse_billboard",
        "particleeffector": "parse_particle_effector",
        "particleemitter": "parse_particle_emitter",
        "texture": "parse_texture",
        "stylesheet": "parse_stylesheet",
        "varp": "parse_var",
        "varn": "parse_var",
        "varc": "parse_var",
        "varworld": "parse_var",
        "varregion": "parse_var",
        "varobj": "parse_var",
        "varclan": "parse_var",
        "varclansetting": "parse_var",
        "varcontroller": "parse_var",
        "varglobal": "parse_var",
        "varplayergroup": "parse_var",
        "varbit": "parse_varbit",
    }


def rust_unpack_targets() -> list[str]:
    return sorted(
        [
            "config/varps.json",
            "config/varbits.json",
            "config/params.json",
            "config/enums.json",
            "config/dbtables.json",
            "config/dbrows.json",
            "config/graphics.defaults",
            "config/audio.defaults",
            "config/wearpos.defaults",
            "config/worldmap.defaults",
            "config/title.defaults",
            "interface",
            "script/decompiled",
            "script/scripts.json",
            "model/decoded",
            "model/models.json|model/models_sample.json",
            "audio",
            "animator",
            "areas.png",
            "binary",
            "cutscene2d",
            "fontmetrics",
            "maps",
            "ttf",
            "uianim",
            "uianimcurve",
            "vfx",
            "worldmap",
        ]
    )


def normalize_dump_name(path: str) -> str:
    return path.removeprefix("config/dump.")


def audit(java_root: Path, rust_root: Path) -> dict[str, object]:
    java_unpack_path = java_root / "src/main/java/rs3/Unpack.java"
    rust_cli_path = rust_root / "src/cli.rs"

    java_unpack = read(java_unpack_path)
    rust_cli = read(rust_cli_path)

    java_dumps = java_dump_targets(java_unpack)
    java_dump_names = sorted(normalize_dump_name(name) for name in java_dumps)
    rust_configs = rust_supported_configs()

    missing_configs = sorted(
        name for name in java_dump_names if name not in rust_configs
    )

    java_targets = java_root_targets(java_unpack)
    rust_targets = rust_unpack_targets()
    java_top_targets = sorted({target.split("/", 1)[0] for target in java_targets})
    rust_top_targets = sorted({target.split("/", 1)[0] for target in rust_targets})

    high_level_map = {
        "interface": "interface",
        "script": "script/decompiled",
        "model": "model/decoded",
    }
    missing_high_level = sorted(
        target for target in java_targets if target in high_level_map and high_level_map[target] not in rust_targets
    )
    missing_top_targets = sorted(set(java_top_targets) - set(rust_top_targets))

    parity = (
        len(missing_configs) == 0
        and len(missing_high_level) == 0
        and len(missing_top_targets) == 0
    )

    return {
        "parity": parity,
        "java": {
            "config_unpacker_count": len(java_config_unpackers(java_root)),
            "config_dump_count": len(java_dumps),
            "config_dumps": java_dump_names,
            "output_target_count": len(java_targets),
            "output_targets": java_targets,
            "top_targets": java_top_targets,
        },
        "rust": {
            "command_count": len(rust_command_names(rust_cli)),
            "commands": rust_command_names(rust_cli),
            "supported_config_count": len(rust_configs),
            "supported_configs": sorted(rust_configs.keys()),
            "unpack_targets": rust_targets,
            "top_targets": rust_top_targets,
        },
        "missing": {
            "configs": missing_configs,
            "high_level_targets": missing_high_level,
            "top_targets": missing_top_targets,
        },
    }


def print_human(report: dict[str, object]) -> None:
    missing = report["missing"]  # type: ignore[assignment]
    java = report["java"]  # type: ignore[assignment]
    rust = report["rust"]  # type: ignore[assignment]

    print(f"full_parity: {report['parity']}")
    print(
        "config_unpackers: "
        f"java={java['config_unpacker_count']} rust_supported={rust['supported_config_count']}"
    )
    print(
        "commands: "
        f"java_outputs={java['output_target_count']} rust_commands={rust['command_count']}"
    )
    print(f"missing_top_targets: {len(missing['top_targets'])}")  # type: ignore[index]
    for name in missing["top_targets"]:  # type: ignore[index]
        print(f"  - {name}")
    print(f"missing_config_features: {len(missing['configs'])}")  # type: ignore[index]
    for name in missing["configs"][:20]:  # type: ignore[index]
        print(f"  - {name}")
    if len(missing["configs"]) > 20:  # type: ignore[index]
        print(f"  ... +{len(missing['configs']) - 20} more")


def main() -> None:
    args = parse_args()
    report = audit(args.java_root, args.rust_root)
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print_human(report)


if __name__ == "__main__":
    main()
