//! Bounded parser for the Modrinth `.mrpack` v1 `modrinth.index.json`.
//!
//! Only the declared identity, loader pin and files are interpreted here. Archive
//! validation, destination safety beyond the shared path rule, and extraction
//! belong to the parent module. The Minecraft pin is accepted as one bounded
//! portable component; whether the game ships that version, and which platforms
//! support it, is left to the version metadata lookup. Download URLs stay
//! transient: errors and `Debug` output never repeat them.

use std::collections::HashSet;

use anyhow::{Context as _, bail, ensure};
use serde_json::{Map, Value};

use super::{PackContent, PackFile, PackFormat, PackInspection};
use crate::catalog::LoaderKind;

/// A pack names one file per mod, resource pack or config; this ceiling keeps a
/// malformed index from being expanded without bound.
const MAX_FILES: usize = 65536;
const MAX_LABEL_CHARS: usize = 128;
/// The same bound the launcher applies to a stored version pin, so a pack cannot
/// declare one the catalog would refuse.
const MAX_MINECRAFT_VERSION_BYTES: usize = 80;
const SHA1_HEX: usize = 40;
const SHA512_HEX: usize = 128;

/// Inspect an unpacked Modrinth v1 index without touching the archive it came from.
///
/// # Errors
/// Rejects other formats, missing or invalid identity, an absent or unsafe
/// Minecraft pin, inconsistent or ambiguous loader pins, malformed
/// hashes/downloads and conflicting destinations.
pub(super) fn parse(bytes: &[u8]) -> anyhow::Result<PackInspection> {
    let index: Value = serde_json::from_slice(bytes).context("Invalid Modrinth pack index")?;
    ensure!(
        index.get("formatVersion").and_then(Value::as_u64) == Some(1),
        "Unsupported Modrinth pack format version"
    );
    ensure!(
        index.get("game").and_then(Value::as_str) == Some("minecraft"),
        "Modrinth pack is not a Minecraft pack"
    );
    let name = label(index.get("name"), "name")?;
    let version = label(index.get("versionId"), "version id")?;
    let (minecraft_version, loader) = dependencies(&index)?;
    let files = files(&index)?;
    Ok(PackInspection {
        format: PackFormat::Modrinth,
        name,
        version: Some(version),
        minecraft_version: Some(minecraft_version),
        loader,
        target_evidence: super::TargetEvidence::Manifest,
        has_mods: false,
        files,
        missing_files: Vec::new(),
        content: PackContent::Modrinth,
        content_entries: Vec::new(),
        archive_prefix: String::new(),
    })
}

fn label(value: Option<&Value>, field: &str) -> anyhow::Result<String> {
    let value = value
        .and_then(Value::as_str)
        .with_context(|| format!("Modrinth pack has no {field}"))?;
    ensure!(
        !value.trim().is_empty()
            && value.chars().count() <= MAX_LABEL_CHARS
            && !value.chars().any(char::is_control),
        "Modrinth pack has an invalid {field}"
    );
    Ok(value.to_owned())
}

fn dependencies(index: &Value) -> anyhow::Result<(String, Option<(LoaderKind, String)>)> {
    let dependencies = index
        .get("dependencies")
        .and_then(Value::as_object)
        .context("Modrinth pack has no dependency list")?;
    let minecraft = dependencies
        .get("minecraft")
        .and_then(Value::as_str)
        .context("Modrinth pack does not declare its Minecraft version")?;
    validate_minecraft_version(minecraft)?;
    let mut loader = None;
    for (name, value) in dependencies {
        let kind = match name.as_str() {
            "minecraft" => continue,
            "fabric-loader" => LoaderKind::Fabric,
            "quilt-loader" => LoaderKind::Quilt,
            "forge" => LoaderKind::Forge,
            "neoforge" => LoaderKind::NeoForge,
            _ => bail!("Modrinth pack depends on an unsupported loader"),
        };
        ensure!(
            loader.is_none(),
            "Modrinth pack declares more than one loader"
        );
        let declared = value
            .as_str()
            .filter(|version| !version.is_empty())
            .context("Modrinth pack loader dependency has no build")?;
        let build = loader_build(&kind, minecraft, declared)?;
        loader = Some((kind, build));
    }
    Ok((minecraft.to_owned(), loader))
}

/// A game pin must be one bounded portable path component. The version metadata
/// lookup decides whether it names an existing game; loader pins are validated
/// separately.
fn validate_minecraft_version(version: &str) -> anyhow::Result<()> {
    ensure!(
        version.len() <= MAX_MINECRAFT_VERSION_BYTES && !version.contains('/'),
        "Modrinth pack has an invalid Minecraft version"
    );
    super::validate_pack_path(version).context("Modrinth pack has an invalid Minecraft version")
}

/// Resolve a declared build into the catalog pin the installers expect.
fn loader_build(kind: &LoaderKind, minecraft: &str, declared: &str) -> anyhow::Result<String> {
    match kind {
        LoaderKind::Forge => {
            let build = game_prefixed(minecraft, declared);
            crate::install::forge::validate_version(minecraft, &build)
                .context("Modrinth pack Forge build does not match its Minecraft version")?;
            Ok(build)
        }
        LoaderKind::NeoForge => {
            let build = if minecraft == "1.20.1" {
                game_prefixed(minecraft, declared)
            } else {
                declared.to_owned()
            };
            crate::install::neoforge::validate_version(minecraft, &build)
                .context("Modrinth pack NeoForge build does not match its Minecraft version")?;
            Ok(build)
        }
        LoaderKind::Fabric | LoaderKind::Quilt | LoaderKind::Vanilla => {
            crate::install::fabric::validate_version(declared)
                .context("Modrinth pack loader build is invalid")?;
            Ok(declared.to_owned())
        }
    }
}

/// Forge builds, and 1.20.1 NeoForge builds, carry the game prefix in official metadata.
fn game_prefixed(minecraft: &str, declared: &str) -> String {
    let already_prefixed = declared
        .strip_prefix(minecraft)
        .is_some_and(|rest| rest.starts_with('-'));
    if already_prefixed {
        declared.to_owned()
    } else {
        format!("{minecraft}-{declared}")
    }
}

fn files(index: &Value) -> anyhow::Result<Vec<PackFile>> {
    let declared = match index.get("files") {
        None => return Ok(Vec::new()),
        Some(Value::Array(files)) => files,
        Some(_) => bail!("Modrinth pack file list is not a list"),
    };
    ensure!(
        declared.len() <= MAX_FILES,
        "Modrinth pack declares too many files"
    );
    let mut files = Vec::with_capacity(declared.len());
    let mut destinations = Vec::with_capacity(declared.len());
    for entry in declared {
        ensure!(entry.is_object(), "Modrinth pack file is not an object");
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .context("Modrinth pack file has no destination path")?;
        super::validate_pack_path(path)
            .context("Modrinth pack file has an invalid destination path")?;
        let client = environment(entry)?;
        let (sha1, sha512) = hashes(entry)?;
        let size = entry
            .get("fileSize")
            .and_then(Value::as_u64)
            .context("Modrinth pack file has no size")?;
        let urls = downloads(entry)?;
        if let Some(required) = client {
            destinations.push(destination_key(path));
            files.push(PackFile {
                path: path.to_owned(),
                sha1,
                sha512,
                size,
                urls,
                required,
            });
        }
    }
    ensure_unique_destinations(&destinations)?;
    Ok(files)
}

/// Case-insensitive portable form used only to detect ambiguous destinations.
fn destination_key(path: &str) -> String {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

fn ensure_unique_destinations(destinations: &[String]) -> anyhow::Result<()> {
    let mut declared = HashSet::new();
    for destination in destinations {
        ensure!(
            declared.insert(destination.as_str()),
            "Modrinth pack declares the same destination twice"
        );
    }
    for destination in destinations {
        let mut prefix = destination.as_str();
        while let Some((parent, _)) = prefix.rsplit_once('/') {
            ensure!(
                !declared.contains(parent),
                "Modrinth pack declares a file where another file needs a directory"
            );
            prefix = parent;
        }
    }
    Ok(())
}

/// Map the client environment to the launcher's required/optional selection.
///
/// `None` means the file is not installed locally. The server value never changes
/// local installation, but it must still be a recognized environment.
fn environment(entry: &Value) -> anyhow::Result<Option<bool>> {
    let environment = match entry.get("env") {
        None | Some(Value::Null) => None,
        Some(Value::Object(environment)) => Some(environment),
        Some(_) => bail!("Modrinth pack file environment is not an object"),
    };
    let client = side(
        environment.and_then(|environment| environment.get("client")),
        "client",
    )?;
    let _ = side(
        environment.and_then(|environment| environment.get("server")),
        "server",
    )?;
    Ok(match client {
        Side::Required => Some(true),
        Side::Optional => Some(false),
        Side::Unsupported => None,
    })
}

#[derive(Clone, Copy)]
enum Side {
    Required,
    Optional,
    Unsupported,
}

fn side(value: Option<&Value>, field: &str) -> anyhow::Result<Side> {
    if value.is_none_or(Value::is_null) {
        return Ok(Side::Required);
    }
    match value.and_then(Value::as_str) {
        Some("required") => Ok(Side::Required),
        Some("optional") => Ok(Side::Optional),
        Some("unsupported") => Ok(Side::Unsupported),
        _ => bail!("Modrinth pack file has an unrecognized {field} environment"),
    }
}

fn hashes(entry: &Value) -> anyhow::Result<(String, String)> {
    let hashes = entry
        .get("hashes")
        .and_then(Value::as_object)
        .context("Modrinth pack file has no hashes")?;
    Ok((
        hash(hashes, "sha1", SHA1_HEX)?,
        hash(hashes, "sha512", SHA512_HEX)?,
    ))
}

fn hash(hashes: &Map<String, Value>, name: &str, digits: usize) -> anyhow::Result<String> {
    let digest = hashes
        .get(name)
        .and_then(Value::as_str)
        .with_context(|| format!("Modrinth pack file has no {name} hash"))?;
    ensure!(
        digest.len() == digits && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Modrinth pack file has an invalid {name} hash"
    );
    Ok(digest.to_ascii_lowercase())
}

fn downloads(entry: &Value) -> anyhow::Result<Vec<String>> {
    let declared = match entry.get("downloads") {
        Some(Value::Array(downloads)) if !downloads.is_empty() => downloads,
        _ => bail!("Modrinth pack file has no download URL"),
    };
    let mut urls = Vec::with_capacity(declared.len());
    for value in declared {
        let declared = value
            .as_str()
            .context("Modrinth pack file has an invalid download URL")?;
        let url = reqwest::Url::parse(declared)
            .map_err(|_| anyhow::anyhow!("Modrinth pack file has an invalid download URL"))?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "Modrinth pack file download URL must use http or https"
        );
        urls.push(declared.to_owned());
    }
    Ok(urls)
}

#[cfg(test)]
mod tests {
    use anyhow::bail;
    use serde_json::{Value, json};

    use super::super::{PackContent, PackFormat};
    use super::{MAX_MINECRAFT_VERSION_BYTES, parse};
    use crate::catalog::LoaderKind;

    fn sha1() -> String {
        "ab".repeat(20)
    }

    fn sha512() -> String {
        "cd".repeat(64)
    }

    fn encoded(index: Value) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(&index)?)
    }

    /// A file declaration with the hashes, size and downloads a real pack carries.
    fn file_entry(path: &str, downloads: Value) -> Value {
        json!({
            "path": path,
            "hashes": {"sha1": sha1(), "sha512": sha512()},
            "fileSize": 1024,
            "downloads": downloads,
        })
    }

    fn pack_index(dependencies: Value, files: Value) -> Value {
        json!({
            "formatVersion": 1,
            "game": "minecraft",
            "name": "Example Pack",
            "versionId": "1.0.0",
            "dependencies": dependencies,
            "files": files,
        })
    }

    fn parse_files(files: Value) -> anyhow::Result<Vec<(String, bool)>> {
        let bytes = encoded(pack_index(json!({"minecraft": "1.20.1"}), files))?;
        let inspection = parse(&bytes)?;
        Ok(inspection
            .files
            .into_iter()
            .map(|file| (file.path, file.required))
            .collect())
    }

    fn parse_loader(dependencies: Value) -> anyhow::Result<Option<(LoaderKind, String)>> {
        let bytes = encoded(pack_index(dependencies, json!([])))?;
        Ok(parse(&bytes)?.loader)
    }

    #[test]
    fn accepts_official_historical_minecraft_ids() -> anyhow::Result<()> {
        for version in ["b1.7.3", "a1.2.6", "1.14 Pre-Release 1"] {
            let bytes = encoded(pack_index(json!({"minecraft": version}), json!([])))?;
            let inspection = parse(&bytes)?;
            assert_eq!(inspection.minecraft_version.as_deref(), Some(version));
        }
        Ok(())
    }

    #[test]
    fn rejects_unsafe_minecraft_version_pins() -> anyhow::Result<()> {
        for version in [
            "",
            ".",
            "..",
            "1.20.1 ",
            "1.20.*",
            ">=1.20.1",
            "1.20.1/",
            "versions/1.20.1",
            "../1.20.1",
            "1.20.1\u{7}",
        ] {
            assert!(
                parse_loader(json!({"minecraft": version})).is_err(),
                "accepted Minecraft version {version:?}"
            );
        }
        let overlong = "1".repeat(MAX_MINECRAFT_VERSION_BYTES + 1);
        assert!(
            parse_loader(json!({"minecraft": overlong})).is_err(),
            "accepted an overlong Minecraft version"
        );
        Ok(())
    }

    #[test]
    fn parses_minimal_pack_and_normalizes_hashes() -> anyhow::Result<()> {
        let mut file = file_entry(
            "mods/example.jar",
            json!(["http://127.0.0.1:8123/mods/example.jar"]),
        );
        file["hashes"] = json!({"sha1": sha1().to_uppercase(), "sha512": sha512().to_uppercase()});
        let bytes = encoded(pack_index(json!({"minecraft": "1.20.1"}), json!([file])))?;

        let inspection = parse(&bytes)?;
        assert_eq!(inspection.format, PackFormat::Modrinth);
        assert_eq!(inspection.content, PackContent::Modrinth);
        assert_eq!(inspection.name, "Example Pack");
        assert_eq!(inspection.version.as_deref(), Some("1.0.0"));
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(inspection.loader, None);
        assert!(inspection.missing_files.is_empty());

        let [file] = inspection.files.as_slice() else {
            bail!("expected exactly one inspected file");
        };
        assert!(file.required);
        assert_eq!(file.path, "mods/example.jar");
        assert_eq!(file.sha1, sha1());
        assert_eq!(file.sha512, sha512());
        assert_eq!(file.size, 1024);
        assert_eq!(file.urls, ["http://127.0.0.1:8123/mods/example.jar"]);
        Ok(())
    }

    #[test]
    fn client_environment_selects_files() -> anyhow::Result<()> {
        let mut defaulted = file_entry(
            "mods/defaulted.jar",
            json!(["https://cdn.example/defaulted.jar"]),
        );
        defaulted["env"] = json!({"server": "unsupported"});
        let mut optional = file_entry(
            "mods/optional.jar",
            json!(["https://cdn.example/optional.jar"]),
        );
        optional["env"] = json!({"client": "optional", "server": "required"});
        let mut unsupported = file_entry(
            "mods/unsupported.jar",
            json!(["https://cdn.example/unsupported.jar"]),
        );
        unsupported["env"] = json!({"client": "unsupported", "server": "required"});
        let mut required = file_entry(
            "mods/required.jar",
            json!(["https://cdn.example/required.jar"]),
        );
        required["env"] = json!({"client": "required", "server": "optional"});

        let files = parse_files(json!([defaulted, optional, unsupported, required]))?;
        assert_eq!(
            files,
            vec![
                ("mods/defaulted.jar".to_owned(), true),
                ("mods/optional.jar".to_owned(), false),
                ("mods/required.jar".to_owned(), true),
            ]
        );
        Ok(())
    }

    #[test]
    fn server_only_entry_does_not_conflict_with_client_destination() -> anyhow::Result<()> {
        let mut client = file_entry(
            "mods/example.jar",
            json!(["https://example.invalid/client"]),
        );
        client["env"] = json!({"client": "required", "server": "unsupported"});
        let mut server = file_entry(
            "mods/example.jar",
            json!(["https://example.invalid/server"]),
        );
        server["env"] = json!({"client": "unsupported", "server": "required"});
        assert_eq!(
            parse_files(json!([client, server]))?,
            vec![("mods/example.jar".to_owned(), true)]
        );
        Ok(())
    }

    #[test]
    fn rejects_unrecognized_environments() -> anyhow::Result<()> {
        for environment in [
            json!({"client": "sometimes"}),
            json!({"client": true}),
            json!({"server": "client"}),
            json!({"client": "required", "server": "dedicated"}),
            json!("required"),
        ] {
            let mut file = file_entry(
                "mods/example.jar",
                json!(["https://cdn.example/example.jar"]),
            );
            file["env"] = environment.clone();
            assert!(
                parse_files(json!([file])).is_err(),
                "accepted environment {environment}"
            );
        }
        Ok(())
    }

    #[test]
    fn loader_dependencies_become_catalog_pins() -> anyhow::Result<()> {
        let cases = [
            (
                json!({"minecraft": "1.20.1", "fabric-loader": "0.16.5"}),
                Some((LoaderKind::Fabric, "0.16.5")),
            ),
            (
                json!({"minecraft": "1.20.1", "quilt-loader": "0.26.1-beta.2"}),
                Some((LoaderKind::Quilt, "0.26.1-beta.2")),
            ),
            (
                json!({"minecraft": "1.20.1", "forge": "47.2.0"}),
                Some((LoaderKind::Forge, "1.20.1-47.2.0")),
            ),
            (
                json!({"minecraft": "1.20.1", "forge": "1.20.1-47.2.0"}),
                Some((LoaderKind::Forge, "1.20.1-47.2.0")),
            ),
            (
                json!({"minecraft": "1.12.2", "forge": "14.23.5.2859"}),
                Some((LoaderKind::Forge, "1.12.2-14.23.5.2859")),
            ),
            (
                json!({"minecraft": "1.20.1", "neoforge": "47.1.106"}),
                Some((LoaderKind::NeoForge, "1.20.1-47.1.106")),
            ),
            (
                json!({"minecraft": "1.20.1", "neoforge": "1.20.1-47.1.106"}),
                Some((LoaderKind::NeoForge, "1.20.1-47.1.106")),
            ),
            (
                json!({"minecraft": "1.21.1", "neoforge": "21.1.80"}),
                Some((LoaderKind::NeoForge, "21.1.80")),
            ),
            (
                json!({"minecraft": "1.20.4", "neoforge": "20.4.237"}),
                Some((LoaderKind::NeoForge, "20.4.237")),
            ),
            (json!({"minecraft": "1.20.1"}), None),
        ];
        for (dependencies, expected) in cases {
            let observed = parse_loader(dependencies.clone())?;
            assert_eq!(
                observed,
                expected.map(|(kind, version)| (kind, version.to_owned())),
                "dependencies {dependencies}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_or_unsupported_loaders() -> anyhow::Result<()> {
        for dependencies in [
            json!({"minecraft": "1.20.1", "fabric-loader": "0.16.5", "quilt-loader": "0.26.1"}),
            json!({"minecraft": "1.20.1", "forge": "47.2.0", "neoforge": "47.1.106"}),
            json!({"minecraft": "1.20.1", "fabric": "0.16.5"}),
            json!({"minecraft": "1.20.1", "iris": "1.7.0"}),
            json!({"minecraft": "1.20.1", "forge": "banana"}),
            json!({"minecraft": "1.20.1", "fabric-loader": "0.16/5"}),
            json!({"minecraft": "1.21.1", "neoforge": "20.4.237"}),
            json!({"minecraft": "1.20.1", "forge": ""}),
            json!({"minecraft": "1.20.1", "forge": 47}),
            json!({"minecraft": "1.20.1", "fabric-loader": "*"}),
            json!({"forge": "47.2.0"}),
            json!({"minecraft": ""}),
            json!({"minecraft": "1.20.*"}),
            json!(["minecraft"]),
        ] {
            assert!(
                parse_loader(dependencies.clone()).is_err(),
                "accepted dependencies {dependencies}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_file_declarations() -> anyhow::Result<()> {
        let documents = [
            json!({"hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": 7, "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": "ab".repeat(38), "sha512": sha512()}, "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": "zz".repeat(20), "sha512": sha512()}, "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1()}, "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": "cd".repeat(126)}, "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": "sha1", "fileSize": 1024, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": -1, "downloads": ["https://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": []}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": [42]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": "https://cdn.example/a.jar"}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": ["ftp://cdn.example/a.jar"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": ["javascript:void(0)"]}),
            json!({"path": "mods/a.jar", "hashes": {"sha1": sha1(), "sha512": sha512()}, "fileSize": 1024, "downloads": ["https://"]}),
            json!(7),
        ];
        for document in documents {
            assert!(
                parse_files(json!([document.clone()])).is_err(),
                "accepted file declaration {document}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_conflicting_destinations() -> anyhow::Result<()> {
        for (left, right) in [
            ("mods/example.jar", "mods/example.jar"),
            ("mods/Example.jar", "mods/example.jar"),
            ("mods/example.jar", "mods/example.jar/config.json"),
            ("mods", "mods/example.jar"),
            ("config", "config/settings.toml"),
        ] {
            let files = json!([
                file_entry(left, json!(["https://cdn.example/left.bin"])),
                file_entry(right, json!(["https://cdn.example/right.bin"])),
            ]);
            assert!(
                parse_files(files).is_err(),
                "accepted {left} together with {right}"
            );
        }

        // A file/directory collision is detected even when a sibling name sorts nearby.
        let files = json!([
            file_entry("mods", json!(["https://cdn.example/mods"])),
            file_entry(
                "mods-extra/readme.txt",
                json!(["https://cdn.example/readme.txt"])
            ),
            file_entry("mods/inner.jar", json!(["https://cdn.example/inner.jar"])),
        ]);
        assert!(parse_files(files).is_err());

        let files = json!([
            file_entry(
                "mods/example.jar",
                json!(["https://cdn.example/example.jar"])
            ),
            file_entry("mods/other.jar", json!(["https://cdn.example/other.jar"])),
            file_entry(
                "config/mod/options.toml",
                json!(["https://cdn.example/options.toml"])
            ),
        ]);
        assert_eq!(parse_files(files)?.len(), 3);
        Ok(())
    }

    #[test]
    fn rejects_unsupported_format_and_identity() -> anyhow::Result<()> {
        let documents = [
            json!({"formatVersion": 2, "game": "minecraft", "name": "P", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": "1", "game": "minecraft", "name": "P", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"game": "minecraft", "name": "P", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft-bedrock", "name": "P", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft", "name": "", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft", "name": "   ", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft", "name": "P\u{0}", "versionId": "1", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft", "name": "P", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft", "name": "P", "versionId": "", "dependencies": {"minecraft": "1.20.1"}}),
            json!({"formatVersion": 1, "game": "minecraft", "name": "P", "versionId": "1"}),
            json!([]),
            json!("modrinth"),
        ];
        for document in documents {
            let bytes = encoded(document.clone())?;
            assert!(parse(&bytes).is_err(), "accepted pack index {document}");
        }

        // A pack that installs no remote files is still a valid pack.
        let mut blank = pack_index(json!({"minecraft": "1.20.1"}), json!([]));
        if let Some(object) = blank.as_object_mut() {
            object.remove("files");
        }
        let bytes = encoded(blank)?;
        assert!(parse(&bytes)?.files.is_empty());
        Ok(())
    }

    #[test]
    fn errors_and_debug_keep_download_urls_private() -> anyhow::Result<()> {
        let url = "http://127.0.0.1:8123/private/token/mod.jar";

        let mut file = file_entry("mods/example.jar", json!([url]));
        file["hashes"]["sha1"] = json!("zz".repeat(20));
        let error = match parse_files(json!([file])) {
            Ok(_) => bail!("accepted a file with an invalid hash"),
            Err(error) => error,
        };
        assert!(!format!("{error:#}").contains("127.0.0.1"));

        let file = file_entry(
            "mods/example.jar",
            json!(["ftp://127.0.0.1:8123/private/mod.jar"]),
        );
        let error = match parse_files(json!([file])) {
            Ok(_) => bail!("accepted a file with an unsupported URL scheme"),
            Err(error) => error,
        };
        assert!(!format!("{error:#}").contains("127.0.0.1"));

        let bytes = encoded(pack_index(
            json!({"minecraft": "1.20.1"}),
            json!([file_entry("mods/example.jar", json!([url]))]),
        ))?;
        let debug = format!("{:?}", parse(&bytes)?);
        assert!(debug.contains("url_count"));
        assert!(!debug.contains("127.0.0.1"));
        assert!(!debug.contains("/private"));

        let error = match parse(br#"{"formatVersion": 1, "game": "hidden-input""#) {
            Ok(_) => bail!("accepted truncated JSON"),
            Err(error) => error,
        };
        assert!(!format!("{error:#}").contains("hidden-input"));
        Ok(())
    }
}
