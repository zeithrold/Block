use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use serde::Serialize;

const RUN_ID_TOKEN: &str = "${run_id}";

#[cfg(test)]
mod tests;

use crate::{
    executor::confined_path,
    plan::{ExecutionPlan, JavaExecutable},
    platform::OperatingSystem,
    session::SessionProvider,
};

#[derive(Clone)]
struct Argument {
    value: String,
    secret: bool,
}

/// The final Java invocation. Secret values are excluded from Debug and previews.
#[derive(Clone)]
pub struct JavaCommand {
    pub program: PathBuf,
    pub cwd: PathBuf,
    arguments: Vec<Argument>,
    secrets: Vec<String>,
    environment: BTreeMap<String, String>,
}

impl fmt::Debug for JavaCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.preview().fmt(f)
    }
}

#[derive(Debug, Serialize)]
pub struct CommandPreview {
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub arguments: Vec<String>,
}

impl JavaCommand {
    /// Bind a session to a prepared command template without invoking a shell.
    ///
    /// # Errors
    /// Fails if required variables, paths, or account fields are unavailable.
    pub fn bind(
        plan: &ExecutionPlan,
        root: &Path,
        run_id: &str,
        provider: &dyn SessionProvider,
    ) -> anyhow::Result<Self> {
        Self::bind_for_directory(plan, root, run_id, provider, None)
    }

    /// Resolve mutable game data separately from the shared installation cache.
    /// # Errors
    /// Fails on invalid identity, unsupported paths, or missing substitutions.
    pub fn bind_for_directory(
        plan: &ExecutionPlan,
        root: &Path,
        run_id: &str,
        provider: &dyn SessionProvider,
        game_directory: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let session = provider.session()?;
        session.validate()?;
        let offline = session.user_type == "msa" && session.access_token == "0";
        let mut secrets = if offline {
            Vec::new()
        } else {
            vec![session.access_token.clone()]
        };
        if session.user_properties != "{}" {
            secrets.push(session.user_properties.clone());
        }
        let template = &plan.command;
        let mut variables = template.variables.clone();
        variables.extend([
            ("auth_player_name".into(), session.username),
            ("auth_uuid".into(), session.uuid.clone()),
            ("auth_access_token".into(), session.access_token.clone()),
            (
                "auth_session".into(),
                if offline {
                    "0".into()
                } else if session.user_type == "mojang" {
                    session.access_token.clone()
                } else {
                    format!("token:{}:{}", session.access_token, session.uuid)
                },
            ),
            ("user_type".into(), session.user_type),
            ("auth_xuid".into(), session.xuid),
            ("clientid".into(), session.client_id),
            ("user_properties".into(), session.user_properties),
        ]);
        for (name, path) in &template.path_variables {
            let path = path.replace(RUN_ID_TOKEN, run_id);
            variables.insert(
                name.clone(),
                java_path(confined_path(root, &path)?, plan.target.os)?
                    .to_str()
                    .context("Java requires UTF-8 argument paths")?
                    .into(),
            );
        }
        if let Some(directory) = game_directory {
            let directory = java_path(directory.to_owned(), plan.target.os)?;
            variables.insert(
                "game_directory".into(),
                directory
                    .to_str()
                    .context("game directory must be UTF-8")?
                    .into(),
            );
        }
        let separator = if plan.target.os == OperatingSystem::Windows {
            ";"
        } else {
            ":"
        };
        let classpath = template
            .classpath
            .iter()
            .map(|path| {
                Ok(java_path(confined_path(root, path)?, plan.target.os)?
                    .to_str()
                    .context("classpath is not UTF-8")?
                    .to_owned())
            })
            .collect::<anyhow::Result<Vec<_>>>()?
            .join(separator);
        variables.insert("classpath".into(), classpath);
        let mut arguments = template
            .jvm_args
            .iter()
            .map(|value| substitute(value, &variables))
            .collect::<anyhow::Result<Vec<_>>>()?;
        arguments.push(Argument {
            value: template.main_class.clone(),
            secret: false,
        });
        for value in &template.game_args {
            arguments.push(substitute(value, &variables)?);
        }
        let program = match &template.java {
            JavaExecutable::Managed { path } => confined_path(root, path)?,
            JavaExecutable::Local { path } => path
                .canonicalize()
                .with_context(|| format!("Java not found: {}", path.display()))?,
        };
        Ok(Self {
            program: java_path(program, plan.target.os)?,
            cwd: java_path(
                game_directory.map_or_else(
                    || confined_path(root, &template.cwd),
                    |path| Ok(path.to_owned()),
                )?,
                plan.target.os,
            )?,
            arguments,
            secrets,
            environment: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn preview(&self) -> CommandPreview {
        CommandPreview {
            program: self.program.clone(),
            cwd: self.cwd.clone(),
            arguments: self
                .arguments
                .iter()
                .map(|arg| {
                    if arg.secret {
                        "<redacted>".into()
                    } else {
                        arg.value.clone()
                    }
                })
                .collect(),
        }
    }

    /// Set explicit process-local host overrides on this transient invocation.
    /// # Errors
    /// Rejects invalid environment names and values.
    pub fn set_environment(&mut self, environment: BTreeMap<String, String>) -> anyhow::Result<()> {
        ensure!(
            environment.iter().all(|(key, value)| !key.is_empty()
                && !key.contains(['=', '\0'])
                && !value.contains('\0')
                && !key.to_ascii_uppercase().contains("TOKEN")
                && !key.to_ascii_uppercase().contains("SECRET")),
            "invalid or credential-bearing game environment override"
        );
        self.environment = environment;
        Ok(())
    }

    #[must_use]
    pub const fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    #[must_use]
    pub fn process(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .current_dir(&self.cwd)
            .args(self.arguments.iter().map(|arg| &arg.value))
            .envs(&self.environment)
            .env_remove("MICROSOFT_CLIENT_SECRET");
        command
    }

    #[must_use]
    pub fn redact(&self, line: &str) -> String {
        let mut line = line.to_owned();
        for argument in &self.arguments {
            if argument.secret && argument.value.len() > 1 {
                line = line.replace(&argument.value, "<redacted>");
            }
        }
        for secret in &self.secrets {
            if !secret.is_empty() {
                line = line.replace(secret, "<redacted>");
            }
        }
        line
    }
}

fn java_path(path: PathBuf, os: OperatingSystem) -> anyhow::Result<PathBuf> {
    if os != OperatingSystem::Windows {
        return Ok(path);
    }
    let value = path.to_str().context("Java requires UTF-8 Windows paths")?;
    Ok(windows_java_path(value).into())
}

// Rust canonical paths use the Win32 verbatim namespace. Java's class loader
// treats its question mark as URL syntax and silently skips these JAR paths.
// Keep canonical paths for filesystem confinement; adapt only the Java command.
fn windows_java_path(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    if let Some(drive) = path.strip_prefix(r"\\?\")
        && drive.as_bytes().get(1) == Some(&b':')
        && drive.as_bytes().get(2) == Some(&b'\\')
    {
        return drive.to_owned();
    }
    path.to_owned()
}

fn substitute(template: &str, variables: &BTreeMap<String, String>) -> anyhow::Result<Argument> {
    let mut rest = template;
    let mut value = String::new();
    let mut secret = false;
    while let Some(start) = rest.find("${") {
        value.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        let end = rest.find('}').context("unterminated launch placeholder")?;
        let name = &rest[..end];
        value.push_str(
            variables
                .get(name)
                .with_context(|| format!("missing launch placeholder: {name}"))?,
        );
        secret |= matches!(
            name,
            "auth_access_token" | "auth_session" | "user_properties"
        );
        rest = &rest[end + 1..];
    }
    value.push_str(rest);
    ensure!(!value.contains('\0'), "Java argument contains a NUL byte");
    Ok(Argument { value, secret })
}
