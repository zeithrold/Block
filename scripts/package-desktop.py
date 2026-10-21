#!/usr/bin/env python3
"""Package an existing Block binary without installing or changing the host."""

import argparse
import gzip
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


def executable_target(binary):
    with binary.open("rb") as source:
        header = source.read(64)
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
    for source, name in (
        ("crates/block-ui/GPUI-LICENSE-APACHE", "GPUI-LICENSE-APACHE"),
        ("vendor/xattr/LICENSE-MIT", "xattr-LICENSE-MIT"),
        ("vendor/xattr/LICENSE-APACHE", "xattr-LICENSE-APACHE"),
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
    png = (ROOT / product["icon_png"]).read_bytes()
    # ICO's zero width/height denote the existing 256 px PNG representation.
    ico = struct.pack("<HHH", 0, 1, 1)
    ico += struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(png), 22)
    (package / "Block.ico").write_bytes(ico + png)
    (package / "Block.ico").chmod(0o644)
    write(package / "Block website.url", f"[InternetShortcut]\nURL={product['website']}\n")
    notices(package)


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
    parser.add_argument("--arch", choices=("arm64", "amd64"))
    parser.add_argument("--version")
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
    for path in (package, bundle_archive, record):
        if path.exists() or path.is_symlink():
            raise ValueError(f"Refusing to overwrite {path}")
    output.mkdir(parents=True, exist_ok=True)
    if target == "macos":
        macos(package, binary, product, version)
    elif target == "linux":
        linux(package, binary, product)
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
    manifest = {"product": product, "version": version, "platform": target,
                "architecture": arch, "source_date_epoch": epoch,
                "binary_sha256": binary_digest, "files": files,
                "archive": {"name": bundle_archive.name, "sha256": sha256(bundle_archive)},
                "signing": "No distribution signing or notarization performed"}
    write(record, json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"package": str(package), "archive": str(bundle_archive),
                      "manifest": str(record), "sha256": manifest["archive"]["sha256"]}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, struct.error, subprocess.CalledProcessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        sys.exit(1)
