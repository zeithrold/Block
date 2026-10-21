use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, ensure};
use block_launcher::{
    CancellationToken, DownloadEvent, Store,
    command::JavaCommand,
    executor::{ExecutionEvent, Executor},
    plan::{ExecutionPlan, compile},
    recipe::LaunchRecipe,
    resolver::{ResolveContext, resolve},
    session::OfflineSession,
};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Compile launch recipes into verified resource plans and complete Java commands"
)]
struct Cli {
    /// All downloaded files, instance data and logs live outside the source checkout.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Disable network IO; launch still uses the recipe's independent account mode.
    #[arg(long, global = true)]
    cache_only: bool,
    #[command(subcommand)]
    command: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    /// Resolve metadata and print/save the complete secret-free execution plan.
    Plan {
        #[arg(long)]
        recipe: PathBuf,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Download and materialize a recipe without starting Java.
    Prepare {
        #[arg(long)]
        recipe: PathBuf,
    },
    /// Prepare and launch a recipe, streaming game logs until exit or Ctrl+C.
    Launch {
        #[arg(long)]
        recipe: PathBuf,
        /// Force-stop after a limit; intended for explicit GUI smoke checks.
        #[arg(long)]
        run_for_seconds: Option<u64>,
    },
    /// Execute a previously compiled plan, with identity bound at execution time.
    Execute {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long, default_value = "Player")]
        username: String,
        #[arg(long)]
        run_for_seconds: Option<u64>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = cli.data_dir.map_or_else(
        || {
            directories::ProjectDirs::from("dev", "zeithrold", "Block")
                .map(|dirs| dirs.data_local_dir().to_owned())
                .context("cannot determine the application data directory")
        },
        Ok,
    )?;
    let _storage = block_launcher::storage::acquire(&root)?;
    let completed = Arc::new(AtomicUsize::new(0));
    let store = Store::new(
        root,
        cli.cache_only,
        Arc::new(move |event| {
            if let DownloadEvent::Complete { .. } = event {
                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(100) {
                    eprintln!("Verified {count} resource objects");
                }
            }
        }),
    )?;
    let cancel = CancellationToken::new();
    let interrupt = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupt.cancel();
        }
    });
    let executor = Executor::new(
        store.clone(),
        Arc::new(|event| match event {
            ExecutionEvent::Log { line, stderr } => {
                if stderr {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
            }
            event => eprintln!("{event:?}"),
        }),
    );
    match cli.command {
        Action::Plan { recipe, output } => {
            let (_, plan) = compile_recipe(recipe, store, cancel).await?;
            let json = serde_json::to_string_pretty(&plan)?;
            if let Some(path) = output {
                tokio::fs::write(path, json).await?;
            } else {
                println!("{json}");
            }
        }
        Action::Prepare { recipe } => {
            let (_, plan) = compile_recipe(recipe, store, cancel.clone()).await?;
            let prepared = executor.prepare(plan, &cancel).await?;
            eprintln!(
                "Prepared {} at {}",
                prepared.plan.version,
                prepared.root.display()
            );
        }
        Action::Launch {
            recipe,
            run_for_seconds,
        } => {
            let (recipe, plan) = compile_recipe(recipe, store, cancel.clone()).await?;
            launch(&executor, plan, recipe.session, cancel, run_for_seconds).await?;
        }
        Action::Execute {
            plan,
            username,
            run_for_seconds,
        } => {
            let plan: ExecutionPlan = serde_json::from_slice(&tokio::fs::read(plan).await?)?;
            launch(
                &executor,
                plan,
                OfflineSession::Offline { username },
                cancel,
                run_for_seconds,
            )
            .await?;
        }
    }
    Ok(())
}

async fn compile_recipe(
    path: PathBuf,
    store: Store,
    cancel: CancellationToken,
) -> anyhow::Result<(LaunchRecipe, ExecutionPlan)> {
    let recipe = LaunchRecipe::load(&path).await?;
    eprintln!("Resolving recipe for instance {}", recipe.instance);
    let context = ResolveContext { store, cancel };
    let metadata = resolve(&recipe, &context).await?;
    let plan = compile(&recipe, &metadata)?;
    Ok((recipe, plan))
}

async fn launch(
    executor: &Executor,
    plan: ExecutionPlan,
    identity: OfflineSession,
    cancel: CancellationToken,
    limit: Option<u64>,
) -> anyhow::Result<()> {
    let prepared = executor.prepare(plan, &cancel).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &JavaCommand::bind(&prepared.plan, &prepared.root, &prepared.run_id, &identity)?
                .preview()
        )?
    );
    if let Some(seconds) = limit {
        let timeout = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            timeout.cancel();
        });
    }
    let code = executor.launch(prepared, &identity, &cancel).await?;
    ensure!(
        cancel.is_cancelled() || code == Some(0),
        "Minecraft exited unsuccessfully: {code:?}"
    );
    Ok(())
}
