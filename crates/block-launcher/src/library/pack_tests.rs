//! Real-entry-point regressions for the local modpack lifecycle.
//!
//! Every test drives `Library`, `Catalog`, `Store` and the real `modpack` stages
//! against synthetic archives, a synthetic Minecraft version document and a
//! local HTTP fixture. Only states a real installation reaches after verified
//! preparation (a `Ready` loader target, a completed pack attachment) are
//! written through the production catalog setters; no production rule is
//! restated here.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context as _;
use block_download::{DownloadOptions, ProxyMode, Store};
use serde_json::{Value, json};
use sha1::Digest as _;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    sync::Mutex,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::Library;
#[cfg(unix)]
use crate::catalog::DataPolicy;
#[cfg(unix)]
use crate::recipe::RuntimeSelection;
use crate::{
    catalog::{InstallStatus, Instance, InstanceConfiguration, PackStatus, TargetSource},
    modpack::{self, PackSelection},
    session::OfflineSession,
};

const PACK_NAME: &str = "Synthetic Pack";
const MINECRAFT_VERSION: &str = "1.21.4";
const REQUIRED_PATH: &str = "mods/example.jar";
const REQUIRED_BYTES: &[u8] = b"synthetic required mod";
const OPTIONAL_PATH: &str = "configs/optional.toml";
const OPTIONAL_BYTES: &[u8] = b"synthetic optional config";
const REQUIRED_URL: &str = "https://cdn.example/mods/example.jar?token=private&signature=abc";
const OPTIONAL_URL: &str = "https://cdn.example/configs/optional.toml";

/// Local content of one pack, applied in archive order: the client layer wins.
const LAYERS: [(&str, &[u8]); 3] = [
    ("overrides/config/pack.toml", b"base-layer"),
    ("overrides/config/keep.toml", b"kept-layer"),
    ("client-overrides/config/pack.toml", b"client-layer"),
];

/// Bytes the real launch path must publish into every fresh game directory.
#[cfg(unix)]
const BASELINE_OPTIONS: &[u8] = b"baseline-options";
#[cfg(unix)]
const BASELINE_MOD: &[u8] = b"baseline-mod";
#[cfg(unix)]
const BASELINE_CONFIG: &[u8] = b"baseline-config";

/// Offline client and asset metadata the synthetic version document declares.
#[cfg(unix)]
const CLIENT_JAR: &[u8] = b"synthetic client jar";
#[cfg(unix)]
const ASSET_INDEX: &[u8] = b"{\"objects\":{}}";

/// One library set that plans without a substitution on every supported host:
/// the ARM64 natives are declared as artifacts, so an ARM64 host recognizes its
/// own core and window artifacts instead of resolving replacements from Maven.
#[cfg(unix)]
const LIBRARIES: [(&str, &str, &[u8]); 6] = [
    (
        "org.lwjgl:lwjgl:3.3.1",
        "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar",
        b"lwjgl-core",
    ),
    (
        "org.lwjgl:lwjgl:3.3.1:natives-linux-arm64",
        "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux-arm64.jar",
        b"linux-arm64-native",
    ),
    (
        "org.lwjgl:lwjgl:3.3.1:natives-windows-arm64",
        "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows-arm64.jar",
        b"windows-arm64-native",
    ),
    (
        "org.lwjgl:lwjgl:3.3.1:natives-macos-arm64",
        "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-macos-arm64.jar",
        b"macos-arm64-native",
    ),
    (
        "org.lwjgl:lwjgl-glfw:3.3.1",
        "org/lwjgl/lwjgl-glfw/3.3.1/lwjgl-glfw-3.3.1.jar",
        b"glfw-core",
    ),
    (
        "org.lwjgl:lwjgl-glfw:3.3.1:natives-macos-arm64",
        "org/lwjgl/lwjgl-glfw/3.3.1/lwjgl-glfw-3.3.1-natives-macos-arm64.jar",
        b"glfw-macos-arm64-native",
    ),
];

fn sha1_hex(bytes: &[u8]) -> String {
    format!("{:x}", sha1::Sha1::digest(bytes))
}

fn sha512_hex(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha512::digest(bytes))
}

/// Transfers stay direct and unretried so a local fixture never reaches a proxy
/// and a rejected body is reported once instead of after retry delays.
fn options() -> DownloadOptions {
    DownloadOptions {
        max_retries: 0,
        proxy: ProxyMode::Direct,
        ..DownloadOptions::default()
    }
}

fn library_at(root: &Path) -> anyhow::Result<Library> {
    Library::with_options(
        root.to_owned(),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
        options(),
    )
}

/// The download cache, which default storage locations keep at the library root.
fn download_store(library: &Library) -> anyhow::Result<Store> {
    Store::with_options(
        library.catalog().root().to_owned(),
        false,
        Arc::new(|_| {}),
        options(),
    )
}

#[cfg(unix)]
fn seed_object(store: &Store, bytes: &[u8]) -> anyhow::Result<String> {
    let digest = sha1_hex(bytes);
    let path = store.object_path(&digest)?;
    fs::create_dir_all(path.parent().context("object path has no parent")?)?;
    fs::write(&path, bytes)?;
    Ok(digest)
}

fn configuration(name: &str) -> InstanceConfiguration {
    InstanceConfiguration {
        name: name.to_owned(),
        ..InstanceConfiguration::default()
    }
}

#[cfg(unix)]
fn configuration_with_java(name: &str, java: &Path) -> InstanceConfiguration {
    InstanceConfiguration {
        runtime: RuntimeSelection::Local {
            java: java.to_owned(),
        },
        ..configuration(name)
    }
}

fn optional_selection() -> PackSelection {
    PackSelection {
        optional_files: BTreeSet::from([OPTIONAL_PATH.to_owned()]),
        optional_curse_files: BTreeSet::new(),
    }
}

fn write_archive(path: &Path, entries: Vec<(&str, Vec<u8>)>) -> anyhow::Result<()> {
    let mut archive = ZipWriter::new(fs::File::create(path)?);
    for (name, bytes) in entries {
        archive.start_file(name, SimpleFileOptions::default())?;
        archive.write_all(&bytes)?;
    }
    archive.finish()?;
    Ok(())
}

fn file_entry(path: &str, bytes: &[u8], urls: &[String], client: Option<&str>) -> Value {
    let mut entry = json!({
        "path": path,
        "hashes": {"sha1": sha1_hex(bytes), "sha512": sha512_hex(bytes)},
        "fileSize": bytes.len(),
        "downloads": urls,
    });
    if let Some(client) = client {
        entry["env"] = json!({"client": client, "server": "required"});
    }
    entry
}

fn write_mrpack_entries(
    path: &Path,
    required_urls: &[String],
    optional_urls: &[String],
    layers: &[(&str, &[u8])],
) -> anyhow::Result<()> {
    let index = json!({
        "formatVersion": 1,
        "game": "minecraft",
        "name": PACK_NAME,
        "versionId": "1.0.0",
        "dependencies": {"minecraft": MINECRAFT_VERSION},
        "files": [
            file_entry(REQUIRED_PATH, REQUIRED_BYTES, required_urls, None),
            file_entry(OPTIONAL_PATH, OPTIONAL_BYTES, optional_urls, Some("optional")),
        ],
    });
    let mut entries = vec![("modrinth.index.json", serde_json::to_vec(&index)?)];
    for (name, bytes) in layers {
        entries.push((*name, (*bytes).to_vec()));
    }
    write_archive(path, entries)
}

fn write_mrpack(
    path: &Path,
    required_urls: &[String],
    optional_urls: &[String],
) -> anyhow::Result<()> {
    write_mrpack_entries(path, required_urls, optional_urls, &LAYERS)
}

/// A standard CurseForge manifest whose entries name files only by ID.
fn write_curseforge_pack(
    path: &Path,
    loader: Option<&str>,
    required: &[(u64, u64)],
    optional: &[(u64, u64)],
) -> anyhow::Result<()> {
    let loaders: Vec<Value> = loader
        .map(|id| json!({"id": id, "primary": true}))
        .into_iter()
        .collect();
    let mut files = Vec::new();
    for &(project, file) in required {
        files.push(json!({"projectID": project, "fileID": file, "required": true}));
    }
    for &(project, file) in optional {
        files.push(json!({"projectID": project, "fileID": file, "required": false}));
    }
    let manifest = json!({
        "manifestType": "minecraftModpack",
        "manifestVersion": 1,
        "name": "Curse Pack",
        "version": "2.1.0",
        "minecraft": {"version": MINECRAFT_VERSION, "modLoaders": loaders},
        "files": files,
        "overrides": "overrides",
    });
    write_archive(
        path,
        vec![
            ("manifest.json", serde_json::to_vec(&manifest)?),
            ("overrides/config/example.toml", b"value".to_vec()),
        ],
    )
}

/// A synthetic version document the real resolver accepts offline.
#[cfg(unix)]
fn version_manifest() -> anyhow::Result<Vec<u8>> {
    let libraries: Vec<Value> = LIBRARIES
        .iter()
        .map(|&(name, path, bytes)| {
            json!({
                "name": name,
                "downloads": {"artifact": {
                    "path": path,
                    "sha1": sha1_hex(bytes),
                    "size": bytes.len(),
                    "url": format!("https://libraries.minecraft.net/{path}"),
                }},
            })
        })
        .collect();
    let manifest = json!({
        "id": MINECRAFT_VERSION,
        "type": "release",
        "time": "2024-01-01T00:00:00+00:00",
        "releaseTime": "2024-01-01T00:00:00+00:00",
        "mainClass": "net.minecraft.client.main.Main",
        "minimumLauncherVersion": 21,
        "assets": "fixture",
        "assetIndex": {
            "id": "fixture",
            "sha1": sha1_hex(ASSET_INDEX),
            "size": ASSET_INDEX.len(),
            "totalSize": 0,
            "url": "https://piston-meta.mojang.com/fixture/assets.json",
        },
        "downloads": {"client": {
            "sha1": sha1_hex(CLIENT_JAR),
            "size": CLIENT_JAR.len(),
            "url": "https://piston-data.mojang.com/fixture/client.jar",
        }},
        "javaVersion": {"component": "java-runtime-gamma", "majorVersion": 17},
        "libraries": libraries,
        "arguments": {
            "jvm": ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"],
            "game": ["--username", "${auth_player_name}", "--gameDir", "${game_directory}"],
        },
    });
    Ok(serde_json::to_vec_pretty(&manifest)?)
}

/// Fixture: publish the `Ready` state a verified installation reaches.
fn mark_pack_ready(library: &Library, instance: &Instance) -> anyhow::Result<()> {
    fs::create_dir_all(instance.resource_directory.join(".block-pack/baseline"))?;
    let pack = instance.pack.as_ref().context("instance has no pack")?;
    let mut target = instance
        .targets
        .iter()
        .find(|target| target.id == pack.target_id)
        .cloned()
        .context("pack target is missing")?;
    target.status = InstallStatus::Ready;
    library.catalog().set_target(&instance.id, &target)?;
    library.catalog().finish_pack_install(&instance.id, &target)
}

fn directory_names(path: &Path) -> anyhow::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// A local archive source for pack files; unknown paths answer `404`.
struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn server(routes: BTreeMap<String, Vec<u8>>) -> anyhow::Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let routes = Arc::new(routes);
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let routes = routes.clone();
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut request = [0_u8; 2048];
                let Ok(length) = socket.read(&mut request).await else {
                    return;
                };
                let text = String::from_utf8_lossy(&request[..length]).into_owned();
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                seen.lock().await.push(path.clone());
                let body = routes.get(&path).cloned().unwrap_or_default();
                let status = if routes.contains_key(&path) { 200 } else { 404 };
                let header = format!(
                    "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if socket.write_all(header.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(&body).await;
                }
            });
        }
    });
    Ok(Server {
        url,
        requests,
        task,
    })
}

#[test]
fn create_pack_publishes_one_instance_and_resumes_the_same_operation() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let source = temporary.path().join("synthetic.mrpack");
    write_mrpack(
        &source,
        &[REQUIRED_URL.to_owned()],
        &[OPTIONAL_URL.to_owned()],
    )?;
    let inspected = modpack::inspect(&source)?;
    let selection = optional_selection();
    let operation = uuid::Uuid::new_v4().to_string();
    let instance = library.create_pack(
        &operation,
        &source,
        &inspected,
        &selection,
        configuration(PACK_NAME),
        &CancellationToken::new(),
    )?;

    assert_eq!(instance.id, operation);
    assert_eq!(instance.name, PACK_NAME);
    assert_eq!(instance.target_id, "vanilla");
    assert!(instance.resource_directory.ends_with(&operation));
    let pack = instance
        .pack
        .as_ref()
        .context("pack attachment is missing")?;
    assert_eq!(pack.target_id, "vanilla");
    assert_eq!(pack.status, PackStatus::Pending);

    let metadata = instance.resource_directory.join(".block-pack");
    let descriptor = modpack::descriptor::load(&instance.resource_directory)?;
    assert_eq!(descriptor.name, PACK_NAME);
    let required = descriptor
        .files
        .iter()
        .find(|file| file.path == REQUIRED_PATH)
        .context("the required file was not declared")?;
    let optional = descriptor
        .files
        .iter()
        .find(|file| file.path == OPTIONAL_PATH)
        .context("the optional file was not declared")?;
    assert!(required.required && required.selected);
    assert!(!optional.required, "the optional declaration is kept");
    assert!(
        optional.selected,
        "the selected optional declaration is kept"
    );
    assert!(!descriptor.has_missing_selected());

    // Both override layers survive, in declaration order.
    assert_eq!(
        fs::read(metadata.join("overrides").join("config").join("pack.toml"))?,
        b"client-layer"
    );
    assert_eq!(
        fs::read(metadata.join("overrides").join("config").join("keep.toml"))?,
        b"kept-layer"
    );

    // The persisted declaration never carries a source or signature.
    let published = fs::read(metadata.join("pack.json"))?;
    let document = String::from_utf8(published.clone())?;
    for forbidden in ["http", "cdn.example", "token", "signature"] {
        assert!(
            !document.contains(forbidden),
            "{forbidden} was persisted in {document}"
        );
    }

    // Reusing the operation ID resumes the same instance.
    let resumed_instance = library.create_pack(
        &operation,
        &source,
        &inspected,
        &selection,
        configuration(PACK_NAME),
        &CancellationToken::new(),
    )?;
    assert_eq!(resumed_instance.id, instance.id);
    let state = library.catalog().snapshot()?;
    assert_eq!(state.instances.len(), 1, "exactly one catalog entry");
    assert_eq!(fs::read(metadata.join("pack.json"))?, published);

    // A different selection is a different import and never rewrites this one.
    let changed = library.create_pack(
        &operation,
        &source,
        &inspected,
        &PackSelection::default(),
        configuration(PACK_NAME),
        &CancellationToken::new(),
    );
    let error = changed
        .err()
        .context("a changed selection replaced an import")?;
    assert!(
        format!("{error:#}").contains("it was preserved"),
        "{error:#}"
    );
    assert_eq!(fs::read(metadata.join("pack.json"))?, published);
    Ok(())
}

#[test]
fn long_pack_name_can_be_replaced_without_changing_the_original_descriptor() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let source = temporary.path().join("unicode.mrpack");
    let original_name = "模".repeat(64);
    let index = json!({
        "formatVersion": 1,
        "game": "minecraft",
        "name": original_name,
        "versionId": "1",
        "dependencies": {"minecraft": MINECRAFT_VERSION},
        "files": []
    });
    write_archive(
        &source,
        vec![("modrinth.index.json", serde_json::to_vec(&index)?)],
    )?;
    let before = fs::read(&source)?;
    let inspected = modpack::inspect(&source)?;
    assert_eq!(inspected.name, original_name);
    let operation = uuid::Uuid::new_v4().to_string();
    for invalid in ["", original_name.as_str()] {
        assert!(
            library
                .create_pack(
                    &operation,
                    &source,
                    &inspected,
                    &PackSelection::default(),
                    configuration(invalid),
                    &CancellationToken::new(),
                )
                .is_err()
        );
        assert!(library.catalog().snapshot()?.instances.is_empty());
    }
    let instance = library.create_pack(
        &operation,
        &source,
        &inspected,
        &PackSelection::default(),
        configuration("短名称"),
        &CancellationToken::new(),
    )?;
    assert_eq!(instance.name, "短名称");
    assert_eq!(instance.name, "短名称");
    assert_eq!(
        modpack::descriptor::load(&instance.resource_directory)?.name,
        original_name
    );
    let again = library.create_pack(
        &operation,
        &source,
        &inspected,
        &PackSelection::default(),
        configuration("短名称"),
        &CancellationToken::new(),
    )?;
    assert_eq!(again.id, instance.id);
    assert_eq!(library.catalog().snapshot()?.instances.len(), 1);
    assert_eq!(fs::read(source)?, before);
    Ok(())
}

#[cfg(unix)]
#[test]
fn completed_pack_import_releases_inherited_lock_descriptions() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let operation = uuid::Uuid::new_v4().to_string();
    let guard = super::PackImportGuard::acquire(temporary.path(), &operation)?;
    let inherited = guard.0.try_clone()?;
    assert!(super::PackImportGuard::acquire(temporary.path(), &operation).is_err());

    drop(guard);
    let next_owner = super::PackImportGuard::acquire(temporary.path(), &operation)?;
    drop(inherited);
    assert!(super::PackImportGuard::acquire(temporary.path(), &operation).is_err());
    drop(next_owner);
    assert!(super::PackImportGuard::acquire(temporary.path(), &operation).is_ok());
    Ok(())
}

#[test]
fn create_pack_preserves_unrelated_data_and_a_foreign_destination() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let existing =
        library
            .catalog()
            .create_instance("Existing world".into(), "1.20.6".into(), None, None)?;
    let world = existing.resource_directory.join("saves").join("level.dat");
    fs::create_dir_all(world.parent().context("world has no parent")?)?;
    fs::write(&world, b"played world")?;

    let source = temporary.path().join("synthetic.mrpack");
    write_mrpack(
        &source,
        &[REQUIRED_URL.to_owned()],
        &[OPTIONAL_URL.to_owned()],
    )?;
    let inspected = modpack::inspect(&source)?;
    let selection = optional_selection();
    let packed = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &selection,
        configuration("Imported Pack"),
        &CancellationToken::new(),
    )?;
    let library_directory = packed
        .resource_directory
        .parent()
        .context("pack destination has no parent")?
        .to_owned();
    assert_eq!(
        Some(library_directory.as_path()),
        existing.resource_directory.parent(),
        "both installations share the library folder"
    );
    assert_eq!(fs::read(&world)?, b"played world");
    let state = library.catalog().snapshot()?;
    assert_eq!(state.instances.len(), 2);
    assert_eq!(
        state.selection(&existing.id)?.0.resource_directory,
        existing.resource_directory,
        "an unrelated installation keeps its directory"
    );

    // A directory that already exists at the operation's destination is never adopted.
    let foreign_operation = uuid::Uuid::new_v4().to_string();
    let foreign = library_directory.join(&foreign_operation);
    fs::create_dir(&foreign)?;
    fs::write(foreign.join("notes.txt"), b"user data")?;
    let attempt = library.create_pack(
        &foreign_operation,
        &source,
        &inspected,
        &selection,
        configuration("Imported Pack"),
        &CancellationToken::new(),
    );
    let error = attempt
        .err()
        .context("an unrelated directory became a pack instance")?;
    assert!(
        format!("{error:#}").contains("it was preserved"),
        "{error:#}"
    );
    assert_eq!(fs::read(foreign.join("notes.txt"))?, b"user data");
    assert_eq!(library.catalog().snapshot()?.instances.len(), 2);
    Ok(())
}

#[test]
fn create_pack_publishes_nothing_on_cancel_or_a_changed_archive() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let source = temporary.path().join("synthetic.mrpack");
    write_mrpack(
        &source,
        &[REQUIRED_URL.to_owned()],
        &[OPTIONAL_URL.to_owned()],
    )?;
    let inspected = modpack::inspect(&source)?;
    let selection = optional_selection();
    let instance = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &selection,
        configuration("Imported Pack"),
        &CancellationToken::new(),
    )?;
    let library_directory: PathBuf = instance
        .resource_directory
        .parent()
        .context("pack destination has no parent")?
        .to_owned();

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let cancelled_operation = uuid::Uuid::new_v4().to_string();
    let attempt = library.create_pack(
        &cancelled_operation,
        &source,
        &inspected,
        &selection,
        configuration("Imported Pack"),
        &cancelled,
    );
    let error = attempt.err().context("a cancelled import was published")?;
    assert!(format!("{error:#}").contains("cancelled"), "{error:#}");
    assert!(!library_directory.join(&cancelled_operation).exists());

    // The archive changed after inspection, so the staged attempt is discarded.
    let mut layers = LAYERS.to_vec();
    layers.push(("overrides/config/added.toml", b"added-layer"));
    write_mrpack_entries(
        &source,
        &[REQUIRED_URL.to_owned()],
        &[OPTIONAL_URL.to_owned()],
        &layers,
    )?;
    let changed_operation = uuid::Uuid::new_v4().to_string();
    let attempt = library.create_pack(
        &changed_operation,
        &source,
        &inspected,
        &selection,
        configuration("Imported Pack"),
        &CancellationToken::new(),
    );
    let error = attempt.err().context("a changed archive was accepted")?;
    assert!(
        format!("{error:#}").contains("inspect it again"),
        "{error:#}"
    );
    assert!(!library_directory.join(&changed_operation).exists());

    let state = library.catalog().snapshot()?;
    assert_eq!(state.instances.len(), 1);
    assert_eq!(state.selection(&instance.id)?.0.id, instance.id);
    assert_eq!(
        directory_names(&library_directory)?,
        vec![instance.id.clone()],
        "a failed import leaves no instance or staging directory"
    );
    Ok(())
}

#[tokio::test]
async fn a_pending_pack_blocks_launch_even_when_its_target_is_ready() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let source = temporary.path().join("curse.zip");
    write_curseforge_pack(&source, None, &[(238_222, 2_917_633)], &[])?;
    let inspected = modpack::inspect(&source)?;
    let instance = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &PackSelection::default(),
        configuration("Curse Pack"),
        &CancellationToken::new(),
    )?;
    assert_eq!(
        instance.pack.as_ref().context("pack is missing")?.status,
        PackStatus::Pending
    );

    // Fixture: the loader target is already Ready, so only the pack can refuse.
    let mut target = instance.targets[0].clone();
    target.status = InstallStatus::Ready;
    library.catalog().set_target(&instance.id, &target)?;

    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };
    let launch = library
        .prepare_launch(
            &instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await;
    let error = launch.err().context("a pending pack prepared a launch")?;
    assert!(
        format!("{error:#}").contains("pack installation is not ready"),
        "{error:#}"
    );
    // Pack metadata already exists at the game root; refusal publishes no
    // mutable game data or prepared Java run.
    assert!(!instance.game_directory.join("options.txt").exists());
    assert!(!instance.resource_directory.join("runs").exists());

    // The archive is not a verifiable substitute for the ID-only entry.
    let install = library
        .install_pack(&instance.id, &source, &CancellationToken::new())
        .await;
    let error = install
        .err()
        .context("an ID-only pack installed from its archive")?;
    assert!(
        format!("{error:#}").contains("cannot be verified from their CurseForge IDs"),
        "{error:#}"
    );
    let state = library.catalog().snapshot()?;
    let (current, target) = state.selection(&instance.id)?;
    assert_eq!(
        current.pack.as_ref().context("pack is missing")?.status,
        PackStatus::Pending,
        "a failed install never publishes the pack"
    );
    assert!(
        matches!(&target.status, InstallStatus::Failed { .. }),
        "retry stays available: {:?}",
        target.status
    );
    Ok(())
}

#[tokio::test]
async fn only_a_selected_optional_curse_entry_blocks_completion() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let source = temporary.path().join("curse.zip");
    write_curseforge_pack(&source, None, &[], &[(306_612, 3_489_031)])?;
    let inspected = modpack::inspect(&source)?;
    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };

    // An unselected optional ID-only entry is not a missing requirement.
    let unselected = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &PackSelection::default(),
        configuration("Curse Pack"),
        &CancellationToken::new(),
    )?;
    assert!(
        !modpack::descriptor::load(&unselected.resource_directory)?.has_missing_selected(),
        "an unselected optional entry cannot block"
    );
    mark_pack_ready(&library, &unselected)?;
    let error = library
        .prepare_launch(
            &unselected.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await
        .err()
        .context("an unselected optional entry blocked the pack")?;
    // The pack gate let it through; only the absent installation metadata refuses.
    assert!(
        format!("{error:#}").contains("installed metadata missing"),
        "{error:#}"
    );

    // Selecting the same entry is an explicit requirement and blocks.
    let selection = PackSelection {
        optional_files: BTreeSet::new(),
        optional_curse_files: BTreeSet::from([(306_612, 3_489_031)]),
    };
    let selected = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &selection,
        configuration("Curse Pack"),
        &CancellationToken::new(),
    )?;
    assert!(
        modpack::descriptor::load(&selected.resource_directory)?.has_missing_selected(),
        "a selected ID-only entry must block"
    );
    mark_pack_ready(&library, &selected)?;
    let error = library
        .prepare_launch(
            &selected.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await
        .err()
        .context("a selected ID-only entry was accepted for launch")?;
    assert!(
        format!("{error:#}").contains("does not match its launch target"),
        "{error:#}"
    );
    Ok(())
}

#[tokio::test]
async fn an_incompatible_target_cannot_bypass_the_declared_pack_target() -> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let source = temporary.path().join("curse.zip");
    write_curseforge_pack(&source, Some("fabric-0.16.10"), &[], &[])?;
    let inspected = modpack::inspect(&source)?;
    let instance = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &PackSelection::default(),
        configuration("Curse Pack"),
        &CancellationToken::new(),
    )?;
    assert_eq!(
        instance.pack.as_ref().context("pack is missing")?.target_id,
        "fabric-0.16.10"
    );
    assert_eq!(instance.target_id, "fabric-0.16.10");

    library.catalog().save_instance(
        &instance.id,
        InstanceConfiguration {
            target_id: "vanilla".into(),
            ..InstanceConfiguration::from(&instance)
        },
    )?;

    // Fixture: both loader targets are Ready and the package completed.
    for id in ["vanilla", "fabric-0.16.10"] {
        let mut target = instance
            .targets
            .iter()
            .find(|target| target.id == id)
            .cloned()
            .context("launch target is missing")?;
        target.status = InstallStatus::Ready;
        library.catalog().set_target(&instance.id, &target)?;
    }
    mark_pack_ready(&library, &instance)?;

    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };
    let error = library
        .prepare_launch(
            &instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await
        .err()
        .context("an incompatible target launched a completed pack")?;
    assert!(
        format!("{error:#}").contains("does not match the pack's declared loader"),
        "{error:#}"
    );

    // The declared target itself reaches installation metadata, so only the
    // mismatch refuses the other target.
    library
        .catalog()
        .save_instance(&instance.id, InstanceConfiguration::from(&instance))?;
    let error = library
        .prepare_launch(
            &instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await
        .err()
        .context("a completed pack launched without installed metadata")?;
    assert!(
        format!("{error:#}").contains("installed metadata missing"),
        "{error:#}"
    );
    Ok(())
}

/// A local Java a real preparation can probe: it reports the version and the
/// architecture the plan was compiled for, and never starts a game.
#[cfg(unix)]
fn synthetic_java(directory: &Path) -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    let host = crate::platform::Target::host()?;
    let arch = match host.arch {
        crate::platform::Architecture::Aarch64 => "aarch64",
        crate::platform::Architecture::X86_64 => "amd64",
        crate::platform::Architecture::X86 => "x86",
    };
    let path = directory.join("java");
    fs::write(
        &path,
        format!("#!/bin/sh\necho 'java.version = 17.0.9' >&2\necho 'os.arch = {arch}' >&2\n"),
    )?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

#[cfg(unix)]
fn use_offline_manifest(library: &mut Library) -> anyhow::Result<()> {
    let store = Store::with_options(
        library.storage.cache.clone(),
        true,
        Arc::new(|_| {}),
        options(),
    )?;
    let manifest = version_manifest()?;
    let manifest_sha1 = sha1_hex(&manifest);
    let metadata = store.root().join("metadata");
    fs::create_dir_all(&metadata)?;
    fs::write(
        metadata.join("version_manifest_v2.json"),
        serde_json::to_vec(&json!({
            "latest": {"release": MINECRAFT_VERSION, "snapshot": MINECRAFT_VERSION},
            "versions": [{
                "id": MINECRAFT_VERSION,
                "type": "release",
                "url": "https://example.invalid/version.json",
                "time": "2024-01-01T00:00:00+00:00",
                "releaseTime": "2024-01-01T00:00:00+00:00",
                "sha1": manifest_sha1,
                "complianceLevel": 1
            }]
        }))?,
    )?;
    fs::write(
        metadata.join(format!("version-{manifest_sha1}.json")),
        manifest,
    )?;
    for (_, _, bytes) in LIBRARIES {
        seed_object(&store, bytes)?;
    }
    seed_object(&store, CLIENT_JAR)?;
    seed_object(&store, ASSET_INDEX)?;
    library.executor = crate::executor::Executor::new(store.clone(), library.events.clone());
    library.store = store;
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn curseforge_without_selected_ids_completes_the_real_install_offline() -> anyhow::Result<()>
{
    let temporary = tempfile::tempdir()?;
    let mut library = library_at(temporary.path())?;
    let java = synthetic_java(temporary.path())?;
    let source = temporary.path().join("curse.zip");
    write_curseforge_pack(&source, None, &[], &[(101, 202)])?;
    let inspected = modpack::inspect(&source)?;
    let instance = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &PackSelection::default(),
        configuration_with_java("Curse Pack", &java),
        &CancellationToken::new(),
    )?;
    use_offline_manifest(&mut library)?;

    library
        .install_pack(&instance.id, &source, &CancellationToken::new())
        .await?;
    let snapshot = library.catalog().snapshot()?;
    let (current, target) = snapshot.selection(&instance.id)?;
    assert_eq!(
        current.pack.as_ref().map(|pack| pack.status),
        Some(PackStatus::Ready)
    );
    assert_eq!(target.status, InstallStatus::Ready);
    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };
    let plan = library
        .prepare_launch(
            &instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    assert_eq!(
        fs::read(plan.game_directory().join("config/example.toml"))?,
        b"value"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn modrinth_completes_real_install_and_launch_preparation_from_verified_cache()
-> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let mut library = library_at(temporary.path())?;
    let java = synthetic_java(temporary.path())?;
    let source = temporary.path().join("pack.mrpack");
    write_mrpack(
        &source,
        &[REQUIRED_URL.to_owned()],
        &[OPTIONAL_URL.to_owned()],
    )?;
    let inspected = modpack::inspect(&source)?;
    let instance = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &optional_selection(),
        configuration_with_java("Modrinth Pack", &java),
        &CancellationToken::new(),
    )?;
    use_offline_manifest(&mut library)?;
    seed_object(&library.store, REQUIRED_BYTES)?;
    seed_object(&library.store, OPTIONAL_BYTES)?;
    library
        .install_pack(&instance.id, &source, &CancellationToken::new())
        .await?;
    let snapshot = library.catalog().snapshot()?;
    let (current, target) = snapshot.selection(&instance.id)?;
    assert_eq!(
        current.pack.as_ref().map(|pack| pack.status),
        Some(PackStatus::Ready)
    );
    assert_eq!(target.status, InstallStatus::Ready);
    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };
    let plan = library
        .prepare_launch(
            &instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    let game = plan.game_directory();
    assert_eq!(fs::read(game.join(REQUIRED_PATH))?, REQUIRED_BYTES);
    assert_eq!(fs::read(game.join(OPTIONAL_PATH))?, OPTIONAL_BYTES);
    assert_eq!(fs::read(game.join("config/pack.toml"))?, b"client-layer");
    Ok(())
}

/// An installed pack whose baseline, loader metadata and offline cache are in
/// place, so the real `Library::prepare_launch` can resolve a complete plan.
#[cfg(unix)]
struct LaunchFixture {
    _temporary: tempfile::TempDir,
    library: Library,
    instance: Instance,
    java: PathBuf,
}

#[cfg(unix)]
async fn launch_fixture(plain: bool) -> anyhow::Result<LaunchFixture> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let java = synthetic_java(temporary.path())?;
    let source = temporary.path().join(if plain {
        "plain.zip"
    } else {
        "synthetic.mrpack"
    });
    if plain {
        write_archive(
            &source,
            vec![
                ("options.txt", BASELINE_OPTIONS.to_vec()),
                ("mods/example.jar", BASELINE_MOD.to_vec()),
                ("config/pack.toml", BASELINE_CONFIG.to_vec()),
                ("logs/latest.log", b"excluded log".to_vec()),
                (
                    "launcher_accounts.json",
                    b"excluded account marker".to_vec(),
                ),
            ],
        )?;
    } else {
        write_mrpack(
            &source,
            &[REQUIRED_URL.to_owned()],
            &[OPTIONAL_URL.to_owned()],
        )?;
    }
    let mut inspected = modpack::inspect(&source)?;
    let selection = if plain {
        assert!(!inspected.target_confirmed());
        inspected.select_target(MINECRAFT_VERSION.into(), None)?;
        PackSelection::default()
    } else {
        optional_selection()
    };
    let instance = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &selection,
        configuration_with_java("Pack baseline", &java),
        &CancellationToken::new(),
    )?;

    let baseline = instance
        .resource_directory
        .join(".block-pack")
        .join("baseline");
    if plain {
        let descriptor = modpack::descriptor::load(&instance.resource_directory)?;
        let source_inspection = modpack::inspect(&source)?;
        modpack::prepare::prepare(
            &download_store(&library)?,
            &instance,
            &descriptor,
            &source_inspection,
            &CancellationToken::new(),
        )
        .await?;
        let (restored, _) = library.inspect_pack(&source, Some(&instance.id))?;
        assert!(restored.target_confirmed());
        assert_eq!(
            restored.minecraft_version.as_deref(),
            Some(MINECRAFT_VERSION)
        );
    } else {
        // Fixture: the verified baseline an accepted install publishes before launch.
        fs::create_dir_all(baseline.join("mods"))?;
        fs::create_dir_all(baseline.join("config"))?;
        fs::write(baseline.join("options.txt"), BASELINE_OPTIONS)?;
        fs::write(baseline.join("mods").join("example.jar"), BASELINE_MOD)?;
        fs::write(baseline.join("config").join("pack.toml"), BASELINE_CONFIG)?;
    }

    // A synthetic version document resolved entirely from the offline cache.
    let installed = temporary.path().join("installation.json");
    fs::write(&installed, version_manifest()?)?;
    let store = download_store(&library)?;
    for (_, _, bytes) in LIBRARIES {
        seed_object(&store, bytes)?;
    }
    seed_object(&store, CLIENT_JAR)?;
    seed_object(&store, ASSET_INDEX)?;

    let mut target = instance.targets[0].clone();
    target.source = TargetSource::Managed {
        manifest: Some(installed.clone()),
    };
    let mut recipe = super::recipe(
        &instance,
        crate::recipe::RecipeSource::LocalManifest { path: installed },
    );
    let cancel = CancellationToken::new();
    let context = crate::resolver::ResolveContext {
        store: Store::with_options(
            library.catalog().root().to_owned(),
            true,
            Arc::new(|_| {}),
            options(),
        )?,
        cancel: cancel.clone(),
    };
    let metadata = super::resolve_for_host(&mut recipe, &context).await?;
    let prepared = library
        .executor
        .prepare_repair(crate::plan::compile_catalog(&recipe, &metadata)?, &cancel)
        .await?;
    drop(prepared);
    target.status = InstallStatus::Ready;
    library.catalog().set_target(&instance.id, &target)?;
    library
        .catalog()
        .finish_pack_install(&instance.id, &target)?;
    Ok(LaunchFixture {
        _temporary: temporary,
        library,
        instance,
        java,
    })
}

#[cfg(unix)]
fn assert_is_baseline(game: &Path) -> anyhow::Result<()> {
    assert_eq!(fs::read(game.join("options.txt"))?, BASELINE_OPTIONS);
    assert_eq!(
        fs::read(game.join("mods").join("example.jar"))?,
        BASELINE_MOD
    );
    assert_eq!(
        fs::read(game.join("config").join("pack.toml"))?,
        BASELINE_CONFIG
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn launch_preparation_seeds_each_new_game_directory_from_the_pack_baseline()
-> anyhow::Result<()> {
    let fixture = launch_fixture(false).await?;
    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };
    let plan = fixture
        .library
        .prepare_launch(
            &fixture.instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    let game = plan.game_directory().to_owned();
    assert_eq!(game, fixture.instance.game_directory.clone());
    assert!(
        fixture
            .library
            .catalog()
            .root()
            .join("preparations")
            .join(&fixture.instance.id)
            .join("prepared.lock.json")
            .is_file()
    );
    for name in ["game", "runs", "instance.lock", "prepared.lock.json"] {
        assert!(
            !game.join(name).exists(),
            "preparation leaked into game data: {name}"
        );
    }
    assert_is_baseline(&game)?;
    let preview = plan.preview();
    assert_eq!(preview.program, fixture.java.canonicalize()?);
    assert_eq!(preview.cwd, game);
    assert!(
        preview
            .arguments
            .iter()
            .any(|argument| argument.ends_with("client.jar")),
        "the prepared classpath is missing its materialized client jar"
    );
    drop(plan);

    // Played data is not a source: a fresh selected directory uses the baseline.
    fs::write(game.join("options.txt"), b"played-options")?;
    let second_game = fixture.instance.resource_directory.join("another-game");
    fixture.library.catalog().save_instance(
        &fixture.instance.id,
        InstanceConfiguration {
            game_directory: second_game.clone(),
            ..InstanceConfiguration::from(&fixture.instance)
        },
    )?;
    let plan = fixture
        .library
        .prepare_launch(
            &fixture.instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    assert_eq!(plan.game_directory(), second_game);
    assert_ne!(second_game, game);
    assert_is_baseline(&second_game)?;
    assert_eq!(fs::read(game.join("options.txt"))?, b"played-options");
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn plain_archive_reaches_the_actual_instance_game_directory() -> anyhow::Result<()> {
    let fixture = launch_fixture(true).await?;
    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };
    let plan = fixture
        .library
        .prepare_launch(
            &fixture.instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    let game = plan.game_directory();
    assert_eq!(game, fixture.instance.game_directory.clone());
    assert_is_baseline(game)?;
    assert!(!game.join("logs").exists());
    assert!(!game.join("launcher_accounts.json").exists());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn switching_an_installed_pack_directory_keeps_each_game_directory() -> anyhow::Result<()> {
    let fixture = launch_fixture(false).await?;
    let session = OfflineSession::Offline {
        username: "PackPlayer".into(),
    };
    let mut configuration = InstanceConfiguration::from(&fixture.instance);
    let plan = fixture
        .library
        .prepare_launch(
            &fixture.instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    let isolated = plan.game_directory().to_owned();
    drop(plan);
    fs::write(isolated.join("options.txt"), b"isolated-played")?;

    let shared_directory = fixture.instance.resource_directory.join("shared-game");
    fs::create_dir(&shared_directory)?;
    configuration.data_policy = DataPolicy::Shared;
    configuration.game_directory = shared_directory.clone();
    fixture
        .library
        .catalog()
        .save_instance(&fixture.instance.id, configuration.clone())?;
    let plan = fixture
        .library
        .prepare_launch(
            &fixture.instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    let shared = plan.game_directory().to_owned();
    drop(plan);
    assert_eq!(shared, shared_directory);
    assert_is_baseline(&shared)?;
    assert_eq!(fs::read(isolated.join("options.txt"))?, b"isolated-played");

    configuration.data_policy = DataPolicy::Isolated;
    configuration.game_directory = isolated.clone();
    fixture
        .library
        .catalog()
        .save_instance(&fixture.instance.id, configuration)?;
    let plan = fixture
        .library
        .prepare_launch(
            &fixture.instance.id,
            &session,
            &CancellationToken::new(),
            None,
            None,
        )
        .await?;
    assert_eq!(plan.game_directory(), isolated.as_path());
    assert_eq!(
        fs::read(isolated.join("options.txt"))?,
        b"isolated-played",
        "the published game directory is never rewritten"
    );
    Ok(())
}

#[tokio::test]
async fn prepare_materializes_verified_files_with_overlay_precedence_and_alternate_source()
-> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let mut routes = BTreeMap::new();
    routes.insert("/alternate/example.jar".to_owned(), REQUIRED_BYTES.to_vec());
    routes.insert("/optional.toml".to_owned(), OPTIONAL_BYTES.to_vec());
    let server = server(routes).await?;
    let source = temporary.path().join("synthetic.mrpack");
    write_mrpack(
        &source,
        &[
            format!("{}/missing/example.jar", server.url),
            format!("{}/alternate/example.jar", server.url),
        ],
        &[format!("{}/optional.toml", server.url)],
    )?;
    let inspected = modpack::inspect(&source)?;
    let instance = library.create_pack(
        &uuid::Uuid::new_v4().to_string(),
        &source,
        &inspected,
        &optional_selection(),
        configuration("Pack"),
        &CancellationToken::new(),
    )?;
    let descriptor = modpack::descriptor::load(&instance.resource_directory)?;
    let store = download_store(&library)?;
    modpack::prepare::prepare(
        &store,
        &instance,
        &descriptor,
        &inspected,
        &CancellationToken::new(),
    )
    .await?;

    // Selected files are verified by SHA-1, SHA-512 and size, then layered.
    let baseline = instance
        .resource_directory
        .join(".block-pack")
        .join("baseline");
    assert_eq!(fs::read(baseline.join(REQUIRED_PATH))?, REQUIRED_BYTES);
    assert_eq!(fs::read(baseline.join(OPTIONAL_PATH))?, OPTIONAL_BYTES);
    assert_eq!(
        fs::read(baseline.join("config").join("pack.toml"))?,
        b"client-layer",
        "client overrides layer over overrides"
    );
    assert_eq!(
        fs::read(baseline.join("config").join("keep.toml"))?,
        b"kept-layer"
    );

    // The rejected primary source is followed by the declared alternate.
    let requests = server.requests.lock().await.clone();
    assert!(
        requests.contains(&"/missing/example.jar".to_owned()),
        "{requests:?}"
    );
    assert!(
        requests.contains(&"/alternate/example.jar".to_owned()),
        "{requests:?}"
    );
    Ok(())
}

#[tokio::test]
async fn prepare_blocks_the_baseline_on_cancellation_or_a_wrong_hash_and_resumes()
-> anyhow::Result<()> {
    let temporary = tempfile::tempdir()?;
    let library = library_at(temporary.path())?;
    let mut routes = BTreeMap::new();
    routes.insert("/example.jar".to_owned(), b"tampered".to_vec());
    routes.insert("/optional.toml".to_owned(), OPTIONAL_BYTES.to_vec());
    let broken = server(routes).await?;
    let source = temporary.path().join("synthetic.mrpack");
    write_mrpack(
        &source,
        &[format!("{}/example.jar", broken.url)],
        &[format!("{}/optional.toml", broken.url)],
    )?;
    let inspected = modpack::inspect(&source)?;
    let selection = optional_selection();
    let operation = uuid::Uuid::new_v4().to_string();
    let instance = library.create_pack(
        &operation,
        &source,
        &inspected,
        &selection,
        configuration("Pack"),
        &CancellationToken::new(),
    )?;
    let descriptor = modpack::descriptor::load(&instance.resource_directory)?;
    let store = download_store(&library)?;
    let baseline = instance
        .resource_directory
        .join(".block-pack")
        .join("baseline");

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let error = modpack::prepare::prepare(&store, &instance, &descriptor, &inspected, &cancelled)
        .await
        .err()
        .context("a cancelled preparation published the baseline")?;
    assert!(
        format!("{error:#}").contains("pack installation cancelled"),
        "{error:#}"
    );
    assert!(!baseline.exists());

    let error = modpack::prepare::prepare(
        &store,
        &instance,
        &descriptor,
        &inspected,
        &CancellationToken::new(),
    )
    .await
    .err()
    .context("a tampered file satisfied its declared digests")?;
    assert!(
        format!("{error:#}").contains(&sha1_hex(REQUIRED_BYTES)),
        "{error:#}"
    );
    assert!(!baseline.exists());

    // The failed install leaves one catalog entry, and the import resumes it.
    let resumed = library.create_pack(
        &operation,
        &source,
        &inspected,
        &selection,
        configuration("Pack"),
        &CancellationToken::new(),
    )?;
    assert_eq!(resumed.id, instance.id);
    let state = library.catalog().snapshot()?;
    assert_eq!(state.instances.len(), 1);
    assert_eq!(state.selection(&instance.id)?.0.id, instance.id);
    assert_eq!(
        state
            .selection(&instance.id)?
            .0
            .pack
            .as_ref()
            .context("pack")?
            .status,
        PackStatus::Pending
    );

    // A corrected source publishes the same baseline on retry.
    let mut routes = BTreeMap::new();
    routes.insert("/example.jar".to_owned(), REQUIRED_BYTES.to_vec());
    routes.insert("/optional.toml".to_owned(), OPTIONAL_BYTES.to_vec());
    let fixed = server(routes).await?;
    write_mrpack(
        &source,
        &[format!("{}/example.jar", fixed.url)],
        &[format!("{}/optional.toml", fixed.url)],
    )?;
    let reinspected = modpack::inspect(&source)?;
    modpack::prepare::prepare(
        &store,
        &instance,
        &descriptor,
        &reinspected,
        &CancellationToken::new(),
    )
    .await?;
    assert_eq!(fs::read(baseline.join(REQUIRED_PATH))?, REQUIRED_BYTES);
    Ok(())
}
