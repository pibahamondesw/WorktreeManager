//! Rewrites the `steps` of an `AUTOMATION.md` from the graph editor. Only the frontmatter's
//! `steps` block changes: the rest of the frontmatter, its comments and the Markdown body stay byte
//! for byte, and keys the editor does not know stay in each step in their order. Comments inside
//! the `steps` block are not kept. A save is validated like the file and refused when the file
//! changed since the editor read it.

#[cfg(test)]
mod tests;

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use serde_norway::{Mapping, Value as Yaml};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Runtime};

use super::fields::{is_automation_name, Report};
use super::{
    load_automation, parse_definition, split_frontmatter, AutomationEntry, DefinitionIssue,
    Environment, Sources, DEFINITION_FILE,
};
use crate::commands::automations::store::write_atomically;

const STEPS_KEY: &str = "steps";
const NEW_KEY_ORDER: [&str; 20] = [
    "id",
    "label",
    "agent",
    "command",
    "wait",
    "approval",
    "publish",
    "model",
    "effort",
    "skill",
    "prompt",
    "title",
    "detail",
    "repo",
    "next",
    "on_fail",
    "on_reject",
    "goto",
    "max",
    "timeout",
];
const CHANGED_ON_DISK: &str =
    "AUTOMATION.md changed on disk since the editor loaded it; reload to edit the current version";
const UNSAFE_EDIT: &str =
    "Could not change the steps without touching the rest of the frontmatter; edit AUTOMATION.md by hand";

#[derive(Debug, PartialEq, Serialize)]
pub struct StepsSource {
    /// SHA-256 of the file the steps were read from.
    pub hash: String,
    /// The `steps` as written, empty when the file has none.
    pub steps: Vec<Json>,
}

#[derive(Debug, Deserialize)]
pub struct StepEdit {
    /// Position in the loaded `steps` of the step this one edits; `None` for a new step.
    pub source: Option<usize>,
    pub value: Json,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StepsSave {
    Saved {
        entry: Box<AutomationEntry>,
    },
    /// Nothing was written.
    Invalid {
        errors: Vec<DefinitionIssue>,
    },
}

pub fn read_steps(dir: &Path, name: &str) -> Result<StepsSource, String> {
    let text = read_definition(&definition_file(dir, name)?)?;
    let (frontmatter, _) = split_frontmatter(&text).ok_or(UNSAFE_EDIT)?;
    let steps = written_steps(&frontmatter_map(frontmatter)?)?
        .iter()
        .map(|step| serde_json::to_value(step).map_err(|error| error.to_string()))
        .collect::<Result<_, _>>()?;
    Ok(StepsSource {
        hash: text_hash(&text),
        steps,
    })
}

pub fn save_steps(
    dir: &Path,
    name: &str,
    env: &Environment,
    hash: &str,
    edits: &[StepEdit],
) -> Result<StepsSave, String> {
    let file = definition_file(dir, name)?;
    let text = read_definition(&file)?;
    if text_hash(&text) != hash {
        return Err(CHANGED_ON_DISK.into());
    }
    let edited = edited_text(&text, edits)?;
    let folder = dir.join(name);
    let mut report = Report::new(file.display().to_string());
    parse_definition(&mut report, &folder, name, env, &edited);
    if !report.errors.is_empty() {
        return Ok(StepsSave::Invalid {
            errors: report.errors,
        });
    }
    let target = fs::canonicalize(&file).map_err(|error| error.to_string())?;
    write_atomically(&target, edited.as_bytes()).map_err(|error| error.to_string())?;
    Ok(StepsSave::Saved {
        entry: Box::new(load_automation(&folder, env)),
    })
}

fn definition_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let file = dir.join(name).join(DEFINITION_FILE);
    if !is_automation_name(name) || !file.is_file() {
        return Err(format!("No automation named {name}"));
    }
    Ok(file)
}

fn read_definition(file: &Path) -> Result<String, String> {
    fs::read_to_string(file).map_err(|error| format!("Could not read {}: {error}", file.display()))
}

fn text_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn frontmatter_map(frontmatter: &str) -> Result<Mapping, String> {
    match serde_norway::from_str::<Yaml>(frontmatter) {
        Ok(Yaml::Mapping(map)) => Ok(map),
        Ok(_) => Err("The frontmatter must be a YAML map".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn written_steps(map: &Mapping) -> Result<Vec<Yaml>, String> {
    match map.get(STEPS_KEY) {
        None | Some(Yaml::Null) => Ok(Vec::new()),
        Some(Yaml::Sequence(steps)) => Ok(steps.clone()),
        Some(_) => Err("steps must be a list to edit it from the graph".into()),
    }
}

/// `text` with its frontmatter's `steps` replaced by `edits`, or an error when the rest of the
/// frontmatter would not read the same afterwards.
fn edited_text(text: &str, edits: &[StepEdit]) -> Result<String, String> {
    let (frontmatter, _) = split_frontmatter(text).ok_or(UNSAFE_EDIT)?;
    let mut map = frontmatter_map(frontmatter)?;
    let written = written_steps(&map)?;
    let steps = edits
        .iter()
        .map(|edit| {
            merged(
                edit.source.and_then(|index| written.get(index)),
                &edit.value,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let block = steps_block(&steps, frontmatter.contains("\r\n"))?;
    let rewritten = spliced(frontmatter, &block);
    map.insert(STEPS_KEY.into(), Yaml::Sequence(steps));
    if frontmatter_map(&rewritten).ok() != Some(map) {
        return Err(UNSAFE_EDIT.into());
    }
    let start = frontmatter.as_ptr() as usize - text.as_ptr() as usize;
    let end = start + frontmatter.len();
    Ok(format!("{}{rewritten}{}", &text[..start], &text[end..]))
}

/// `edited` written over `original`: unchanged values keep their YAML, map keys keep their order
/// and keys new to the map follow in `NEW_KEY_ORDER`.
fn merged(original: Option<&Yaml>, edited: &Json) -> Result<Yaml, String> {
    if let Some(original) = original {
        if serde_json::to_value(original).ok().as_ref() == Some(edited) {
            return Ok(original.clone());
        }
    }
    let Json::Object(edited) = edited else {
        return serde_norway::to_value(edited).map_err(|error| error.to_string());
    };
    let unwritten = Mapping::new();
    let original = match original {
        Some(Yaml::Mapping(original)) => original,
        _ => &unwritten,
    };
    let mut map = Mapping::new();
    for (key, value) in original {
        if let Some(edited) = key.as_str().and_then(|key| edited.get(key)) {
            map.insert(key.clone(), merged(Some(value), edited)?);
        }
    }
    let mut added: Vec<_> = edited
        .iter()
        .filter(|(key, _)| !map.contains_key(key.as_str()))
        .collect();
    added.sort_by_key(|(key, _)| {
        NEW_KEY_ORDER
            .iter()
            .position(|known| known == key)
            .unwrap_or(NEW_KEY_ORDER.len())
    });
    for (key, value) in added {
        map.insert(key.as_str().into(), merged(None, value)?);
    }
    Ok(Yaml::Mapping(map))
}

fn steps_block(steps: &[Yaml], crlf: bool) -> Result<String, String> {
    let mut root = Mapping::new();
    root.insert(STEPS_KEY.into(), Yaml::Sequence(steps.to_vec()));
    let block = serde_norway::to_string(&root).map_err(|error| error.to_string())?;
    Ok(if crlf {
        block.replace('\n', "\r\n")
    } else {
        block
    })
}

/// `frontmatter` with its top-level `steps` lines replaced by `block`, or `block` appended when it
/// has none. Comments and blank lines right before the next key stay with that key.
fn spliced(frontmatter: &str, block: &str) -> String {
    let lines: Vec<&str> = frontmatter.split_inclusive('\n').collect();
    let Some(start) = lines.iter().position(|line| is_steps_key(line)) else {
        return format!("{frontmatter}{block}");
    };
    let mut end = lines[start + 1..]
        .iter()
        .position(|line| is_top_level_key(line))
        .map_or(lines.len(), |offset| start + 1 + offset);
    while end > start + 1 && is_blank_or_comment(lines[end - 1]) {
        end -= 1;
    }
    format!(
        "{}{block}{}",
        lines[..start].concat(),
        lines[end..].concat()
    )
}

fn is_steps_key(line: &str) -> bool {
    line.strip_prefix(STEPS_KEY)
        .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
}

fn is_top_level_key(line: &str) -> bool {
    !line.starts_with([' ', '\t', '#', '-', '\r', '\n'])
}

fn is_blank_or_comment(line: &str) -> bool {
    let line = line.trim_start();
    line.is_empty() || line.starts_with('#')
}

#[tauri::command]
pub async fn automation_steps_source<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> Result<StepsSource, String> {
    let dir = Sources::of(&app)?.dir;
    tauri::async_runtime::spawn_blocking(move || read_steps(&dir, &name))
        .await
        .map_err(|_| "Could not read the automation".to_string())?
}

#[tauri::command]
pub async fn automation_steps_save<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    hash: String,
    steps: Vec<StepEdit>,
) -> Result<StepsSave, String> {
    let sources = Sources::of(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (dir, env) = sources.environment();
        save_steps(&dir, &name, &env, &hash, &steps)
    })
    .await
    .map_err(|_| "Could not save the automation".to_string())?
}
