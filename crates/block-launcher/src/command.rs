use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, ensure};
use serde::Serialize;

const RUN_ID_TOKEN: &str = "${run_id}";

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
        let session = provider.session()?;
        let template = &plan.command;
        let mut variables = template.variables.clone();
        variables.extend([
            ("auth_player_name".into(), session.username),
            ("auth_uuid".into(), session.uuid.clone()),
            ("auth_access_token".into(), session.access_token.clone()),
            (
                "auth_session".into(),
                if session.access_token == "0" {
                    "0".into()
                } else {
                    format!("token:{}:{}", session.access_token, session.uuid)
                },
            ),
            ("user_type".into(), session.user_type),
            ("auth_xuid".into(), session.xuid),
            ("clientid".into(), session.client_id),
        ]);
        for (name, path) in &template.path_variables {
            let path = path.replace(RUN_ID_TOKEN, run_id);
            variables.insert(
                name.clone(),
                confined_path(root, &path)?
                    .to_str()
                    .context("Java requires UTF-8 argument paths")?
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
                Ok(confined_path(root, path)?
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
            program,
            cwd: confined_path(root, &template.cwd)?,
            arguments,
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

    #[must_use]
    pub fn process(&self) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .current_dir(&self.cwd)
            .args(self.arguments.iter().map(|arg| &arg.value));
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
        line
    }
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
        secret |= name == "auth_access_token" || name == "auth_session";
        rest = &rest[end + 1..];
    }
    value.push_str(rest);
    ensure!(!value.contains('\0'), "Java argument contains a NUL byte");
    Ok(Argument { value, secret })
}
