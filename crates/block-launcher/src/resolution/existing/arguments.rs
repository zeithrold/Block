use std::collections::BTreeMap;

use anyhow::{Context as _, ensure};

use super::resources::Inputs;
use crate::{
    manifest::model::{Argument, VersionManifest},
    plan::ResourcePath,
    platform::{OperatingSystem, Target},
};

pub(super) fn normalize(
    manifest: &mut VersionManifest,
    target: &Target,
    inputs: &mut Inputs<'_>,
) -> anyhow::Result<BTreeMap<String, ResourcePath>> {
    let arguments = manifest
        .arguments
        .as_mut()
        .context("Existing version has no supported arguments")?;
    let mut variables = BTreeMap::new();
    let mut jvm = crate::plan::expand_arguments(&arguments.jvm, target, &BTreeMap::new())?;
    if let Some(defaults) = arguments.default_user_jvm.take() {
        jvm.extend(crate::plan::expand_arguments(
            &defaults,
            target,
            &BTreeMap::new(),
        )?);
    }
    let mut expanded = Vec::new();
    for value in jvm {
        if let Some(reference) = value.strip_prefix('@') {
            let logical =
                local_path(reference, inputs)?.context("Unsupported source argument-file path")?;
            let file = inputs.select(&logical, None)?;
            ensure!(
                file.size <= 1024 * 1024,
                "Existing argument file is too large"
            );
            for argument in tokenize(std::str::from_utf8(&file.read_verified(1024 * 1024)?)?)? {
                ensure!(
                    !argument.starts_with('@'),
                    "Nested argument files are unsupported"
                );
                expanded.push(argument);
            }
        } else {
            expanded.push(value);
        }
    }
    let mut path_list = false;
    for argument in &mut expanded {
        ensure!(
            !argument.starts_with("-javaagent:") && !argument.starts_with("-agentpath:"),
            "Existing source agent arguments are unsupported"
        );
        let (prefix, value) = argument
            .split_once('=')
            .map_or(("", argument.as_str()), |(key, value)| (key, value));
        let declared_path_value = path_list
            || matches!(
                prefix,
                "--class-path"
                    | "--module-path"
                    | "-DlegacyClassPath"
                    | "-Djava.class.path"
                    | "-Djava.library.path"
                    | "-DlibraryDirectory"
                    | "-Dlog4j.configurationFile"
            );
        path_list = matches!(
            argument.as_str(),
            "-cp" | "-classpath" | "--class-path" | "-p" | "--module-path"
        );
        let separator = if target.os == OperatingSystem::Windows {
            ";"
        } else {
            ":"
        };
        let value = value.replace("${classpath_separator}", separator);
        let mut paths = Vec::new();
        for entry in value.split(separator) {
            if let Some(logical) = local_path(entry, inputs)? {
                inputs.select(&logical, None)?;
                let key = format!("existing_resource_{}", variables.len());
                variables.insert(
                    key.clone(),
                    inputs
                        .paths
                        .get(&logical)
                        .context("Missing normalized resource path")?
                        .clone(),
                );
                paths.push(format!("${{{key}}}"));
            } else {
                ensure!(
                    !entry.contains("${library_directory}")
                        && !std::path::Path::new(entry).is_absolute()
                        && (!declared_path_value
                            || matches!(
                                entry,
                                "${classpath}" | "${natives_directory}" | "${path}"
                            )),
                    "Unsupported source path argument: {entry}"
                );
                paths.push(entry.to_owned());
            }
        }
        let value = paths.join(separator);
        *argument = if prefix.is_empty() {
            value
        } else {
            format!("{prefix}={value}")
        };
    }
    arguments.jvm = expanded.into_iter().map(Argument::Literal).collect();
    let mut game = crate::plan::expand_arguments(&arguments.game, target, &BTreeMap::new())?;
    let mut index = 0;
    while index < game.len() {
        let (flag, inline) = game[index]
            .split_once('=')
            .map_or((game[index].as_str(), false), |(flag, _)| (flag, true));
        let owned = match flag {
            "--gameDir" => Some("${game_directory}"),
            "--assetsDir" => Some(
                if game[index].ends_with("=${game_assets}")
                    || game
                        .get(index + 1)
                        .is_some_and(|value| value == "${game_assets}")
                {
                    "${game_assets}"
                } else {
                    "${assets_root}"
                },
            ),
            "--assetIndex" => Some("${assets_index_name}"),
            "--version" => Some("${version_name}"),
            "--versionType" => Some("${version_type}"),
            "--username" => Some("${auth_player_name}"),
            "--uuid" => Some("${auth_uuid}"),
            "--accessToken" => Some("${auth_access_token}"),
            "--userType" => Some("${user_type}"),
            "--xuid" => Some("${auth_xuid}"),
            "--clientId" => Some("${clientid}"),
            "--userProperties" => Some("${user_properties}"),
            "--session" => Some("${auth_session}"),
            _ => None,
        };
        if let Some(value) = owned {
            if inline {
                game[index] = format!("{flag}={value}");
                index += 1;
                continue;
            }
            let argument = game
                .get_mut(index + 1)
                .context("Existing owned launch argument has no value")?;
            *argument = value.to_owned();
            index += 2;
        } else {
            index += 1;
        }
    }
    arguments.game = game.into_iter().map(Argument::Literal).collect();
    Ok(variables)
}

fn local_path(value: &str, inputs: &Inputs<'_>) -> anyhow::Result<Option<String>> {
    let logical = if let Some(relative) = value.strip_prefix("${library_directory}/") {
        Some(format!("libraries/{relative}"))
    } else if value.starts_with("libraries/") || value.starts_with("versions/") {
        Some(value.to_owned())
    } else if std::path::Path::new(value).is_absolute() {
        let path = std::path::Path::new(value);
        let canonical = crate::catalog::normalize_directory(
            path.parent()
                .context("Source argument resource has no parent")?,
        )?
        .join(
            path.file_name()
                .context("Source argument resource has no filename")?,
        );
        let relative = canonical
            .strip_prefix(inputs.root)
            .context("Source argument resource is outside the registered root")?;
        Some(
            relative
                .to_str()
                .context("Existing resource path is not UTF-8")?
                .replace('\\', "/"),
        )
    } else {
        None
    };
    if let Some(logical) = &logical {
        crate::executor::confined_path(inputs.root, logical)?;
        ensure!(
            logical.starts_with("libraries/") || logical.starts_with("versions/"),
            "Source argument path is outside version/library resources"
        );
        ensure!(
            !logical.contains("${"),
            "Unsupported source path placeholder"
        );
    }
    Ok(logical)
}

/// Tokenize legacy manifest and Java argument-file quoting without a shell.
pub(super) fn tokenize(value: &str) -> anyhow::Result<Vec<String>> {
    let mut result = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut started = false;
    for character in value.chars() {
        ensure!(character != '\0', "NUL in source arguments");
        if comment {
            comment = character != '\n';
            continue;
        }
        if escaped {
            token.push(match character {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
            started = true;
        } else if character == '\\' {
            escaped = true;
        } else if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                token.push(character);
            }
        } else if character == '\'' || character == '"' {
            quote = Some(character);
            started = true;
        } else if character == '#' && !started {
            comment = true;
        } else if character.is_whitespace() {
            if started {
                result.push(std::mem::take(&mut token));
                started = false;
            }
        } else {
            token.push(character);
            started = true;
        }
    }
    ensure!(
        quote.is_none() && !escaped,
        "Unterminated source argument quoting"
    );
    if started {
        result.push(token);
    }
    Ok(result)
}
