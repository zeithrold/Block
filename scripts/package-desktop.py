#!/usr/bin/env python3
"""Package an existing Block binary without installing or changing the host."""

import argparse
import gzip
import io
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile


ROOT = Path(__file__).resolve().parents[1]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def unsupported_architectures(arches):
    return sorted(set(arches) - {"arm64", "amd64"})


def executable_target(binary):
    with binary.open("rb") as source:
        header = source.read(64)
        if header[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf",
                          b"\xbe\xba\xfe\xca", b"\xbf\xba\xfe\xca"):
            endian = ">" if header[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf") else "<"
            entry_size = 32 if header[:4] in (b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca") else 20
            count = struct.unpack_from(endian + "I", header, 4)[0]
            if not 1 <= count <= 8:
                raise ValueError("Unsupported Mach-O universal header")
            arches = []
            source.seek(8)
            for _ in range(count):
                entry = source.read(entry_size)
                if len(entry) < entry_size:
                    raise ValueError("Truncated Mach-O universal header")
                cputype = struct.unpack_from(endian + "I", entry, 0)[0]
                arches.append({0x0100000C: "arm64", 0x01000007: "amd64"}.get(cputype, "unknown"))
            if unsupported_architectures(arches):
                raise ValueError("Unsupported architecture in Mach-O universal binary")
            return "macos", "universal"
        if header[:4] == b"\xcf\xfa\xed\xfe":
            machine = struct.unpack_from("<I", header, 4)[0]
            return "macos", {0x0100000C: "arm64", 0x01000007: "amd64"}[machine]
        if header[:4] == b"\x7fELF" and header[4] == 2:
            endian = "<" if header[5] == 1 else ">"
            machine = struct.unpack_from(endian + "H", header, 18)[0]
            return "linux", {62: "amd64", 183: "arm64"}[machine]
        if header[:2] == b"MZ":
            source.seek(struct.unpack_from("<I", header, 60)[0])
            pe = source.read(6)
            if pe[:4] == b"PE\0\0":
                machine = struct.unpack_from("<H", pe, 4)[0]
                return "windows", {0x8664: "amd64", 0xAA64: "arm64"}[machine]
    raise ValueError("Expected a native 64-bit Mach-O, ELF, or PE executable")


def copy(source, destination, executable=False):
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")
    path.chmod(0o644)


def notices(destination):
    copy(ROOT / "LICENSE", destination / "LICENSE")
    for source, name in (
        ("crates/block-ui/GPUI-LICENSE-APACHE", "GPUI-LICENSE-APACHE"),
        ("packaging/licenses/GPUI-BASE-LICENSE-APACHE", "GPUI-BASE-LICENSE-APACHE"),
        ("crates/block-ui/assets/fonts/OFL.txt", "Inter-OFL.txt"),
        ("crates/block-ui/assets/icons/LICENSE", "Lucide-LICENSE"),
        ("packaging/licenses/SOURCES.md", "SOURCES.md"),
    ):
        copy(ROOT / source, destination / "licenses" / name)
    for source, name in (
        ("packaging/THIRD_PARTY_NOTICES.txt", "THIRD_PARTY_NOTICES.txt"),
        ("crates/block-ui/assets/provenance.json", "brand-provenance.json"),
    ):
        copy(ROOT / source, destination / name)


def make_icns(png, destination):
    if sys.platform != "darwin":
        raise ValueError("macOS ICNS conversion requires native sips and iconutil")
    with tempfile.TemporaryDirectory(prefix="block-icon-") as temporary:
        iconset = Path(temporary) / "Block.iconset"
        iconset.mkdir()
        for logical in (16, 32, 128, 256, 512):
            for scale in (1, 2):
                size = logical * scale
                suffix = "@2x" if scale == 2 else ""
                name = f"icon_{logical}x{logical}{suffix}.png"
                subprocess.run(
                    ["/usr/bin/sips", "-z", str(size), str(size), str(png),
                     "--out", str(iconset / name)],
                    check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )
        result = subprocess.run(
            ["/usr/bin/iconutil", "-c", "icns", str(iconset), "-o", str(destination)],
            check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        if result.returncode:
            # Some hosts reject iconset -> ICNS despite accepting the resulting
            # standard PNG chunks. Only wrap sips' existing bytes; do not edit
            # pixels or fall back to an enlarged low-resolution source.
            chunks = []
            for kind, size, name in (
                (b"icp4", 16, "icon_16x16.png"),
                (b"icp5", 32, "icon_32x32.png"),
                (b"icp6", 64, "icon_32x32@2x.png"),
                (b"ic07", 128, "icon_128x128.png"),
                (b"ic08", 256, "icon_256x256.png"),
                (b"ic09", 512, "icon_512x512.png"),
                (b"ic10", 1024, "icon_512x512@2x.png"),
            ):
                data = png.read_bytes() if size == 1024 else (iconset / name).read_bytes()
                chunks.append(kind + struct.pack(">I", len(data) + 8) + data)
            data = b"".join(chunks)
            destination.write_bytes(b"icns" + struct.pack(">I", len(data) + 8) + data)
    destination.chmod(0o644)


def macos(package, binary, product, version):
    contents = package / "Contents"
    resources = contents / "Resources"
    resources.mkdir(parents=True)
    copy(binary, contents / "MacOS" / product["executable"], executable=True)
    info = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": product["name"],
        "CFBundleExecutable": product["executable"],
        "CFBundleIconFile": "Block.icns",
        "CFBundleIdentifier": product["application_id"],
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": product["name"],
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": version.split("-", 1)[0].split("+", 1)[0],
        "CFBundleVersion": version.split("-", 1)[0].split("+", 1)[0],
        "LSApplicationCategoryType": "public.app-category.games",
        "NSHighResolutionCapable": True,
        "NSRequiresAquaSystemAppearance": False,
        "BlockWebsite": product["website"],
    }
    with (contents / "Info.plist").open("wb") as target:
        plistlib.dump(info, target, sort_keys=True)
    (contents / "Info.plist").chmod(0o644)
    write(contents / "PkgInfo", "APPL????")
    make_icns(ROOT / product["icon_macos_png"], resources / "Block.icns")
    notices(resources)


def linux(package, binary, product):
    app_id = product["application_id"]
    copy(binary, package / "bin" / product["executable"], executable=True)
    copy(ROOT / "packaging/linux/one.ztd.block.desktop.in",
         package / "share/applications" / f"{app_id}.desktop")
    png = ROOT / product["icon_png"]
    with png.open("rb") as source:
        header = source.read(24)
    width, height = struct.unpack_from(">II", header, 16)
    copy(png, package / f"share/icons/hicolor/{width}x{height}/apps/{app_id}.png")
    copy(ROOT / product["icon_svg"],
         package / f"share/icons/hicolor/scalable/apps/{app_id}.svg")
    notices(package / "share/doc" / app_id)


def windows(package, binary, product):
    copy(binary, package / f"{product['executable']}.exe", executable=True)
    copy(ROOT / product["icon_windows_ico"], package / "Block.ico")
    copy(ROOT / "packaging/windows/icon-provenance.json",
         package / "windows-icon-provenance.json")
    write(package / "Block website.url", f"[InternetShortcut]\nURL={product['website']}\n")
    notices(package)


def macos_dmg(package, destination, product, version):
    if sys.platform != "darwin":
        raise ValueError("DMG creation requires macOS hdiutil")
    with tempfile.TemporaryDirectory(prefix="block-dmg-") as temporary:
        stage = Path(temporary) / "stage"
        stage.mkdir()
        shutil.copytree(package, stage / package.name, symlinks=True)
        (stage / "Applications").symlink_to("/Applications")
        subprocess.run(
            ["hdiutil", "create", "-volname", f"{product['name']} {version}",
             "-srcfolder", str(stage), "-ov", "-format", "UDZO", str(destination)],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        )
    destination.chmod(0o644)


def tar_payload(entries, epoch):
    """Deterministic gzip-less tar payload for .deb members."""
    seen = set()
    directories = []
    for path, _, _ in sorted(entries):
        parts = path.strip("/").split("/")
        for index in range(1, len(parts)):
            directory = "/".join(parts[:index])
            if directory not in seen:
                seen.add(directory)
                directories.append(("./" + directory + "/", None))
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as target:
        root = tarfile.TarInfo("./")
        root.type = tarfile.DIRTYPE
        root.mode = 0o755
        root.mtime = epoch
        target.addfile(root)
        for name, _ in directories:
            info = tarfile.TarInfo(name)
            info.type = tarfile.DIRTYPE
            info.mode = 0o755
            info.mtime = epoch
            target.addfile(info)
        for path, mode, data in sorted(entries):
            info = tarfile.TarInfo("./" + path.strip("/"))
            info.mode = mode
            info.mtime = epoch
            info.size = len(data)
            target.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


def gzip_bytes(data, epoch):
    buffer = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=buffer, mtime=epoch) as compressed:
        compressed.write(data)
    return buffer.getvalue()


def ar_archive(members):
    output = bytearray(b"!<arch>\n")
    for name, data in members:
        header = (f"{name:<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n").encode("ascii")
        output += header
        output += data
        if len(data) % 2:
            output += b"\n"
    return bytes(output)


def linux_notices():
    entries = [("LICENSE", (ROOT / "LICENSE").read_bytes())]
    for source, name in (
        ("crates/block-ui/GPUI-LICENSE-APACHE", "licenses/GPUI-LICENSE-APACHE"),
        ("packaging/licenses/GPUI-BASE-LICENSE-APACHE", "licenses/GPUI-BASE-LICENSE-APACHE"),
        ("crates/block-ui/assets/fonts/OFL.txt", "licenses/Inter-OFL.txt"),
        ("crates/block-ui/assets/icons/LICENSE", "licenses/Lucide-LICENSE"),
        ("packaging/licenses/SOURCES.md", "licenses/SOURCES.md"),
        ("packaging/THIRD_PARTY_NOTICES.txt", "THIRD_PARTY_NOTICES.txt"),
        ("crates/block-ui/assets/provenance.json", "brand-provenance.json"),
    ):
        entries.append((name, (ROOT / source).read_bytes()))
    return entries


def debian(destination, binary, product, version, arch, epoch):
    """Create a deterministic .deb without dpkg tooling."""
    app_id = product["application_id"]
    png = ROOT / product["icon_png"]
    with png.open("rb") as source:
        header = source.read(24)
    width, height = struct.unpack_from(">II", header, 16)
    entries = [
        (f"usr/bin/{product['executable']}", 0o755, binary.read_bytes()),
        (f"usr/share/applications/{app_id}.desktop", 0o644,
         (ROOT / "packaging/linux/one.ztd.block.desktop.in").read_bytes()),
        (f"usr/share/icons/hicolor/{width}x{height}/apps/{app_id}.png", 0o644, png.read_bytes()),
        (f"usr/share/icons/hicolor/scalable/apps/{app_id}.svg", 0o644,
         (ROOT / product["icon_svg"]).read_bytes()),
    ]
    for name, data in linux_notices():
        entries.append((f"usr/share/doc/{app_id}/{name}", 0o644, data))
    md5sums = "\n".join(
        f"{hashlib.md5(data).hexdigest()}  {path}" for path, _, data in sorted(entries)
    ) + "\n"
    installed_size = max(1, (sum(len(data) for _, _, data in entries) + 1023) // 1024)
    deb_version = version.replace("-", "~", 1)
    control = "\n".join([
        f"Package: {app_id}",
        f"Version: {deb_version}",
        f"Architecture: {arch}",
        f"Maintainer: {product['maintainer']}",
        f"Installed-Size: {installed_size}",
        "Section: games",
        "Priority: optional",
        f"Homepage: {product['website']}",
        "Depends: libc6, libfontconfig1, libfreetype6, libx11-xcb1, libxcb1, libxcb-shape0, "
        "libxcb-xfixes0, libxkbcommon0, libwayland-client0, libegl1, libgl1, libvulkan1, "
        "libasound2 | libasound2t64",
        f"Description: {product['name']} - {product['description']}",
        f" {product['name']} is a native launcher for Minecraft Java Edition.",
        "",
    ])
    control_tar = tar_payload([
        ("control", 0o644, control.encode("utf-8")),
        ("md5sums", 0o644, md5sums.encode("utf-8")),
    ], epoch)
    data_tar = tar_payload(entries, epoch)
    destination.write_bytes(ar_archive([
        ("debian-binary", b"2.0\n"),
        ("control.tar.gz", gzip_bytes(control_tar, epoch)),
        ("data.tar.gz", gzip_bytes(data_tar, epoch)),
    ]))
    destination.chmod(0o644)


def linux_appdir(appdir, binary, product):
    app_id = product["application_id"]
    appdir.mkdir(parents=True)
    copy(binary, appdir / product["executable"], executable=True)
    app_run = appdir / "AppRun"
    write(app_run, '#!/bin/sh\nexec "$APPDIR/' + product["executable"] + '" "$@"\n')
    app_run.chmod(0o755)
    copy(ROOT / "packaging/linux/one.ztd.block.desktop.in", appdir / f"{app_id}.desktop")
    png = ROOT / product["icon_png"]
    copy(png, appdir / f"{app_id}.png")
    copy(png, appdir / ".DirIcon")


def appimage(appdir, destination, tool, arch):
    if not tool.is_file():
        raise ValueError(f"AppImage tool not found: {tool}")
    # Resolve relative paths: executing a bare filename such as "appimagetool"
    # makes the OS search PATH instead of the current directory.
    tool = tool.resolve(strict=True)
    environment = dict(
        os.environ,
        ARCH={"amd64": "x86_64", "arm64": "aarch64"}[arch],
        APPIMAGE_EXTRACT_AND_RUN="1",
    )
    subprocess.run([str(tool), str(appdir), str(destination)], check=True, env=environment)
    destination.chmod(0o755)


def archive(package, destination, epoch):
    members = [package, *sorted(package.rglob("*"))]
    if destination.suffix == ".zip":
        from datetime import datetime, timezone
        timestamp = datetime.fromtimestamp(max(epoch, 315532800), timezone.utc)
        with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED,
                             compresslevel=9) as target:
            for path in members:
                name = path.relative_to(package.parent).as_posix()
                info = zipfile.ZipInfo(name + ("/" if path.is_dir() else ""),
                                       timestamp.timetuple()[:6])
                info.create_system = 3
                mode = 0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644
                info.external_attr = ((stat.S_IFDIR if path.is_dir() else stat.S_IFREG) | mode) << 16
                if path.is_dir():
                    info.external_attr |= 0x10
                info.compress_type = zipfile.ZIP_DEFLATED
                target.writestr(info, b"" if path.is_dir() else path.read_bytes(), compresslevel=9)
    else:
        with destination.open("xb") as output:
            with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=epoch) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as target:
                    for path in members:
                        name = path.relative_to(package.parent).as_posix()
                        info = tarfile.TarInfo(name)
                        info.mtime = epoch
                        info.mode = 0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644
                        if path.is_dir():
                            info.type = tarfile.DIRTYPE
                            target.addfile(info)
                        else:
                            info.size = path.stat().st_size
                            with path.open("rb") as source:
                                target.addfile(info, source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path,
                        default=ROOT / "target/debug" / ("block.exe" if os.name == "nt" else "block"))
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--platform", choices=("macos", "linux", "windows"))
    parser.add_argument("--arch", choices=("arm64", "amd64", "universal"))
    parser.add_argument("--version")
    parser.add_argument("--appimage-tool", type=Path,
                        help="appimagetool (or compatible) used to build a Linux AppImage")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    binary_digest = sha256(binary)
    target, arch = executable_target(binary)
    if args.platform and args.platform != target or args.arch and args.arch != arch:
        raise ValueError(f"Executable is {target}/{arch}; target assertions do not match")
    product = json.loads((ROOT / "packaging/product.json").read_text())
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    version = args.version or cargo["workspace"]["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:[-+][A-Za-z0-9.+-]+)?", version):
        raise ValueError("Expected a semantic version without filesystem separators")
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    if not 0 <= epoch <= 0xFFFFFFFF:
        raise ValueError("SOURCE_DATE_EPOCH must be an unsigned 32-bit Unix timestamp")
    output = args.output_dir.resolve()
    stem = f"{product['name']}-{version}-{target}-{arch}"
    package = output / (f"{product['name']}.app" if target == "macos" else stem)
    bundle_archive = output / (stem + (".tar.gz" if target == "linux" else ".zip"))
    record = output / (stem + ".manifest.json")
    installers = []
    planned = [package, bundle_archive, record]
    if target == "macos":
        installers.append(output / (stem + ".dmg"))
    elif target == "linux":
        installers.append(output / (stem + ".deb"))
        if args.appimage_tool:
            installers.append(output / (stem + ".AppImage"))
    for path in planned + installers:
        if path.exists() or path.is_symlink():
            raise ValueError(f"Refusing to overwrite {path}")
    output.mkdir(parents=True, exist_ok=True)
    if target == "macos":
        macos(package, binary, product, version)
        macos_dmg(package, installers[0], product, version)
    elif target == "linux":
        linux(package, binary, product)
        debian(installers[0], binary, product, version, arch, epoch)
        if args.appimage_tool:
            with tempfile.TemporaryDirectory(prefix="block-appdir-") as temporary:
                appdir = Path(temporary) / f"{product['name']}.AppDir"
                linux_appdir(appdir, binary, product)
                appimage(appdir, installers[-1], args.appimage_tool, arch)
    else:
        package.mkdir()
        windows(package, binary, product)
    executable = {
        "macos": package / "Contents/MacOS" / product["executable"],
        "linux": package / "bin" / product["executable"],
        "windows": package / f"{product['executable']}.exe",
    }[target]
    if sha256(executable) != binary_digest or sha256(binary) != binary_digest:
        raise ValueError("The input executable changed during packaging; use a stable build")
    files = []
    for path in sorted(package.rglob("*")):
        if path.is_file():
            files.append({"path": path.relative_to(output).as_posix(),
                          "sha256": sha256(path), "size": path.stat().st_size,
                          "mode": oct(stat.S_IMODE(path.stat().st_mode))})
    archive(package, bundle_archive, epoch)
    manifests = []
    for path in installers:
        manifests.append({"name": path.name, "sha256": sha256(path),
                          "size": path.stat().st_size})
    manifest = {
        "product": product,
        "version": version,
        "platform": target,
        "architecture": arch,
        "source_date_epoch": epoch,
        "binary_sha256": binary_digest,
        "files": files,
        "archive": {"name": bundle_archive.name, "sha256": sha256(bundle_archive)},
        "installers": manifests,
        "signing": "No distribution signing or notarization performed",
    }
    write(record, json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(json.dumps({
        "package": str(package),
        "archive": str(bundle_archive),
        "manifest": str(record),
        "installers": [str(path) for path in installers],
        "sha256": manifest["archive"]["sha256"],
    }, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, struct.error, subprocess.CalledProcessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        sys.exit(1)
