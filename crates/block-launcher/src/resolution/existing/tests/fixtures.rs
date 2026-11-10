use std::{fs, path::Path};

use serde_json::{Value, json};
use sha1::{Digest as _, Sha1};

use super::super::{ResolveContext, ResolvedMetadata, resolve_existing};
use crate::catalog::{Catalog, Instance, InstanceOwnership, TargetSource};
use crate::recipe::RuntimeSelection;
use anyhow::Context as _;

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    pub(super) instance: Instance,
    pub(super) context: ResolveContext,
    pub(super) manifest: Value,
}

pub(super) fn write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    fs::create_dir_all(path.parent().context("Missing fixture parent")?)?;
    fs::write(path, bytes)?;
    Ok(())
}

pub(super) fn descriptor(bytes: &[u8]) -> Value {
    json!({"sha1":format!("{:x}", Sha1::digest(bytes)),"size":bytes.len(),"url":"https://example.invalid/fixture"})
}

impl Fixture {
    #[cfg(unix)]
    pub(super) fn registered(
        &mut self,
        events: crate::executor::ExecutionSink,
    ) -> anyhow::Result<(crate::library::Library, Instance)> {
        use crate::{
            catalog::{DataPolicy, InstanceConfiguration},
            game_import::{DiscoveryRoots, discover},
            library::Library,
        };
        use std::os::unix::fs::PermissionsExt as _;
        self.manifest["javaVersion"] = json!({"component":"java-runtime-gamma","majorVersion":17});
        self.save_manifest()?;
        let library = Library::new(
            self.context.store.root().join("control"),
            std::sync::Arc::new(|_| {}),
            events,
        )?;
        let java = self.context.store.root().join("java");
        let architecture = match crate::platform::Target::host()?.arch {
            crate::platform::Architecture::Aarch64 => "aarch64",
            crate::platform::Architecture::X86_64 => "amd64",
            crate::platform::Architecture::X86 => "x86",
        };
        fs::write(
            &java,
            format!(
                "#!/bin/sh\necho 'java.version = 17.0.9' >&2\necho 'os.arch = {architecture}' >&2\n"
            ),
        )?;
        fs::set_permissions(&java, fs::Permissions::from_mode(0o755))?;
        let report = discover(&DiscoveryRoots {
            adjacent_minecraft: Some(self.root().to_owned()),
            official_launcher: None,
        });
        let candidate = report.candidates.first().context("Source candidate")?;
        let instance = library.catalog().register_existing(
            candidate,
            InstanceConfiguration {
                name: "Direct source".into(),
                game_directory: self.root().to_owned(),
                data_policy: DataPolicy::Shared,
                runtime: RuntimeSelection::Local { java },
                ..Default::default()
            },
        )?;
        Ok((library, instance))
    }

    pub(super) fn new() -> anyhow::Result<Self> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?.join("source");
        fs::create_dir(&root)?;
        let catalog = Catalog::open(temp.path().join("catalog"))?;
        let mut instance = catalog.create_instance("Source".into(), "1.20.1".into(), None, None)?;
        instance.ownership = InstanceOwnership::Registered;
        instance.resource_directory = root.clone();
        instance.game_directory = root.clone();
        instance.runtime = RuntimeSelection::Local {
            java: std::env::current_exe()?,
        };
        instance.targets[0].source = TargetSource::Existing {
            version_id: "1.20.1".into(),
        };
        instance.compatibility = crate::recipe::Compatibility::Official;
        let asset = b"asset fixture";
        let hash = format!("{:x}", Sha1::digest(asset));
        let assets = serde_json::to_vec(
            &json!({"objects":{"minecraft/sound.ogg":{"hash":hash,"size":asset.len()}}}),
        )?;
        write(
            &root.join(format!("assets/objects/{}/{hash}", &hash[..2])),
            asset,
        )?;
        write(&root.join("assets/indexes/test.json"), &assets)?;
        write(&root.join("versions/1.20.1/1.20.1.jar"), b"client fixture")?;
        let target = crate::platform::Target::host()?;
        let classifier = match target.os {
            crate::platform::OperatingSystem::Macos => "natives-macos-arm64",
            crate::platform::OperatingSystem::Linux => "natives-linux-arm64",
            crate::platform::OperatingSystem::Windows => "natives-windows-arm64",
        };
        let mut libraries = Vec::new();
        for module in ["lwjgl", "lwjgl-glfw"] {
            let name = format!("org.lwjgl:{module}:3.3.3:{classifier}");
            let path = crate::install::neoforge::coordinate_path(&name)?;
            let bytes = name.as_bytes();
            write(&root.join("libraries").join(&path), bytes)?;
            let mut artifact = descriptor(bytes);
            artifact["path"] = path.into();
            libraries.push(json!({"name":name,"downloads":{"artifact":artifact}}));
        }
        write(
            &root.join("libraries/example/override/1/override-1.jar"),
            b"parent library",
        )?;
        libraries.push(json!({"name":"example:override:1"}));
        let mut index = descriptor(&assets);
        index["id"] = "test".into();
        index["totalSize"] = asset.len().into();
        let manifest = json!({
            "id":"1.20.1","type":"release","time":"","releaseTime":"","minimumLauncherVersion":0,
            "assets":"test","assetIndex":index,"downloads":{"client":descriptor(b"client fixture")},
            "mainClass":"net.minecraft.client.main.Main","libraries":libraries,
            "arguments":{"jvm":["-cp","${classpath}","-Djava.library.path=${natives_directory}"],"game":["--gameDir","ignored-source-directory","--assetsDir","${assets_root}"]}
        });
        write(
            &root.join("versions/1.20.1/1.20.1.json"),
            &serde_json::to_vec(&manifest)?,
        )?;
        let cache = temp.path().join("cache");
        fs::create_dir(&cache)?;
        let context = ResolveContext {
            store: crate::Store::new(cache, true, std::sync::Arc::new(|_| {}))?,
            cancel: crate::CancellationToken::new(),
        };
        Ok(Self {
            _temp: temp,
            instance,
            context,
            manifest,
        })
    }

    pub(super) fn root(&self) -> &Path {
        &self.instance.resource_directory
    }

    pub(super) fn save_manifest(&self) -> anyhow::Result<()> {
        write(
            &self.root().join("versions/1.20.1/1.20.1.json"),
            &serde_json::to_vec(&self.manifest)?,
        )
    }

    pub(super) fn select(&mut self, id: &str, overlay: Value) -> anyhow::Result<()> {
        write(
            &self.root().join(format!("versions/{id}/{id}.json")),
            &serde_json::to_vec(&overlay)?,
        )?;
        self.instance.targets[0].source = TargetSource::Existing {
            version_id: id.into(),
        };
        Ok(())
    }

    pub(super) async fn resolve(&self) -> anyhow::Result<ResolvedMetadata> {
        resolve_existing(&self.instance, &self.instance.targets[0], &self.context).await
    }
}
