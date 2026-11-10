use anyhow::{Context as _, ensure};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn merge(parent: &mut Value, mut child: Value) -> anyhow::Result<()> {
    let child = child
        .as_object_mut()
        .context("Invalid inherited document")?;
    let parent = parent.as_object_mut().context("Invalid base document")?;
    if let Some(libraries) = child.remove("libraries") {
        let mut selected = libraries
            .as_array()
            .context("Invalid inherited libraries")?
            .clone();
        let keys = selected
            .iter()
            .map(library_key)
            .collect::<anyhow::Result<BTreeSet<_>>>()?;
        for library in parent
            .remove("libraries")
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
        {
            if !keys.contains(&library_key(&library)?) {
                selected.push(library);
            }
        }
        parent.insert("libraries".into(), Value::Array(selected));
    }
    if let Some(arguments) = child.remove("arguments") {
        if !parent.contains_key("arguments") {
            let game = parent
                .remove("minecraftArguments")
                .and_then(|value| value.as_str().map(str::to_owned))
                .context("Inherited launch arguments are missing")?;
            parent.insert("arguments".into(), serde_json::json!({"game": super::super::arguments::tokenize(&game)?, "jvm":["-Djava.library.path=${natives_directory}","-cp","${classpath}"]}));
        }
        let inherited = parent
            .get_mut("arguments")
            .and_then(Value::as_object_mut)
            .context("Invalid base arguments")?;
        for (key, values) in arguments.as_object().context("Invalid loader arguments")? {
            ensure!(
                matches!(key.as_str(), "game" | "jvm" | "default-user-jvm"),
                "Unsupported loader argument section: {key}"
            );
            let base = inherited
                .entry(key.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .context("Invalid inherited argument list")?;
            base.extend(
                values
                    .as_array()
                    .context("Invalid loader argument list")?
                    .iter()
                    .cloned(),
            );
        }
    } else if child.contains_key("minecraftArguments") {
        parent.remove("arguments");
    }
    child.remove("inheritsFrom");
    child.remove("jar");
    parent.extend(child.clone());
    Ok(())
}

fn library_key(library: &Value) -> anyhow::Result<String> {
    let name = library
        .get("name")
        .and_then(Value::as_str)
        .context("Library coordinate is missing")?;
    let mut parts = name.split(':');
    let group = parts.next().context("Missing library group")?;
    let artifact = parts.next().context("Missing library artifact")?;
    parts.next().context("Missing library version")?;
    Ok(format!(
        "{group}:{artifact}:{}",
        parts.collect::<Vec<_>>().join(":")
    ))
}
