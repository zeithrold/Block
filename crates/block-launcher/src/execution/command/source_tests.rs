#[cfg(unix)]
#[tokio::test]
async fn source_classpath_and_legacy_assets_are_read_from_the_locked_actual_directory()
-> anyhow::Result<()> {
    use crate::{
        plan::{
            CommandTemplate, ExecutionPlan, ExistingResource, JavaExecutable, PrepareAction,
            ResourcePath,
        },
        platform::Target,
        session::OfflineSession,
    };
    use anyhow::Context as _;
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt as _};

    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let source = root.join("existing client.jar");
    std::fs::write(&source, b"source client bytes")?;
    let source = ExistingResource::capture(source)?;
    let actual = root.join("actual game");
    std::fs::create_dir_all(actual.join("resources"))?;
    std::fs::write(actual.join("resources/sound.ogg"), b"actual legacy asset")?;
    let executable = root.join("java-fixture");
    std::fs::write(
        &executable,
        br#"#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in
    -cp) shift; client="$1" ;;
    --assetsDir) shift; assets="$1" ;;
    --gameDir) shift; game="$1" ;;
  esac
  shift
done
[ "$PWD" = "$game" ] || exit 41
cat "$client"
printf '\n'
cat "$assets/sound.ogg"
"#,
    )?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
    let mut plan = ExecutionPlan {
        schema_version: 2,
        instance: "source-binding".into(),
        version: "fixture".into(),
        manifest_sha1: String::new(),
        target: Target::host()?,
        resources: Vec::new(),
        existing_resources: vec![source.clone()],
        existing_asset_root: None,
        actions: vec![PrepareAction::GameResource {
            source: ResourcePath::Existing {
                path: source.path.clone(),
            },
            sha1: source.sha1.clone(),
            size: source.size,
            path: "sound.ogg".into(),
        }],
        command: CommandTemplate {
            java: JavaExecutable::Local { path: executable },
            java_major: 8,
            main_class: "fixture.Main".into(),
            jvm_args: vec!["-cp".into(), "${classpath}".into()],
            game_args: vec![
                "--gameDir".into(),
                "${game_directory}".into(),
                "--assetsDir".into(),
                "${game_assets}".into(),
            ],
            classpath: vec![ResourcePath::Existing {
                path: source.path.clone(),
            }],
            variables: BTreeMap::new(),
            path_variables: BTreeMap::from([
                (
                    "game_directory".into(),
                    ResourcePath::Cache {
                        relative: "synthetic/game".into(),
                    },
                ),
                (
                    "game_assets".into(),
                    ResourcePath::Cache {
                        relative: "synthetic/game/resources".into(),
                    },
                ),
            ]),
            cwd: "synthetic/game".into(),
        },
        compatibility: None,
        runtime_substitution: None,
    };
    let session = OfflineSession::Offline {
        username: "Player".into(),
    };
    let command =
        super::JavaCommand::bind_for_directory(&plan, &root, "run", &session, Some(&actual))?;
    let output = command.process().output().await?;
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(output.stdout, b"source client bytes\nactual legacy asset");
    plan.existing_resources.clear();
    assert!(
        super::JavaCommand::bind_for_directory(&plan, &root, "run", &session, Some(&actual))
            .err()
            .context("undeclared source classpath was accepted")?
            .to_string()
            .contains("Undeclared")
    );
    Ok(())
}
