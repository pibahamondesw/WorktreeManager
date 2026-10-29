use serde_json::{Map, Value};

use super::{DefinitionIssue, IssueKind};

const TEMPLATE_VARIABLES: &[&str] = &[
    "automation",
    "run.id",
    "run.short_id",
    "run.scheduled_for",
    "item.id",
    "item.version",
    "item.title",
    "tracker.issue",
    "task.id",
    "task.branch",
    "pr.number",
    "pr.url",
];
const ITEM_DATA_PREFIX: &str = "item.data.";

pub(super) struct Report {
    file: String,
    pub errors: Vec<DefinitionIssue>,
    pub warnings: Vec<DefinitionIssue>,
}

impl Report {
    pub fn new(file: String) -> Self {
        Self {
            file,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn error(&mut self, kind: IssueKind, field: &str, message: impl Into<String>) {
        let issue = self.issue(kind, field, message);
        self.errors.push(issue);
    }

    pub fn warning(&mut self, kind: IssueKind, field: &str, message: impl Into<String>) {
        let issue = self.issue(kind, field, message);
        self.warnings.push(issue);
    }

    fn issue(&self, kind: IssueKind, field: &str, message: impl Into<String>) -> DefinitionIssue {
        DefinitionIssue {
            path: self.file.clone(),
            field: field.into(),
            kind,
            message: message.into(),
        }
    }
}

pub(super) fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.into()
    } else {
        format!("{path}.{key}")
    }
}

pub(super) fn index(path: &str, position: usize) -> String {
    format!("{path}[{position}]")
}

/// The keys of one frontmatter map. Keys never read are reported as unknown on `finish`, except
/// `x-` extension keys; a null value reads as absent.
pub(super) struct Fields<'a> {
    map: &'a Map<String, Value>,
    path: String,
    known: Vec<&'static str>,
}

impl<'a> Fields<'a> {
    pub fn of(report: &mut Report, value: &'a Value, path: &str) -> Option<Self> {
        Some(Self {
            map: map(report, value, path)?,
            path: path.into(),
            known: Vec::new(),
        })
    }

    pub fn path(&self, key: &str) -> String {
        join(&self.path, key)
    }

    pub fn get(&mut self, key: &'static str) -> Option<&'a Value> {
        self.known.push(key);
        self.map.get(key).filter(|value| !value.is_null())
    }

    pub fn required(&mut self, report: &mut Report, key: &'static str) -> Option<&'a Value> {
        let value = self.get(key);
        if value.is_none() {
            report.error(IssueKind::Missing, &self.path(key), "is required");
        }
        value
    }

    /// `None` when present and invalid, `Some(None)` when absent.
    pub fn optional<T>(
        &mut self,
        report: &mut Report,
        key: &'static str,
        parse: impl FnOnce(&mut Report, &Value, &str) -> Option<T>,
    ) -> Option<Option<T>> {
        let field = self.path(key);
        match self.get(key) {
            Some(value) => parse(report, value, &field).map(Some),
            None => Some(None),
        }
    }

    pub fn or<T>(
        &mut self,
        report: &mut Report,
        key: &'static str,
        default: T,
        parse: impl FnOnce(&mut Report, &Value, &str) -> Option<T>,
    ) -> Option<T> {
        self.optional(report, key, parse)
            .map(|value| value.unwrap_or(default))
    }

    pub fn require<T>(
        &mut self,
        report: &mut Report,
        key: &'static str,
        parse: impl FnOnce(&mut Report, &Value, &str) -> Option<T>,
    ) -> Option<T> {
        let field = self.path(key);
        let value = self.required(report, key)?;
        parse(report, value, &field)
    }

    /// Paths of the keys not read so far, other than `x-` extension keys.
    pub fn unread(&self) -> Vec<String> {
        self.map
            .keys()
            .filter(|key| !key.starts_with("x-") && !self.known.contains(&key.as_str()))
            .map(|key| join(&self.path, key))
            .collect()
    }

    pub fn finish(self, report: &mut Report) {
        for field in self.unread() {
            report.error(IssueKind::UnknownField, &field, "unknown field");
        }
    }
}

pub(super) fn string(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        _ => {
            report.error(IssueKind::Invalid, field, "must be a string");
            None
        }
    }
}

pub(super) fn boolean(report: &mut Report, value: &Value, field: &str) -> Option<bool> {
    let flag = value.as_bool();
    if flag.is_none() {
        report.error(IssueKind::Invalid, field, "must be true or false");
    }
    flag
}

pub(super) fn integer(report: &mut Report, value: &Value, field: &str, min: u64) -> Option<u64> {
    let number = value.as_u64().filter(|number| *number >= min);
    if number.is_none() {
        report.error(
            IssueKind::Invalid,
            field,
            format!("must be an integer of at least {min}"),
        );
    }
    number
}

pub(super) fn choice<T: Clone>(
    report: &mut Report,
    value: &Value,
    field: &str,
    options: &[(&str, T)],
) -> Option<T> {
    let found = value
        .as_str()
        .and_then(|text| options.iter().find(|(name, _)| *name == text))
        .map(|(_, option)| option.clone());
    if found.is_none() {
        let names: Vec<_> = options.iter().map(|(name, _)| *name).collect();
        report.error(
            IssueKind::Invalid,
            field,
            format!("must be one of {}", names.join(", ")),
        );
    }
    found
}

pub(super) fn map<'a>(
    report: &mut Report,
    value: &'a Value,
    field: &str,
) -> Option<&'a Map<String, Value>> {
    match value {
        Value::Object(map) => Some(map),
        _ => {
            report.error(IssueKind::Invalid, field, "must be a map");
            None
        }
    }
}

pub(super) fn each_entry<T>(
    report: &mut Report,
    value: &Value,
    field: &str,
    mut parse: impl FnMut(&mut Report, &str, &Value, &str) -> Option<T>,
) -> Option<Vec<T>> {
    let entries = map(report, value, field)?;
    let parsed: Vec<Option<T>> = entries
        .iter()
        .map(|(key, item)| parse(report, key, item, &join(field, key)))
        .collect();
    parsed.into_iter().collect()
}

pub(super) fn list<'a>(report: &mut Report, value: &'a Value, field: &str) -> Option<&'a [Value]> {
    match value {
        Value::Array(items) => Some(items),
        _ => {
            report.error(IssueKind::Invalid, field, "must be a list");
            None
        }
    }
}

pub(super) fn each<T>(
    report: &mut Report,
    value: &Value,
    field: &str,
    mut parse: impl FnMut(&mut Report, &Value, &str) -> Option<T>,
) -> Option<Vec<T>> {
    let items = list(report, value, field)?;
    let parsed: Vec<Option<T>> = items
        .iter()
        .enumerate()
        .map(|(position, item)| parse(report, item, &index(field, position)))
        .collect();
    parsed.into_iter().collect()
}

pub(super) fn string_list(report: &mut Report, value: &Value, field: &str) -> Option<Vec<String>> {
    each(report, value, field, string)
}

pub(super) fn duration_secs(text: &str) -> Option<u64> {
    let factor = match text.chars().last()? {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => return None,
    };
    let digits = &text[..text.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok()?.checked_mul(factor)
}

pub(super) fn duration(report: &mut Report, value: &Value, field: &str) -> Option<u64> {
    let text = string(report, value, field)?;
    let secs = duration_secs(&text);
    if secs.is_none() {
        report.error(
            IssueKind::Invalid,
            field,
            "must be a duration such as 90s, 45m, 2h or 7d",
        );
    }
    secs
}

pub(super) fn is_instant(text: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(text).is_ok()
}

fn template_problem(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(position) = rest.find(['{', '}']) {
        let tail = &rest[position..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            rest = &tail[2..];
            continue;
        }
        if tail.starts_with('}') {
            return Some("has an unmatched }; write }} for a literal brace".into());
        }
        let Some(end) = tail.find('}') else {
            return Some("has an unclosed {; write {{ for a literal brace".into());
        };
        let variable = &tail[1..end];
        if !is_template_variable(variable) {
            return Some(format!("uses the unknown variable {{{variable}}}"));
        }
        rest = &tail[end + 1..];
    }
    None
}

fn is_template_variable(name: &str) -> bool {
    TEMPLATE_VARIABLES.contains(&name)
        || name
            .strip_prefix(ITEM_DATA_PREFIX)
            .is_some_and(|key| !key.is_empty())
}

pub(super) fn template(report: &mut Report, value: &Value, field: &str) -> Option<String> {
    let text = string(report, value, field)?;
    if let Some(problem) = template_problem(&text) {
        report.error(IssueKind::Invalid, field, problem);
        return None;
    }
    Some(text)
}

fn identifier(text: &str, max_len: usize, first: fn(char) -> bool, rest: fn(char) -> bool) -> bool {
    let mut chars = text.chars();
    text.len() <= max_len && chars.next().is_some_and(first) && chars.all(rest)
}

fn is_lower_or_digit(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

pub(super) fn is_automation_name(text: &str) -> bool {
    identifier(text, 63, is_lower_or_digit, |c| {
        is_lower_or_digit(c) || c == '-'
    })
}

pub(super) fn is_step_id(text: &str) -> bool {
    identifier(
        text,
        41,
        |c| c.is_ascii_lowercase(),
        |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_',
    )
}
