use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const CONFIG_FILE: &str = ".worktreemanager/pools.toml";
const DEFAULT_POOL: &str = "heavy_checks";
const VALUE_PLACEHOLDER: &str = "{value}";

#[derive(Debug, Clone, PartialEq)]
pub struct Pool {
    pub values: Vec<String>,
    env: BTreeMap<String, String>,
}

impl Pool {
    pub fn env_for(&self, value: &str) -> BTreeMap<String, String> {
        self.env
            .iter()
            .map(|(key, template)| (key.clone(), template.replace(VALUE_PLACEHOLDER, value)))
            .collect()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PoolEntry {
    size: Option<usize>,
    values: Option<Vec<PoolValue>>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PoolValue {
    Number(i64),
    Text(String),
}

impl PoolValue {
    fn into_text(self) -> String {
        match self {
            Self::Number(number) => number.to_string(),
            Self::Text(text) => text,
        }
    }
}

pub fn default_path(home: Option<PathBuf>) -> Option<PathBuf> {
    home.map(|home| home.join(CONFIG_FILE))
}

/// Without a file only `heavy_checks` (one slot) exists; the file adds pools or overrides it.
pub fn load(path: Option<&Path>) -> Result<BTreeMap<String, Pool>, String> {
    let mut pools = BTreeMap::from([(
        DEFAULT_POOL.to_string(),
        Pool {
            values: vec!["1".into()],
            env: BTreeMap::new(),
        },
    )]);
    let Some(path) = path else {
        return Ok(pools);
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(pools),
        Err(_) => return Err(format!("Cannot read {}", path.display())),
    };
    let entries: BTreeMap<String, PoolEntry> = toml::from_str(&text)
        .map_err(|error| format!("Invalid {}: {}", path.display(), error.message()))?;
    for (name, entry) in entries {
        let pool = resolve(&name, entry)?;
        pools.insert(name, pool);
    }
    Ok(pools)
}

fn resolve(name: &str, entry: PoolEntry) -> Result<Pool, String> {
    if !is_identifier(name, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!(
            "Pool name `{name}` may only use letters, digits, `_` and `-`"
        ));
    }
    if let Some(key) = entry
        .env
        .keys()
        .find(|key| !is_identifier(key, |c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return Err(format!(
            "Pool `{name}` has an invalid variable name `{key}`"
        ));
    }
    let values: Vec<String> = match (entry.size, entry.values) {
        (Some(size), None) => (1..=size).map(|slot| slot.to_string()).collect(),
        (None, Some(values)) => values.into_iter().map(PoolValue::into_text).collect(),
        _ => Vec::new(),
    };
    let unique: BTreeSet<&String> = values.iter().collect();
    if values.is_empty() || unique.len() != values.len() {
        return Err(format!(
            "Pool `{name}` needs either `size` ≥ 1 or a non-empty list of unique `values`"
        ));
    }
    Ok(Pool {
        values,
        env: entry.env,
    })
}

fn is_identifier(text: &str, allowed: impl Fn(char) -> bool) -> bool {
    !text.is_empty() && text.chars().all(allowed)
}
