use super::{DetectedLoader, LoaderKind};

pub(crate) fn is_known_main_class(main_class: &str) -> bool {
    matches!(
        main_class,
        "net.minecraft.client.main.Main"
            | "net.minecraft.client.Minecraft"
            | "net.minecraft.launchwrapper.Launch"
            | "net.fabricmc.loader.impl.launch.knot.KnotClient"
            | "net.fabricmc.loader.launch.knot.KnotClient"
            | "org.quiltmc.loader.impl.launch.knot.KnotClient"
            | "cpw.mods.modlauncher.Launcher"
            | "cpw.mods.bootstraplauncher.BootstrapLauncher"
    )
}

pub(crate) fn detect_loader(json: &serde_json::Value, id: &str) -> Option<DetectedLoader> {
    let (from_id, base) = classify_version(id);
    if from_id.kind != LoaderKind::Vanilla {
        return Some(from_id);
    }
    let libraries = json.get("libraries").and_then(serde_json::Value::as_array);
    let coordinate = |expected_group: &str, expected_artifact: &str| {
        libraries?.iter().find_map(|library| {
            let name = library.get("name")?.as_str()?;
            let (group, artifact, version) = split_maven_coordinate(name)?;
            (group == expected_group && artifact == expected_artifact).then(|| version.to_owned())
        })
    };
    let detected = if let Some(version) = coordinate("net.neoforged", "neoforge") {
        Some(DetectedLoader {
            kind: LoaderKind::NeoForge,
            version: Some(version),
        })
    } else if let Some(version) = coordinate("net.minecraftforge", "forge") {
        Some(DetectedLoader {
            kind: LoaderKind::Forge,
            version: Some(version),
        })
    } else if let Some(version) = coordinate("net.fabricmc", "fabric-loader") {
        Some(DetectedLoader {
            kind: LoaderKind::Fabric,
            version: Some(version),
        })
    } else if let Some(version) = coordinate("org.quiltmc", "quilt-loader") {
        Some(DetectedLoader {
            kind: LoaderKind::Quilt,
            version: Some(version),
        })
    } else {
        let main_class = json
            .get("mainClass")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if main_class.contains("fabricmc") {
            Some(DetectedLoader {
                kind: LoaderKind::Fabric,
                version: None,
            })
        } else if main_class.contains("quiltmc") {
            Some(DetectedLoader {
                kind: LoaderKind::Quilt,
                version: None,
            })
        } else if main_class.contains("neoforged") {
            Some(DetectedLoader {
                kind: LoaderKind::NeoForge,
                version: None,
            })
        } else if main_class.contains("forge") || main_class.contains("modlauncher") {
            Some(DetectedLoader {
                kind: LoaderKind::Forge,
                version: None,
            })
        } else if looks_like_minecraft_version(id) || main_class == "net.minecraft.client.main.main"
        {
            Some(DetectedLoader {
                kind: LoaderKind::Vanilla,
                version: None,
            })
        } else {
            None
        }
    };
    detected.or_else(|| {
        (base.as_deref().is_some_and(looks_like_minecraft_version)).then_some(DetectedLoader {
            kind: LoaderKind::Vanilla,
            version: None,
        })
    })
}

fn split_maven_coordinate(name: &str) -> Option<(&str, &str, &str)> {
    let mut parts = name.split(':');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

pub(crate) fn minecraft_version_from_id(id: &str) -> Option<String> {
    let (loader, base) = classify_version(id);
    if loader.kind != LoaderKind::Vanilla {
        base.filter(|value| looks_like_minecraft_version(value))
    } else {
        looks_like_minecraft_version(id).then(|| id.to_owned())
    }
}

pub(crate) fn looks_like_minecraft_version(value: &str) -> bool {
    fn numeric(value: &str) -> bool {
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
    }
    let lower = value.to_ascii_lowercase();
    if lower.as_bytes().get(2) == Some(&b'w')
        && lower.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && lower.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
        && lower
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return true;
    }
    let concrete = lower
        .strip_prefix('a')
        .or_else(|| lower.strip_prefix('b'))
        .unwrap_or(&lower);
    let (base, suffix) = concrete
        .split_once('-')
        .map_or((concrete, None), |(base, suffix)| (base, Some(suffix)));
    let parts = base.split(['.', '_']).collect::<Vec<_>>();
    if parts.len() < 2 || !parts.iter().all(|part| numeric(part)) {
        return false;
    }
    suffix.is_none_or(|suffix| {
        suffix
            .strip_prefix("pre")
            .or_else(|| suffix.strip_prefix("rc"))
            .is_some_and(numeric)
            || suffix
                .strip_prefix("snapshot-")
                .or_else(|| suffix.strip_prefix("experimental-snapshot-"))
                .is_some_and(numeric)
    })
}

pub(super) fn classify_version(id: &str) -> (DetectedLoader, Option<String>) {
    let lower = id.to_ascii_lowercase();
    if let Some((base, rest)) = lower.split_once("-neoforge-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::NeoForge,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some((base, rest)) = lower.split_once("-forge-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::Forge,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some((base, rest)) = lower.split_once("-fabric-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::Fabric,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some((base, rest)) = lower.split_once("-quilt-")
        && looks_like_minecraft_version(base)
    {
        return (
            DetectedLoader {
                kind: LoaderKind::Quilt,
                version: Some(rest.to_owned()),
            },
            Some(base.to_owned()),
        );
    }
    if let Some(rest) = lower.strip_prefix("fabric-loader-") {
        let mut parts = rest.splitn(2, '-');
        let loader = parts.next().unwrap_or_default();
        let mc = parts.next().map(str::to_owned);
        return (
            DetectedLoader {
                kind: LoaderKind::Fabric,
                version: Some(loader.to_owned()),
            },
            mc,
        );
    }
    if let Some(rest) = lower.strip_prefix("quilt-loader-") {
        let mut parts = rest.splitn(2, '-');
        let loader = parts.next().unwrap_or_default();
        let mc = parts.next().map(str::to_owned);
        return (
            DetectedLoader {
                kind: LoaderKind::Quilt,
                version: Some(loader.to_owned()),
            },
            mc,
        );
    }
    if let Some(rest) = lower.strip_prefix("neoforge-") {
        return (
            DetectedLoader {
                kind: LoaderKind::NeoForge,
                version: Some(rest.to_owned()),
            },
            None,
        );
    }
    if let Some(rest) = lower.strip_prefix("forge-") {
        let (base, loader_version) = if let Some((mc, loader)) = rest.split_once('-') {
            if mc.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                (Some(mc.to_owned()), Some(loader.to_owned()))
            } else {
                (None, Some(rest.to_owned()))
            }
        } else {
            (None, Some(rest.to_owned()))
        };
        return (
            DetectedLoader {
                kind: LoaderKind::Forge,
                version: loader_version,
            },
            base,
        );
    }
    (
        DetectedLoader {
            kind: LoaderKind::Vanilla,
            version: None,
        },
        Some(id.to_owned()),
    )
}
