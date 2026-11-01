#!/usr/bin/env python3
"""Generate synthetic modpack and plain Minecraft ZIP fixtures.

The fixtures are original and contain no credentials or third-party content.
Modrinth payloads are served from a loopback HTTP server that the operator
starts separately; the packs only declare http://127.0.0.1:<port>/ URLs. The
fixtures never make an installation Ready or change Block's catalog by
themselves: Block still installs Minecraft 1.20.1 through its normal official
metadata path.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shlex
import zipfile


DEFAULT_PORT = 8123
CANCEL_BYTES = 1024 * 1024
HTTP_ROOT = "http"
RETRY_SOURCE = "retry-source"
VERSION_JSON = {
    "id": "1.20.1",
    "type": "release",
    "releaseTime": "2023-06-12T00:00:00+00:00",
    "mainClass": "net.minecraft.client.main.Main",
}
LAUNCHER_ACCOUNTS = (
    b'{"accounts": {}, "activeAccount": null, "activeProfiles": [], '
    b'"syntheticFixture": true, "note": "Non-secret marker; never copied."}\n'
)
LOG_LINE = b"[00:00:00] [main/INFO]: Synthetic fixture log. No game was run.\n"


def payload(name: str, size: int) -> bytes:
    """Inert bytes of an exact size, recognizable as a synthetic fixture."""
    header = f"Block synthetic modpack fixture: {name}\n".encode()
    if size < len(header):
        raise ValueError(f"fixture payload {name} is too small")
    filler = b"Synthetic fixture bytes. This is not executable Java code.\n"
    return (header + filler * ((size - len(header)) // len(filler) + 1))[:size]


def json_bytes(document: object) -> bytes:
    return json.dumps(document, indent=2).encode() + b"\n"


def write_zip(path: Path, entries: list[tuple[str, bytes]]) -> None:
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in entries:
            archive.writestr(name, data)


def modrinth_entry(
    path: str, data: bytes, downloads: list[str], client: str
) -> dict[str, object]:
    return {
        "path": path,
        "hashes": {
            "sha1": hashlib.sha1(data).hexdigest(),
            "sha512": hashlib.sha512(data).hexdigest(),
        },
        "fileSize": len(data),
        "downloads": downloads,
        "env": {"client": client},
    }


def modrinth_index(name: str, files: list[dict[str, object]]) -> dict[str, object]:
    return {
        "formatVersion": 1,
        "game": "minecraft",
        "name": name,
        "versionId": "1.0.0",
        "dependencies": {"minecraft": "1.20.1"},
        "files": files,
    }


def plain_game_data(prefix: str, with_version: bool = True) -> list[tuple[str, bytes]]:
    """Game-data markers at one candidate root, plus exclusion markers."""
    entries = [
        (prefix + "options.txt", b"version:1.20.1\n"),
        (prefix + "config/example.txt", b"synthetic configuration\n"),
        (prefix + "mods/example.jar", b"synthetic mod bytes; not executable\n"),
        (prefix + "saves/Synthetic World/level.dat", b"synthetic world marker\n"),
        (prefix + "logs/latest.log", LOG_LINE),
        (prefix + "launcher_accounts.json", LAUNCHER_ACCOUNTS),
    ]
    if with_version:
        entries.append(
            (prefix + "versions/1.20.1/1.20.1.json", json_bytes(VERSION_JSON))
        )
    return entries


def generate(out: Path, port: int) -> None:
    http = out / HTTP_ROOT
    retry_source = out / RETRY_SOURCE
    http.mkdir()
    retry_source.mkdir()

    required = payload("required.jar", 4096)
    optional = payload("optional.jar", 1536)
    unsupported = payload("client-unsupported.jar", 1024)
    retry = payload("retry.jar", 2048)
    cancel = payload("cancel.jar", CANCEL_BYTES)

    for name, data in [
        ("required.jar", required),
        ("optional.jar", optional),
        ("client-unsupported.jar", unsupported),
        ("cancel.jar", cancel),
    ]:
        (http / name).write_bytes(data)
    (retry_source / "retry.jar").write_bytes(retry)

    base = f"http://127.0.0.1:{port}"

    write_zip(
        out / "required-optional.mrpack",
        [
            (
                "modrinth.index.json",
                json_bytes(
                    modrinth_index(
                        "Block Required and Optional Fixture",
                        [
                            modrinth_entry(
                                "mods/fixture-required.jar",
                                required,
                                [f"{base}/missing/required.jar", f"{base}/required.jar"],
                                "required",
                            ),
                            modrinth_entry(
                                "mods/fixture-optional.jar",
                                optional,
                                [f"{base}/optional.jar"],
                                "optional",
                            ),
                            modrinth_entry(
                                "mods/fixture-client-unsupported.jar",
                                unsupported,
                                [f"{base}/client-unsupported.jar"],
                                "unsupported",
                            ),
                        ],
                    )
                ),
            ),
            ("overrides/config/block-fixture.cfg", b"layer=overrides\n"),
            (
                "client-overrides/config/block-fixture.cfg",
                b"layer=client-overrides\n",
            ),
            (
                "server-overrides/config/block-fixture.cfg",
                b"layer=server-overrides\n",
            ),
            (
                "overrides/kubejs/scripts/block-fixture.js",
                b"// Synthetic KubeJS fixture copied by Block; no script runs from it.\n",
            ),
        ],
    )

    write_zip(
        out / "long-name.mrpack",
        [
            ("modrinth.index.json", json_bytes(modrinth_index("模" * 64, []))),
            ("overrides/config/block-fixture.cfg", b"source=long-name\n"),
        ],
    )

    write_zip(
        out / "retry.mrpack",
        [
            (
                "modrinth.index.json",
                json_bytes(
                    modrinth_index(
                        "Block Retry Fixture",
                        [
                            modrinth_entry(
                                "mods/fixture-retry.jar",
                                retry,
                                [f"{base}/retry.jar"],
                                "required",
                            )
                        ],
                    )
                ),
            )
        ],
    )

    write_zip(
        out / "cancel.mrpack",
        [
            (
                "modrinth.index.json",
                json_bytes(
                    modrinth_index(
                        "Block Cancel Fixture",
                        [
                            modrinth_entry(
                                "mods/fixture-cancel.jar",
                                cancel,
                                [f"{base}/cancel.jar"],
                                "required",
                            )
                        ],
                    )
                ),
            )
        ],
    )

    curseforge_manifest = {
        "manifestType": "minecraftModpack",
        "manifestVersion": 1,
        "name": "Block CurseForge Blocked Fixture",
        "version": "1.0.0",
        "minecraft": {"version": "1.20.1", "modLoaders": []},
        "files": [
            {"projectID": 990001, "fileID": 9900011, "required": True},
            {"projectID": 990002, "fileID": 9900021, "required": False},
        ],
        "overrides": "overrides",
    }
    write_zip(
        out / "curseforge-blocked.zip",
        [
            ("manifest.json", json_bytes(curseforge_manifest)),
            ("overrides/config/block-fixture.cfg", b"source=curseforge-overrides\n"),
        ],
    )

    prism_components = [
        {"uid": "net.minecraft", "version": "1.20.1"},
        {"uid": "org.lwjgl3", "version": "3.3.1"},
    ]
    prism_cfg = b"[General]\nname=Block Prism Valid Fixture\nInstanceType=OneSix\n"
    prism_content = [
        ("minecraft/options.txt", b"version:1.20.1\n"),
        ("minecraft/config/block-fixture.cfg", b"source=prism-content\n"),
    ]
    write_zip(
        out / "prism-valid.zip",
        [
            (
                "mmc-pack.json",
                json_bytes({"formatVersion": 1, "components": prism_components}),
            ),
            ("instance.cfg", prism_cfg),
            *prism_content,
        ],
    )

    unsupported_components = [
        *prism_components,
        {"uid": "org.multimc.patch.block-fixture", "version": "1.0.0"},
    ]
    write_zip(
        out / "prism-unsupported.zip",
        [
            (
                "mmc-pack.json",
                json_bytes(
                    {"formatVersion": 1, "components": unsupported_components}
                ),
            ),
            (
                "instance.cfg",
                b"[General]\nname=Block Prism Unsupported Fixture\nInstanceType=OneSix\n",
            ),
            *prism_content,
        ],
    )

    write_zip(out / "plain-root.zip", plain_game_data(""))
    write_zip(out / "plain-dotminecraft.zip", plain_game_data(".minecraft/"))
    write_zip(out / "plain-wrapper.zip", plain_game_data("MinecraftBackup/"))
    write_zip(
        out / "plain-ambiguous.zip",
        plain_game_data("AlphaGame/") + plain_game_data("BetaGame/"),
    )
    write_zip(
        out / "plain-unknown-version.zip", plain_game_data("", with_version=False)
    )

    malformed_index = {
        "formatVersion": 1,
        "game": "minecraft",
        "name": "Block Malformed Modrinth Fixture",
        "versionId": "1.0.0",
        "files": [],
    }
    write_zip(
        out / "plain-bad-standard.zip",
        [
            ("modrinth.index.json", json_bytes(malformed_index)),
            *plain_game_data(""),
        ],
    )


def explain(out: Path, port: int) -> None:
    http = out / HTTP_ROOT
    quoted_http = shlex.quote(str(http))
    print(f"Generated synthetic modpack and plain ZIP fixtures in {out}")
    print()
    print("Start the payload server in a separate terminal (loopback only):")
    print(
        f"  python3 -m http.server {port} --bind 127.0.0.1 --directory {quoted_http}"
    )
    print("Keep it running while Block downloads, then stop it with Ctrl+C.")
    print(
        f"The Modrinth packs declare http://127.0.0.1:{port}/... URLs. Regenerate "
        "them after changing the port."
    )
    print(
        "The server is separate from Block; if it is not running, Block reports a "
        "failed download."
    )
    print(
        "Minecraft 1.20.1 is declared as vanilla. Block still installs the game and "
        "any loader through its normal official metadata path."
    )
    print()
    print("Modpack fixtures")
    print(f"  {out / 'required-optional.mrpack'}")
    print(
        "    The required file declares .../missing/required.jar first (404) and "
        ".../required.jar as the fallback, so the download succeeds from the second "
        "URL. The optional file can be toggled; the client=unsupported file is never "
        "installed. After import, config/block-fixture.cfg must read "
        "\"layer=client-overrides\": client-overrides applies after overrides, and "
        "server-overrides is never applied. kubejs/scripts/block-fixture.js is "
        "copied verbatim and contains comments only."
    )
    print(f"  {out / 'long-name.mrpack'}")
    print(
        "    The pack name is 64 Chinese characters (192 UTF-8 bytes). The review "
        "must show the name validation, allow editing the instance name, and retain "
        "the original pack name in its descriptor."
    )
    print(f"  {out / 'retry.mrpack'}")
    print(
        "    http/retry.jar is intentionally absent. Start the import and let the "
        "payload fail, then copy the supplied payload and press Retry, selecting "
        "this same pack again:"
    )
    print(
        f"      cp {shlex.quote(str(out / RETRY_SOURCE / 'retry.jar'))} "
        f"{shlex.quote(str(http / 'retry.jar'))}"
    )
    print(
        "    The declared URL, size and hashes stay the same, so no new modpack "
        "catalog entry is needed."
    )
    print(f"  {out / 'cancel.mrpack'}")
    print(
        "    One required ~1 MiB payload. Lower Settings -> Downloads -> \"Download "
        "speed limit (MB/s)\" (for example 0.01), start the import, and press Stop "
        "while the payload downloads. No additional server harness is involved."
    )
    print(f"  {out / 'curseforge-blocked.zip'}")
    print(
        "    Standard CurseForge manifest v1 with no hashes, URLs or API data. The "
        "required project/file ID entry (990001/9900011) cannot be verified, so the "
        "review is visibly blocked; the optional ID-only entry (990002/9900021) is "
        "listed but does not make it completable."
    )
    print(f"  {out / 'prism-valid.zip'}")
    print(
        "    Standard Prism instance: [General] instance.cfg name, mmc-pack.json with "
        "Minecraft 1.20.1 and an LWJGL helper component, and minecraft/ options and "
        "config content. It defines no pre-launch, post-exit or wrapper command."
    )
    print(f"  {out / 'prism-unsupported.zip'}")
    print(
        "    The same instance plus the unknown component "
        "\"org.multimc.patch.block-fixture\" (a synthetic custom patch) in "
        "mmc-pack.json. Inspection must reject it as unsupported."
    )
    print()
    print("Plain Minecraft ZIP fixtures (detection only)")
    print(f"  {out / 'plain-root.zip'}")
    print(
        "    Game data at the archive root: options.txt, config/, mods/, saves/, and "
        "versions/1.20.1/1.20.1.json for version detection. logs/latest.log and "
        "launcher_accounts.json are inert exclusion markers. Block should recognize "
        "Minecraft 1.20.1, require an explicit loader choice because mods are "
        "present, and not copy versions/ or the exclusion markers."
    )
    print(f"  {out / 'plain-dotminecraft.zip'}")
    print(
        "    The same game data under .minecraft/. Block should detect .minecraft/ "
        "as the game root."
    )
    print(f"  {out / 'plain-wrapper.zip'}")
    print(
        "    The same game data under one wrapper directory, MinecraftBackup/. Block "
        "should detect that single wrapper as the game root."
    )
    print(f"  {out / 'plain-ambiguous.zip'}")
    print(
        "    Two independent candidate game roots, AlphaGame/ and BetaGame/, each "
        "with valid game data. Detection must reject the archive as ambiguous."
    )
    print(f"  {out / 'plain-unknown-version.zip'}")
    print(
        "    mods/ and config/ markers without any versions/ metadata. It is still "
        "plain game data, but no Minecraft version can be inferred."
    )
    print(f"  {out / 'plain-bad-standard.zip'}")
    print(
        "    A recognized modrinth.index.json that parses as JSON but is not a valid "
        "pack (it has no dependencies), plus valid game data at the root. Inspection "
        "must reject the malformed Modrinth pack and must not fall back to Plain."
    )
    print()
    print(
        "Paths and URLs contain no credentials or third-party content. The HTTP "
        "server binds only 127.0.0.1 and the operator must stop it."
    )
    print(
        "These are test inputs only: they do not mark an installation Ready or change "
        "the catalog by themselves."
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "output_directory",
        help="new or empty directory that receives the fixtures",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=DEFAULT_PORT,
        help=f"loopback HTTP port declared in the packs (default: {DEFAULT_PORT})",
    )
    args = parser.parse_args()
    if not 1 <= args.port <= 65535:
        parser.error("--port must be between 1 and 65535")
    out = Path(args.output_directory).expanduser().resolve()
    if out.exists():
        if not out.is_dir():
            parser.error(f"output path exists and is not a directory: {out}")
        if any(out.iterdir()):
            parser.error(f"output directory is not empty: {out}")
    else:
        out.mkdir(parents=True)
    generate(out, args.port)
    explain(out, args.port)


if __name__ == "__main__":
    main()
