use super::fixtures::{Fixture, descriptor, write};
use crate::plan::{PrepareAction, ResourcePath};

#[tokio::test]
async fn unlisted_relative_module_paths_fail_before_java_instead_of_bypassing_resource_validation()
-> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    fixture.manifest["arguments"]["jvm"]
        .as_array_mut()
        .context("jvm arguments missing")?
        .extend([
            Value::String("-p".into()),
            Value::String("unlisted.jar".into()),
        ]);
    fixture.save_manifest()?;
    let error = fixture
        .resolve()
        .await
        .err()
        .context("undeclared module path was accepted")?;
    assert!(
        error
            .to_string()
            .contains("Unsupported source path argument")
    );
    Ok(())
}
use anyhow::Context as _;
use serde_json::{Value, json};
use std::fs;

#[tokio::test]
async fn legacy_named_assets_are_declared_for_the_actual_game_directory() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let index_path = fixture.root().join("assets/indexes/test.json");
    let mut index: Value = serde_json::from_slice(&fs::read(&index_path)?)?;
    index["map_to_resources"] = true.into();
    let bytes = serde_json::to_vec(&index)?;
    write(&index_path, &bytes)?;
    fixture.manifest["assetIndex"]["sha1"] = descriptor(&bytes)["sha1"].clone();
    fixture.manifest["assetIndex"]["size"] = bytes.len().into();
    fixture
        .manifest
        .as_object_mut()
        .context("fixture object")?
        .remove("arguments");
    fixture.manifest["minecraftArguments"] =
        "--gameDir \"${game_directory}\" --assetsDir ${game_assets}".into();
    fixture.save_manifest()?;
    let metadata = fixture.resolve().await?;
    let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    assert!(plan.actions.iter().any(|action| matches!(action, PrepareAction::GameResource { source: ResourcePath::Existing { path: source }, path, .. } if path == "minecraft/sound.ogg" && source.starts_with(fixture.root().join("assets/objects")))));
    assert!(!plan.actions.iter().any(|action| matches!(action, PrepareAction::Copy { path, .. } if path.contains("/game/resources/"))));
    Ok(())
}

#[tokio::test]
async fn corrupt_source_and_incomplete_asset_declarations_fail_before_command_binding()
-> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    let metadata = fixture.resolve().await?;
    let mut plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    plan.existing_resources.retain(|resource| {
        !resource
            .path
            .starts_with(fixture.root().join("assets/objects"))
    });
    assert!(
        plan.verify_existing_resources()
            .err()
            .context("incomplete assets were accepted")?
            .to_string()
            .contains("not declared")
    );
    fs::write(
        fixture.root().join("versions/1.20.1/1.20.1.jar"),
        b"changed client",
    )?;
    assert!(
        fixture
            .resolve()
            .await
            .err()
            .context("corrupt source was accepted")?
            .to_string()
            .contains("corrupt")
    );
    Ok(())
}

#[tokio::test]
async fn existing_native_classifiers_extract_only_into_run_private_cache() -> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let target = crate::platform::Target::host()?;
    let relative = "example/native/1/native-1-test.jar";
    let bytes = b"source native archive fixture";
    write(&fixture.root().join("libraries").join(relative), bytes)?;
    let mut artifact = descriptor(bytes);
    artifact["path"] = relative.into();
    let library = json!({
        "name":"example:native:1",
        "downloads":{"classifiers":{"test":artifact}},
        "natives":{(target.manifest_os()):"test"},
        "extract":{"exclude":["META-INF/"]}
    });
    fixture.manifest["libraries"]
        .as_array_mut()
        .context("libraries missing")?
        .push(library);
    fixture.save_manifest()?;
    let metadata = fixture.resolve().await?;
    let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    let native = plan
        .actions
        .iter()
        .find_map(|action| match action {
            PrepareAction::ExtractExisting {
                resource,
                directory,
                excludes,
            } => Some((resource, directory, excludes)),
            _ => None,
        })
        .context("Source native extraction was not planned")?;
    assert_eq!(
        native.0.path,
        fixture.root().join("libraries").join(relative)
    );
    assert_eq!(
        native.1,
        &format!(
            "preparations/{}/runs/${{run_id}}/natives",
            fixture.instance.id
        )
    );
    assert_eq!(native.2, &["META-INF/"]);
    native.0.verify()?;
    Ok(())
}

#[tokio::test]
async fn source_argument_files_are_pinned_and_expanded_into_declared_module_paths()
-> anyhow::Result<()> {
    let mut fixture = Fixture::new()?;
    let args = b"# installed launch args\n-p \"${library_directory}/example/override/1/override-1.jar\"\n--add-modules ALL-MODULE-PATH\n";
    write(&fixture.root().join("libraries/launch.args"), args)?;
    fixture.manifest["arguments"]["jvm"]
        .as_array_mut()
        .context("jvm arguments missing")?
        .push("@libraries/launch.args".into());
    fixture.save_manifest()?;
    let metadata = fixture.resolve().await?;
    let plan = crate::plan::compile_existing(&fixture.instance, &metadata)?;
    assert!(
        plan.existing_resources
            .iter()
            .any(|resource| resource.path == fixture.root().join("libraries/launch.args"))
    );
    assert!(plan.command.jvm_args.contains(&"ALL-MODULE-PATH".into()));
    assert!(
        !plan
            .command
            .jvm_args
            .iter()
            .any(|argument| argument.starts_with('@') || argument.contains("${library_directory}"))
    );
    assert!(plan.command.path_variables.values().any(|path| {
        *path
            == ResourcePath::Existing {
                path: fixture
                    .root()
                    .join("libraries/example/override/1/override-1.jar"),
            }
    }));
    Ok(())
}
