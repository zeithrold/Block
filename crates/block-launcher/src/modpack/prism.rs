//! Bounded parser for a Prism Launcher or MultiMC instance directory.
//!
//! `mmc-pack.json` pins the Minecraft and loader components; the sibling
//! `instance.cfg` supplies the display name. Inspection takes no Java path, JVM
//! argument, memory limit or hook behavior from either file: an imported
//! instance launches with Block's own defaults, and a nonempty hook command is
//! rejected because Block never runs it.

use std::collections::BTreeSet;

use anyhow::{Context as _, bail, ensure};
use serde::Deserialize;

use super::{LoaderKind, PackContent, PackFormat, PackInspection};

const MINECRAFT_COMPONENT: &str = "net.minecraft";

/// Components that do not select a launcher loader: LWJGL and the mapping jars a
/// Prism or Quilt instance pins beside its loader.
const HELPER_COMPONENTS: &[&str] = &[
    "org.lwjgl",
    "org.lwjgl3",
    "net.fabricmc.intermediary",
    "org.quiltmc.hashed",
];

/// Instance configuration keys whose commands Block must not silently drop.
const HOOK_KEYS: &[&str] = &["PreLaunchCommand", "PostExitCommand", "WrapperCommand"];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MmcPack {
    format_version: u64,
    components: Vec<Component>,
}

#[derive(Deserialize)]
struct Component {
    uid: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    disabled: bool,
}

/// Read one Prism or MultiMC instance as a transient inspection.
///
/// `game_root` is the instance-relative Minecraft directory the archive owner
/// selected and validated.
///
/// # Errors
/// Rejects a nonstandard pack format, a disabled, unknown, duplicate or
/// unversioned component, more than one loader, a missing Minecraft pin, and an
/// unusable instance configuration.
pub(super) fn parse(pack: &[u8], cfg: &[u8], game_root: &str) -> anyhow::Result<PackInspection> {
    let document: MmcPack = serde_json::from_slice(pack).context("Invalid Prism pack metadata")?;
    ensure!(
        document.format_version == 1,
        "Prism pack format version {} is not supported; only version 1 is",
        document.format_version
    );
    let (minecraft_version, loader) = components(&document.components)?;
    Ok(PackInspection {
        format: PackFormat::Prism,
        name: instance_name(cfg)?,
        version: None,
        minecraft_version: Some(minecraft_version),
        loader,
        target_evidence: super::TargetEvidence::Manifest,
        has_mods: false,
        files: Vec::new(),
        missing_files: Vec::new(),
        content: PackContent::Prism {
            root: game_root.to_owned(),
        },
        content_entries: Vec::new(),
        archive_prefix: String::new(),
    })
}

/// Resolve the pinned Minecraft version and the single declared loader.
///
/// Forge builds are stored as `<minecraft>-<loader>`, and NeoForge 1.20.1 still
/// ships from the Forge artifact, so it uses the same form. A pin that already
/// carries the pinned Minecraft version keeps its exact form.
fn components(entries: &[Component]) -> anyhow::Result<(String, Option<(LoaderKind, String)>)> {
    let mut seen = BTreeSet::new();
    let mut minecraft = None;
    let mut selected = None;
    for component in entries {
        let uid = component.uid.as_str();
        ensure!(
            !uid.trim().is_empty(),
            "Prism pack declares an empty component identity"
        );
        ensure!(
            !component.disabled,
            "Prism component '{uid}' is disabled; disabled components are not supported"
        );
        ensure!(
            seen.insert(uid),
            "Prism pack declares component '{uid}' twice"
        );
        let Some(raw) = component.version.as_deref() else {
            bail!("Prism component '{uid}' has no version");
        };
        if uid == MINECRAFT_COMPONENT {
            minecraft = Some(token(raw, "Minecraft version")?);
        } else if let Some(kind) = loader_kind(uid) {
            ensure!(
                selected.is_none(),
                "Prism pack declares more than one loader"
            );
            selected = Some((kind, token(raw, "loader version")?));
        } else if !HELPER_COMPONENTS.contains(&uid) {
            bail!("Prism component '{uid}' is not supported");
        }
    }
    let Some(minecraft_version) = minecraft else {
        bail!("Prism pack does not pin a {MINECRAFT_COMPONENT} version");
    };
    let loader = selected.map(|(kind, raw)| {
        let version = loader_version(&kind, &minecraft_version, &raw);
        (kind, version)
    });
    Ok((minecraft_version, loader))
}

/// Block stores Forge builds as `<minecraft>-<loader>`; a pin that already
/// carries the pinned Minecraft version keeps its exact form.
fn loader_version(kind: &LoaderKind, minecraft: &str, version: &str) -> String {
    let already_prefixed = version
        .strip_prefix(minecraft)
        .is_some_and(|rest| rest.starts_with('-'));
    let game_prefixed = match kind {
        LoaderKind::Forge => true,
        LoaderKind::NeoForge => minecraft == "1.20.1",
        LoaderKind::Vanilla | LoaderKind::Fabric | LoaderKind::Quilt => false,
    };
    if game_prefixed && !already_prefixed {
        format!("{minecraft}-{version}")
    } else {
        version.to_owned()
    }
}

fn loader_kind(uid: &str) -> Option<LoaderKind> {
    match uid {
        "net.minecraftforge" => Some(LoaderKind::Forge),
        "net.neoforged" => Some(LoaderKind::NeoForge),
        "net.fabricmc.fabric-loader" => Some(LoaderKind::Fabric),
        "org.quiltmc.quilt-loader" => Some(LoaderKind::Quilt),
        _ => None,
    }
}

/// Read the display name from `instance.cfg` with ordinary cfg semantics.
///
/// Qt QSettings writes the instance's own settings either before any section or
/// in `[General]`; both scopes are instance root. Keys inside any other group are
/// unrelated and ignored. Java paths, JVM arguments and memory limits stay unread
/// so Block keeps its defaults, and a nonempty pre-launch, post-exit or wrapper
/// command is rejected in either root form. Errors never repeat configuration
/// contents, which may name local paths.
fn instance_name(cfg: &[u8]) -> anyhow::Result<String> {
    let text = std::str::from_utf8(cfg).context("Prism instance configuration is not UTF-8")?;
    let mut name = None;
    let mut root = true;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line).trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let section = line
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                .map(str::trim);
            let Some(section) = section.filter(|section| !section.is_empty()) else {
                bail!("Prism instance configuration has a malformed section header");
            };
            root = section == "General";
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            bail!("Prism instance configuration has a malformed line");
        };
        let (key, value) = (key.trim(), value.trim());
        ensure!(
            !key.is_empty(),
            "Prism instance configuration has a malformed line"
        );
        if !root {
            continue;
        }
        if key == "name" {
            name = Some(value.to_owned());
        } else if HOOK_KEYS.contains(&key) {
            ensure!(
                value.is_empty(),
                "Prism instance defines a {key} command, which Block does not run"
            );
        }
    }
    text_of(&name.unwrap_or_default(), "display name")
}

/// Non-empty display text without control characters.
fn text_of(value: &str, what: &str) -> anyhow::Result<String> {
    ensure!(
        !value.trim().is_empty() && !value.chars().any(char::is_control),
        "Prism instance has no usable {what}"
    );
    Ok(value.to_owned())
}

/// A single version-like token.
fn token(value: &str, what: &str) -> anyhow::Result<String> {
    ensure!(
        !value.is_empty()
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
            }),
        "Prism pack has an invalid {what} '{value}'"
    );
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(uid: &str, version: &str) -> String {
        format!(r#"{{"uid":"{uid}","version":"{version}"}}"#)
    }

    fn pack(components: &[String]) -> String {
        format!(
            r#"{{"formatVersion":1,"components":[{}]}}"#,
            components.join(",")
        )
    }

    fn cfg(name: &str) -> String {
        format!("InstanceType=OneSix\niconKey=default\nname={name}\n")
    }

    fn parse_instance(components: &[String], cfg: &str) -> anyhow::Result<PackInspection> {
        parse(pack(components).as_bytes(), cfg.as_bytes(), "minecraft")
    }

    #[test]
    fn parses_vanilla_instance_with_display_name() -> anyhow::Result<()> {
        let inspection = parse_instance(
            &[
                component("net.minecraft", "1.20.1"),
                component("org.lwjgl3", "3.3.1"),
            ],
            &cfg("My Pack"),
        )?;
        assert_eq!(inspection.format, PackFormat::Prism);
        assert_eq!(inspection.name, "My Pack");
        assert_eq!(inspection.version, None);
        assert_eq!(inspection.minecraft_version.as_deref(), Some("1.20.1"));
        assert_eq!(inspection.loader, None);
        assert!(inspection.files.is_empty());
        assert!(inspection.missing_files.is_empty());
        assert_eq!(
            inspection.content,
            PackContent::Prism {
                root: "minecraft".into()
            }
        );
        Ok(())
    }

    #[test]
    fn rejects_disabled_components_without_enabling_them() -> anyhow::Result<()> {
        for uid in ["net.fabricmc.fabric-loader", "org.lwjgl3", "net.minecraft"] {
            let disabled = format!(r#"{{"uid":"{uid}","version":"1.20.1","disabled":true}}"#);
            let components = if uid == "net.minecraft" {
                vec![disabled]
            } else {
                vec![component("net.minecraft", "1.20.1"), disabled]
            };
            let result = parse_instance(&components, &cfg("Pack"));
            assert!(result.is_err(), "disabled component {uid} was accepted");
            if let Err(error) = result {
                let message = format!("{error:#}");
                assert!(message.contains("disabled"), "{message}");
                assert!(message.contains("not supported"), "{message}");
            }
        }
        Ok(())
    }

    #[test]
    fn maps_supported_loaders_and_pinned_helper_components() -> anyhow::Result<()> {
        for (components, kind, version) in [
            (
                vec![
                    component("net.minecraft", "1.20.1"),
                    component("net.minecraftforge", "47.4.28"),
                    component("org.lwjgl", "2.9.4-nightly-20150209"),
                ],
                LoaderKind::Forge,
                "1.20.1-47.4.28",
            ),
            (
                vec![
                    component("net.minecraft", "1.21.4"),
                    component("org.lwjgl3", "3.3.3"),
                    component("net.fabricmc.intermediary", "1.21.4"),
                    component("net.fabricmc.fabric-loader", "0.16.10"),
                ],
                LoaderKind::Fabric,
                "0.16.10",
            ),
            (
                vec![
                    component("net.minecraft", "1.20.1"),
                    component("org.quiltmc.hashed", "1.20.1"),
                    component("org.quiltmc.quilt-loader", "0.27.1"),
                ],
                LoaderKind::Quilt,
                "0.27.1",
            ),
            (
                vec![
                    component("net.minecraft", "1.20.1"),
                    component("net.neoforged", "47.1.106"),
                ],
                LoaderKind::NeoForge,
                "1.20.1-47.1.106",
            ),
            (
                vec![
                    component("net.minecraft", "1.21.1"),
                    component("net.neoforged", "21.1.219"),
                ],
                LoaderKind::NeoForge,
                "21.1.219",
            ),
            // A pin that already carries the Minecraft version keeps its exact form.
            (
                vec![
                    component("net.minecraft", "1.20.1"),
                    component("net.minecraftforge", "1.20.1-47.4.28"),
                ],
                LoaderKind::Forge,
                "1.20.1-47.4.28",
            ),
            (
                vec![
                    component("net.minecraft", "1.20.1"),
                    component("net.neoforged", "1.20.1-47.1.106"),
                ],
                LoaderKind::NeoForge,
                "1.20.1-47.1.106",
            ),
        ] {
            let inspection = parse_instance(&components, &cfg("Pack"))?;
            assert_eq!(
                inspection.loader,
                Some((kind, version.to_owned())),
                "{components:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_unknown_ambiguous_and_unversioned_components() -> anyhow::Result<()> {
        for components in [
            vec![
                component("net.minecraft", "1.20.1"),
                component("com.mumfrey.liteloader", "1.12.2"),
            ],
            vec![
                component("net.minecraft", "1.20.1"),
                component("net.minecraftforge", "47.4.28"),
                component("net.fabricmc.fabric-loader", "0.16.10"),
            ],
            vec![
                component("net.minecraft", "1.20.1"),
                r#"{"uid":"org.lwjgl3"}"#.to_owned(),
            ],
            vec![component("org.lwjgl3", "3.3.1")],
            vec![],
            vec![
                component("net.minecraft", "1.20.1"),
                component("net.minecraft", "1.20.1"),
            ],
            vec![
                component("net.minecraft", "1.20.1"),
                component("org.quiltmc.quilt-loader", ""),
            ],
        ] {
            assert!(
                parse_instance(&components, &cfg("Pack")).is_err(),
                "{components:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_other_pack_format_versions() -> anyhow::Result<()> {
        let components = pack(&[component("net.minecraft", "1.20.1")]);
        let other = components.replace(r#""formatVersion":1"#, r#""formatVersion":2"#);
        assert!(parse(other.as_bytes(), cfg("Pack").as_bytes(), "minecraft").is_err());
        assert!(parse(components.as_bytes(), cfg("Pack").as_bytes(), "minecraft").is_ok());
        Ok(())
    }

    #[test]
    fn rejects_nonempty_hook_commands() -> anyhow::Result<()> {
        let components = [component("net.minecraft", "1.20.1")];
        for key in HOOK_KEYS {
            let hooked = format!("name=Pack\n{key}=java -jar hook.jar\n");
            assert!(parse_instance(&components, &hooked).is_err(), "{key}");
            let empty = format!("name=Pack\n{key}=\n");
            assert!(parse_instance(&components, &empty).is_ok(), "{key}");
        }
        Ok(())
    }

    #[test]
    fn ignores_source_java_path_and_jvm_settings() -> anyhow::Result<()> {
        let cfg = "InstanceType=OneSix\nname=Modded Instance\nJavaPath=/opt/jdk-21/bin/java\nOverrideJavaLocation=true\nJvmArgs=-Xmx8G -XX:+UseG1GC\nOverrideJavaArgs=true\nMaxMemAlloc=8192\nMinMemAlloc=1024\nOverrideMemory=true\n";
        let inspection = parse_instance(
            &[
                component("net.minecraft", "1.21.4"),
                component("net.fabricmc.fabric-loader", "0.16.10"),
            ],
            cfg,
        )?;
        assert_eq!(inspection.name, "Modded Instance");
        assert_eq!(
            inspection.loader,
            Some((LoaderKind::Fabric, "0.16.10".into()))
        );
        Ok(())
    }

    #[test]
    fn reads_names_with_ordinary_cfg_whitespace_and_sections() -> anyhow::Result<()> {
        let components = [component("net.minecraft", "1.20.1")];
        let spaced = "; leading comment\r\n# another\r\n  name  =  Spaced Name  \r\n[Controls]\nPreLaunchCommand=echo hooked\n";
        assert_eq!(parse_instance(&components, spaced)?.name, "Spaced Name");
        let crlf = "InstanceType=OneSix\r\nname=Packed\r\nPreLaunchCommand=\r\n";
        assert_eq!(parse_instance(&components, crlf)?.name, "Packed");
        assert_eq!(
            parse_instance(&components, "[General]\nname=Grouped\n")?.name,
            "Grouped"
        );
        Ok(())
    }

    #[test]
    fn treats_general_section_as_instance_root_and_rejects_its_hooks() -> anyhow::Result<()> {
        let components = [component("net.minecraft", "1.20.1")];
        let realistic = "[General]\nInstanceType=OneSix\niconKey=default\nname=General Pack\nOverrideMemory=true\nMaxMemAlloc=4096\nJavaPath=/opt/jdk-21/bin/java\n";
        assert_eq!(parse_instance(&components, realistic)?.name, "General Pack");
        let quiet = "[General]\nInstanceType=OneSix\nname=Quiet Pack\nPreLaunchCommand=\nPostExitCommand=\nWrapperCommand=\n";
        assert_eq!(parse_instance(&components, quiet)?.name, "Quiet Pack");
        for key in HOOK_KEYS {
            let hooked = format!(
                "[General]\nInstanceType=OneSix\nname=Hooked Pack\n{key}=java -jar hook.jar\n"
            );
            assert!(parse_instance(&components, &hooked).is_err(), "{key}");
        }
        // An unrelated group is ignored, including hook-looking keys inside it.
        let grouped = "name=Kept\n[Controls]\nPreLaunchCommand=echo hooked\n";
        assert_eq!(parse_instance(&components, grouped)?.name, "Kept");
        Ok(())
    }

    #[test]
    fn errors_do_not_repeat_configuration_contents() -> anyhow::Result<()> {
        let components = [component("net.minecraft", "1.20.1")];
        let secret = "C:\\Users\\someone\\private\\run.bat";
        for cfg in [
            format!("name {secret}\n"),
            format!("name=Pack\nPreLaunchCommand={secret}\n"),
            format!("[General]\nname=Pack\nWrapperCommand={secret}\n"),
            format!("[{secret}]\nname=Pack\n"),
        ] {
            let failed = parse_instance(&components, &cfg);
            let message = failed
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_default();
            assert!(!message.contains("private"), "{message}");
        }
        Ok(())
    }

    #[test]
    fn rejects_unusable_instance_configurations() -> anyhow::Result<()> {
        let components = [component("net.minecraft", "1.20.1")];
        for cfg in [
            "InstanceType=OneSix\n",
            "name=\n",
            "name=   \n",
            "name=Bad\u{7}Name\n",
            "name Pack\n",
            "[ ]\nname=Pack\n",
        ] {
            assert!(parse_instance(&components, cfg).is_err(), "{cfg:?}");
        }
        let pack = pack(&components);
        assert!(parse(pack.as_bytes(), b"name=\xff\n", "minecraft").is_err());
        Ok(())
    }
}
