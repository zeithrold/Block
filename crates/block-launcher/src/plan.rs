use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Context as _, ensure};
use block_download::Resource;
use minecraft_manifest::{
    model::{Argument, ArgumentValue},
    resources::RuntimeFile,
};
use serde::{Deserialize, Serialize};

use crate::{
    platform::{Architecture, OperatingSystem, Target},
    recipe::{LaunchRecipe, RuntimeSelection},
    resolver::{ResolvedMetadata, download_resource},
    rules,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrepareAction {
    Copy {
        sha1: String,
        path: String,
        executable: bool,
    },
    Directory {
        path: String,
    },
    Link {
        path: String,
        target: String,
        scope: String,
    },
    Extract {
        sha1: String,
        directory: String,
        excludes: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JavaExecutable {
    Managed { path: String },
    Local { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandTemplate {
    pub java: JavaExecutable,
    pub java_major: u32,
    pub main_class: String,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub classpath: Vec<String>,
    pub variables: BTreeMap<String, String>,
    pub path_variables: BTreeMap<String, String>,
    pub cwd: String,
}

/// Serializable, secret-free instructions. The executor does not parse recipes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    pub schema_version: u32,
    pub instance: String,
    pub version: String,
    pub manifest_sha1: String,
    pub target: Target,
    pub resources: Vec<Resource>,
    pub actions: Vec<PrepareAction>,
    pub command: CommandTemplate,
    /// Explicit compatibility substitution applied during resolution, if any.
    /// Saved plans keep it so review and logs never present a silent change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
}

#[derive(Default)]
struct PlanFiles {
    resources: Vec<Resource>,
    actions: Vec<PrepareAction>,
    classpath: Vec<String>,
}

const CLASSPATH_TOKEN: &str = "${classpath}";

/// Compile resolved metadata into an inspectable plan without network or filesystem IO.
///
/// # Errors
/// Fails for invalid rules, missing native/runtime metadata, or inconsistent paths.
pub fn compile(
    recipe: &LaunchRecipe,
    metadata: &ResolvedMetadata,
) -> anyhow::Result<ExecutionPlan> {
    recipe.validate()?;
    crate::support::manifest_support(
        &metadata.parsed.manifest,
        &metadata.target,
        &recipe.features,
    )?
    .require()?;
    let mut files = PlanFiles::default();
    plan_libraries(recipe, metadata, &mut files)?;
    plan_assets(metadata, &recipe.instance, &mut files)?;
    let java = plan_runtime(recipe, metadata, &mut files.resources, &mut files.actions)?;
    let command = plan_command(recipe, metadata, java, &mut files)?;
    Ok(ExecutionPlan {
        schema_version: 1,
        instance: recipe.instance.clone(),
        version: metadata.parsed.manifest.id.clone(),
        manifest_sha1: metadata.parsed.manifest_sha1.clone(),
        target: metadata.target.clone(),
        resources: files.resources,
        actions: files.actions,
        command,
        compatibility: metadata.compatibility.clone(),
    })
}

fn plan_libraries(
    recipe: &LaunchRecipe,
    metadata: &ResolvedMetadata,
    files: &mut PlanFiles,
) -> anyhow::Result<()> {
    let manifest = &metadata.parsed.manifest;
    let target = &metadata.target;
    let natives = format!("instances/{}/runs/${{run_id}}/natives", recipe.instance);
    let library_root = format!("views/libraries/{}", metadata.parsed.manifest_sha1);
    for library in &manifest.libraries {
        if !rules::allowed(library.rules.as_deref(), target, &recipe.features)? {
            continue;
        }
        if let Some(artifact) = &library.downloads.artifact {
            if target.os == OperatingSystem::Linux
                && target.arch == Architecture::Aarch64
                && library.name.starts_with("org.lwjgl:")
                && library.name.ends_with(":natives-linux")
            {
                ensure!(
                    metadata.library_overrides.contains_key(&library.name),
                    "Linux ARM64 requires the explicit linux_arm64_lwjgl3 compatibility recipe"
                );
            }
            let (resource, filename) =
                if let Some(replacement) = metadata.library_overrides.get(&library.name) {
                    (replacement.resource.clone(), replacement.filename.as_str())
                } else {
                    (
                        Resource {
                            sha1: artifact.sha1.clone(),
                            size: artifact.size,
                            url: artifact.url.clone(),
                        },
                        artifact
                            .path
                            .rsplit('/')
                            .next()
                            .context("empty artifact path")?,
                    )
                };
            let path = format!("{library_root}/{}-{filename}", resource.sha1);
            add_copy(
                &mut files.resources,
                &mut files.actions,
                resource,
                path.clone(),
                false,
            );
            files.classpath.push(path);
        }
        if let Some(classifier) = library
            .natives
            .as_ref()
            .and_then(|natives| natives.get(target.manifest_os()))
        {
            let key = classifier.replace("${arch}", target.bits());
            let artifact = library
                .downloads
                .classifiers
                .as_ref()
                .and_then(|classifiers| classifiers.get(&key))
                .with_context(|| format!("native classifier {key} missing for {}", library.name))?;
            ensure!(
                target.arch != Architecture::Aarch64 || target.os != OperatingSystem::Linux,
                "legacy native classifier {} has no reviewed Linux ARM64 recipe",
                library.name
            );
            files.resources.push(Resource {
                sha1: artifact.sha1.clone(),
                size: artifact.size,
                url: artifact.url.clone(),
            });
            files.actions.push(PrepareAction::Extract {
                sha1: artifact.sha1.clone(),
                directory: natives.clone(),
                excludes: library
                    .extract
                    .as_ref()
                    .map_or_else(Vec::new, |extract| extract.exclude.clone()),
            });
        }
    }
    let client_path = format!(
        "views/clients/{}/client.jar",
        manifest.downloads.client.sha1
    );
    add_copy(
        &mut files.resources,
        &mut files.actions,
        download_resource(&manifest.downloads.client),
        client_path.clone(),
        false,
    );
    files.classpath.push(client_path);
    Ok(())
}

fn plan_assets(
    metadata: &ResolvedMetadata,
    instance: &str,
    files: &mut PlanFiles,
) -> anyhow::Result<()> {
    let manifest = &metadata.parsed.manifest;
    let game_dir = format!("instances/{instance}/game");
    let asset_root = format!("views/assets/{}", manifest.asset_index.sha1);
    let asset_descriptor = Resource {
        sha1: manifest.asset_index.sha1.clone(),
        size: manifest.asset_index.size,
        url: manifest.asset_index.url.clone(),
    };
    add_copy(
        &mut files.resources,
        &mut files.actions,
        asset_descriptor,
        format!("{asset_root}/indexes/{}.json", manifest.asset_index.id),
        false,
    );
    for (name, asset) in &metadata.assets.objects {
        ensure!(
            asset.hash.len() == 40 && asset.hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid asset hash"
        );
        let resource = Resource {
            sha1: asset.hash.clone(),
            size: asset.size,
            url: format!(
                "https://resources.download.minecraft.net/{}/{}",
                &asset.hash[..2],
                asset.hash
            ),
        };
        let path = format!("{asset_root}/objects/{}/{}", &asset.hash[..2], asset.hash);
        add_copy(
            &mut files.resources,
            &mut files.actions,
            resource,
            path,
            false,
        );
        let named_root = if metadata.assets.virtual_layout == Some(true) {
            Some(format!("{asset_root}/virtual/{}", manifest.asset_index.id))
        } else if metadata.assets.map_to_resources == Some(true) {
            Some(format!("{game_dir}/resources"))
        } else {
            None
        };
        if let Some(root) = named_root {
            files.actions.push(PrepareAction::Copy {
                sha1: asset.hash.clone(),
                path: format!("{root}/{name}"),
                executable: false,
            });
        }
    }
    Ok(())
}

fn argument_sets(
    recipe: &LaunchRecipe,
    metadata: &ResolvedMetadata,
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let manifest = &metadata.parsed.manifest;
    let target = &metadata.target;
    let (mut jvm_args, game_args) = if let Some(arguments) = &manifest.arguments {
        let mut jvm = expand_arguments(&arguments.jvm, target, &recipe.features)?;
        if let Some(defaults) = &arguments.default_user_jvm {
            jvm.extend(expand_arguments(defaults, target, &recipe.features)?);
        }
        (
            jvm,
            expand_arguments(&arguments.game, target, &recipe.features)?,
        )
    } else {
        (
            vec![
                "-Djava.library.path=${natives_directory}".into(),
                "-cp".into(),
                CLASSPATH_TOKEN.into(),
            ],
            manifest
                .minecraft_arguments
                .as_deref()
                .context("no game arguments")?
                .split_whitespace()
                .map(str::to_owned)
                .collect(),
        )
    };
    jvm_args.retain(|arg| !arg.starts_with("-Xms") && !arg.starts_with("-Xmx"));
    jvm_args.push("-Xms256M".into());
    jvm_args.push(format!("-Xmx{}M", recipe.memory_mib));
    jvm_args.extend(recipe.jvm_args.clone());
    Ok((jvm_args, game_args))
}

fn plan_command(
    recipe: &LaunchRecipe,
    metadata: &ResolvedMetadata,
    java: JavaExecutable,
    files: &mut PlanFiles,
) -> anyhow::Result<CommandTemplate> {
    let manifest = &metadata.parsed.manifest;
    let (mut jvm_args, game_args) = argument_sets(recipe, metadata)?;
    let game_dir = format!("instances/{}/game", recipe.instance);
    let natives = format!("instances/{}/runs/${{run_id}}/natives", recipe.instance);
    let asset_root = format!("views/assets/{}", manifest.asset_index.sha1);
    let library_root = format!("views/libraries/{}", metadata.parsed.manifest_sha1);
    let mut path_variables = BTreeMap::from([
        ("game_directory".into(), game_dir.clone()),
        ("natives_directory".into(), natives.clone()),
        ("assets_root".into(), asset_root.clone()),
        (
            "game_assets".into(),
            if metadata.assets.map_to_resources == Some(true) {
                format!("{game_dir}/resources")
            } else {
                format!("{asset_root}/virtual/{}", manifest.asset_index.id)
            },
        ),
        ("library_directory".into(), library_root),
    ]);
    if let Some(logging) = &manifest.logging {
        let file = &logging.client.file;
        let path = format!("views/logging/{}/{}", file.sha1, file.id);
        add_copy(
            &mut files.resources,
            &mut files.actions,
            Resource {
                sha1: file.sha1.clone(),
                size: file.size,
                url: file.url.clone(),
            },
            path.clone(),
            false,
        );
        path_variables.insert("path".into(), path);
        jvm_args.push(logging.client.argument.clone());
    }
    for path in [&natives, &game_dir] {
        files
            .actions
            .push(PrepareAction::Directory { path: path.clone() });
    }
    // Recent manifests partition native extraction locations by subsystem.
    for directory in ["java", "jna", "lwjgl", "netty"] {
        files.actions.push(PrepareAction::Directory {
            path: format!("{natives}/{directory}"),
        });
    }
    let variables = BTreeMap::from([
        ("version_name".into(), manifest.id.clone()),
        ("assets_index_name".into(), manifest.asset_index.id.clone()),
        ("launcher_name".into(), "Block".into()),
        ("launcher_version".into(), env!("CARGO_PKG_VERSION").into()),
        (
            "version_type".into(),
            serde_json::to_value(manifest.kind)?
                .as_str()
                .context("invalid version type")?
                .into(),
        ),
        ("user_properties".into(), "{}".into()),
        ("resolution_width".into(), "1280".into()),
        ("resolution_height".into(), "720".into()),
    ]);
    Ok(CommandTemplate {
        java,
        java_major: manifest
            .java_version
            .as_ref()
            .map_or(8, |java| java.major_version),
        main_class: manifest.main_class.clone(),
        jvm_args,
        game_args,
        classpath: std::mem::take(&mut files.classpath),
        variables,
        path_variables,
        cwd: game_dir,
    })
}

fn plan_runtime(
    recipe: &LaunchRecipe,
    metadata: &ResolvedMetadata,
    resources: &mut Vec<Resource>,
    actions: &mut Vec<PrepareAction>,
) -> anyhow::Result<JavaExecutable> {
    if let RuntimeSelection::Local { java } = &recipe.runtime {
        return Ok(JavaExecutable::Local { path: java.clone() });
    }
    let (digest, manifest) = metadata
        .runtime
        .as_ref()
        .context("managed runtime metadata is missing")?;
    let root = format!("runtimes/{}/{digest}", metadata.target.runtime_platform());
    let mut java_path = None;
    for (name, file) in &manifest.files {
        let path = format!("{root}/{name}");
        if name == "bin/java" || name == "bin/java.exe" || name.ends_with("/Contents/Home/bin/java")
        {
            java_path = Some(path.clone());
        }
        match file {
            RuntimeFile::File {
                downloads,
                executable,
            } => add_copy(
                resources,
                actions,
                download_resource(&downloads.raw),
                path,
                *executable,
            ),
            RuntimeFile::Directory => actions.push(PrepareAction::Directory { path }),
            RuntimeFile::Link { target } => actions.push(PrepareAction::Link {
                path,
                target: target.clone(),
                scope: root.clone(),
            }),
        }
    }
    Ok(JavaExecutable::Managed {
        path: java_path.context("runtime manifest contains no Java executable")?,
    })
}

fn add_copy(
    resources: &mut Vec<Resource>,
    actions: &mut Vec<PrepareAction>,
    resource: Resource,
    path: String,
    executable: bool,
) {
    actions.push(PrepareAction::Copy {
        sha1: resource.sha1.clone(),
        path,
        executable,
    });
    resources.push(resource);
}

/// Evaluate mixed literal/conditional arguments without tokenizing expanded values.
///
/// # Errors
/// Fails if a rule contains an invalid expression or version range.
pub fn expand_arguments(
    arguments: &[Argument],
    target: &Target,
    features: &BTreeMap<String, bool>,
) -> anyhow::Result<Vec<String>> {
    let mut expanded = Vec::new();
    for argument in arguments {
        match argument {
            Argument::Literal(value) => expanded.push(value.clone()),
            Argument::Conditional(conditional) => {
                if rules::allowed(conditional.rules.as_deref(), target, features)? {
                    match &conditional.value {
                        ArgumentValue::Single(value) => expanded.push(value.clone()),
                        ArgumentValue::Multiple(values) => expanded.extend(values.clone()),
                    }
                }
            }
        }
    }
    Ok(expanded)
}
