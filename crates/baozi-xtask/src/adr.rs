//! Architecture decision record governance checks.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const ADR_DIRECTORY: &str = "docs/adr";
const INDEX_FILE: &str = "docs/adr/README.md";
const DEFERRED_TRACER_ADRS: [u16; 2] = [37, 38];

/// The result of checking every ADR governance invariant.
#[derive(Debug, Default)]
pub struct CheckReport {
    pub adrs_checked: usize,
    pub violations: Vec<Violation>,
}

/// One actionable governance violation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Violation {
    pub code: &'static str,
    pub path: PathBuf,
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} [{}]: {}",
            self.path.display(),
            self.code,
            self.message
        )
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Status {
    Proposed,
    Accepted,
    Superseded,
    Rejected,
}

impl Status {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "proposed" => Some(Self::Proposed),
            "accepted" => Some(Self::Accepted),
            "superseded" => Some(Self::Superseded),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }

    fn index_label(self) -> &'static str {
        match self {
            Self::Proposed => "Proposed",
            Self::Accepted => "Accepted",
            Self::Superseded => "Superseded",
            Self::Rejected => "Rejected",
        }
    }

    fn authority_label(self) -> &'static str {
        match self {
            Self::Accepted => "Normative",
            Self::Proposed => "Non-authoritative",
            Self::Superseded | Self::Rejected => "Historical",
        }
    }
}

#[derive(Debug)]
struct FrontMatter {
    status: Status,
    depends_on: Vec<String>,
    related: Vec<String>,
    supersedes: Vec<String>,
    superseded_by: Vec<String>,
    evidence: Vec<String>,
    tracers: Vec<String>,
    promotion_requires: Vec<String>,
}

impl FrontMatter {
    fn link_fields(&self) -> impl Iterator<Item = (&'static str, &str)> {
        self.depends_on
            .iter()
            .map(|value| ("depends_on", value.as_str()))
            .chain(self.related.iter().map(|value| ("related", value.as_str())))
            .chain(
                self.supersedes
                    .iter()
                    .map(|value| ("supersedes", value.as_str())),
            )
            .chain(
                self.superseded_by
                    .iter()
                    .map(|value| ("superseded_by", value.as_str())),
            )
            .chain(
                self.evidence
                    .iter()
                    .map(|value| ("evidence", value.as_str())),
            )
            .chain(self.tracers.iter().map(|value| ("tracers", value.as_str())))
    }
}

#[derive(Debug)]
struct Adr {
    id: u16,
    path: PathBuf,
    title: String,
    body: String,
    front_matter: FrontMatter,
}

#[derive(Debug)]
enum ScalarToken {
    Plain(String),
    Quoted(String),
}

impl ScalarToken {
    fn value(&self) -> &str {
        match self {
            Self::Plain(value) | Self::Quoted(value) => value,
        }
    }

    fn into_value(self) -> String {
        match self {
            Self::Plain(value) | Self::Quoted(value) => value,
        }
    }
}

#[derive(Debug)]
enum RawValue {
    Scalar(ScalarToken),
    List {
        items: Vec<ScalarToken>,
        explicit_empty: bool,
    },
}

type NumberedFrontMatterLine<'a> = (usize, &'a str);
type FrontMatterLines<'a> = Vec<NumberedFrontMatterLine<'a>>;

/// Check the ADR index, metadata, authority graph, and local links.
pub fn check_repository(repository_root: &Path) -> CheckReport {
    let root = fs::canonicalize(repository_root)
        .unwrap_or_else(|_| normalize_path(repository_root.to_path_buf()));
    let mut report = CheckReport::default();
    let adr_directory = match resolve_existing_repository_target(&root, Path::new(ADR_DIRECTORY)) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            report.violations.push(violation(
                "ADR001",
                ADR_DIRECTORY,
                "ADR path must resolve to a repository directory",
            ));
            return report;
        }
        Err(message) => {
            report.violations.push(violation(
                "ADR001",
                ADR_DIRECTORY,
                format!("ADR directory is not a valid repository path: {message}"),
            ));
            return report;
        }
    };

    let entries = match fs::read_dir(&adr_directory) {
        Ok(entries) => entries,
        Err(error) => {
            report.violations.push(violation(
                "ADR001",
                ADR_DIRECTORY,
                format!("cannot read ADR directory: {error}"),
            ));
            return report;
        }
    };

    let mut files = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
                    && path.file_name().and_then(|value| value.to_str()) != Some("README.md")
                {
                    files.push(path);
                }
            }
            Err(error) => report.violations.push(violation(
                "ADR001",
                ADR_DIRECTORY,
                format!("cannot read an ADR directory entry: {error}"),
            )),
        }
    }
    files.sort();

    let mut adrs = BTreeMap::new();
    for path in files {
        match parse_adr(&root, &path) {
            Ok(adr) => {
                report.adrs_checked += 1;
                if let Some(previous) = adrs.insert(adr.id, adr) {
                    let duplicate = adrs.get(&previous.id).map_or_else(
                        || previous.path.display().to_string(),
                        |current| current.path.display().to_string(),
                    );
                    report.violations.push(violation(
                        "ADR002",
                        &previous.path,
                        format!("ADR {:04} is duplicated by {duplicate}", previous.id),
                    ));
                }
            }
            Err(problem) => report.violations.push(problem),
        }
    }

    let mut path_policy = RepositoryPathPolicy::new(&root);
    check_contiguous_ids(&adrs, &mut report.violations);
    check_adr_links(&root, &adrs, &mut path_policy, &mut report.violations);
    check_authority_graph(&root, &adrs, &mut path_policy, &mut report.violations);
    check_index(&root, &adrs, &mut path_policy, &mut report.violations);
    report
}

fn parse_adr(root: &Path, absolute_path: &Path) -> Result<Adr, Violation> {
    let path = absolute_path
        .strip_prefix(root)
        .unwrap_or(absolute_path)
        .to_path_buf();
    let file_name = absolute_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| violation("ADR003", &path, "ADR filename is not valid UTF-8"))?;
    let id = parse_file_id(file_name)
        .ok_or_else(|| violation("ADR003", &path, "expected filename NNNN-lowercase-slug.md"))?;
    let content = fs::read_to_string(absolute_path).map_err(|error| {
        violation(
            "ADR004",
            &path,
            format!("cannot read ADR as UTF-8: {error}"),
        )
    })?;
    let (front_matter_lines, body) =
        split_front_matter(&content).map_err(|message| violation("ADR005", &path, message))?;
    let front_matter = parse_front_matter(front_matter_lines)
        .map_err(|message| violation("ADR006", &path, message))?;
    let (heading_id, title) = parse_heading(&body)
        .ok_or_else(|| violation("ADR007", &path, "missing '# ADR NNNN: Title' heading"))?;
    if heading_id != id {
        return Err(violation(
            "ADR007",
            &path,
            format!("heading ADR {heading_id:04} does not match filename ADR {id:04}"),
        ));
    }

    Ok(Adr {
        id,
        path,
        title,
        body,
        front_matter,
    })
}

fn parse_file_id(file_name: &str) -> Option<u16> {
    let (prefix, slug) = file_name.split_once('-')?;
    if prefix.len() != 4
        || !prefix.bytes().all(|byte| byte.is_ascii_digit())
        || !slug.ends_with(".md")
        || slug == ".md"
        || slug
            .strip_suffix(".md")?
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
    {
        return None;
    }
    let id = prefix.parse().ok()?;
    (id != 0).then_some(id)
}

fn split_front_matter(content: &str) -> Result<(FrontMatterLines<'_>, String), String> {
    let mut lines = content.lines().enumerate();
    match lines.next() {
        Some((_, "---")) => {}
        _ => return Err("document must begin with a YAML frontmatter delimiter".to_owned()),
    }

    let mut front_matter = Vec::new();
    let mut body = Vec::new();
    let mut found_end = false;
    for (index, line) in lines {
        if !found_end && line == "---" {
            found_end = true;
        } else if found_end {
            body.push(line);
        } else {
            front_matter.push((index + 1, line));
        }
    }
    if !found_end {
        return Err("frontmatter is missing its closing delimiter".to_owned());
    }
    Ok((front_matter, body.join("\n")))
}

fn parse_front_matter(lines: FrontMatterLines<'_>) -> Result<FrontMatter, String> {
    let allowed = BTreeSet::from([
        "status",
        "date",
        "updated",
        "authors",
        "decision_type",
        "depends_on",
        "related",
        "supersedes",
        "superseded_by",
        "evidence",
        "tracers",
        "promotion_requires",
    ]);
    let mut values = BTreeMap::<String, RawValue>::new();
    let mut active_list: Option<String> = None;

    for (line_number, line) in lines {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.contains('\t') {
            return Err(format!(
                "line {line_number}: tabs are not allowed in frontmatter"
            ));
        }
        if let Some(item) = line.strip_prefix("  - ") {
            let key = active_list.as_ref().ok_or_else(|| {
                format!("line {line_number}: list item has no preceding list field")
            })?;
            let item = parse_scalar_token(item.trim(), line_number)?;
            if item.value().is_empty() {
                return Err(format!("line {line_number}: list items must not be empty"));
            }
            match values.get_mut(key) {
                Some(RawValue::List { items, .. }) => items.push(item),
                _ => {
                    return Err(format!("line {line_number}: field '{key}' is not a list"));
                }
            }
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            return Err(format!(
                "line {line_number}: canonical list items must use exactly two spaces and '- '"
            ));
        }

        let (key, raw_value) = line
            .split_once(':')
            .ok_or_else(|| format!("line {line_number}: expected 'field: value'"))?;
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return Err(format!("line {line_number}: invalid field name '{key}'"));
        }
        if !allowed.contains(key) {
            return Err(format!("line {line_number}: unknown field '{key}'"));
        }
        if values.contains_key(key) {
            return Err(format!("line {line_number}: duplicate field '{key}'"));
        }

        let raw_value = raw_value.trim();
        let value = if raw_value.is_empty() {
            active_list = Some(key.to_owned());
            RawValue::List {
                items: Vec::new(),
                explicit_empty: false,
            }
        } else if raw_value == "[]" {
            active_list = None;
            RawValue::List {
                items: Vec::new(),
                explicit_empty: true,
            }
        } else {
            active_list = None;
            RawValue::Scalar(parse_scalar_token(raw_value, line_number)?)
        };
        values.insert(key.to_owned(), value);
    }

    let status_text = take_required_scalar(&mut values, "status")?;
    let status = Status::parse(&status_text).ok_or_else(|| {
        format!(
            "unknown status '{status_text}'; expected proposed, accepted, superseded, or rejected"
        )
    })?;
    let date = take_required_scalar(&mut values, "date")?;
    validate_date("date", &date)?;
    let updated = take_optional_scalar(&mut values, "updated")?;
    if let Some(updated) = &updated {
        validate_date("updated", updated)?;
        if updated < &date {
            return Err(format!(
                "updated date {updated} precedes decision date {date}"
            ));
        }
    }
    let authors = take_required_list(&mut values, "authors")?;
    if authors.is_empty() {
        return Err("authors must contain at least one entry".to_owned());
    }
    let decision_type = take_required_scalar(&mut values, "decision_type")?;
    if decision_type.trim().is_empty() {
        return Err("decision_type must not be empty".to_owned());
    }

    let mut front_matter = FrontMatter {
        status,
        depends_on: take_optional_list(&mut values, "depends_on")?,
        related: take_optional_list(&mut values, "related")?,
        supersedes: take_optional_list(&mut values, "supersedes")?,
        superseded_by: take_optional_list(&mut values, "superseded_by")?,
        evidence: take_optional_list(&mut values, "evidence")?,
        tracers: take_optional_list(&mut values, "tracers")?,
        promotion_requires: take_optional_list(&mut values, "promotion_requires")?,
    };

    for (field, entries) in [
        ("depends_on", &mut front_matter.depends_on),
        ("related", &mut front_matter.related),
        ("supersedes", &mut front_matter.supersedes),
        ("superseded_by", &mut front_matter.superseded_by),
        ("evidence", &mut front_matter.evidence),
        ("tracers", &mut front_matter.tracers),
    ] {
        ensure_unique_links(field, entries)?;
    }
    ensure_unique("promotion_requires", &mut front_matter.promotion_requires)?;
    for requirement in &front_matter.promotion_requires {
        if requirement != "implementation_tracer" {
            return Err(format!(
                "unknown promotion requirement '{requirement}'; expected implementation_tracer"
            ));
        }
    }
    debug_assert!(values.is_empty());
    Ok(front_matter)
}

fn take_required_scalar(
    values: &mut BTreeMap<String, RawValue>,
    key: &str,
) -> Result<String, String> {
    match values.remove(key) {
        Some(RawValue::Scalar(value)) if !value.value().is_empty() => Ok(value.into_value()),
        Some(RawValue::Scalar(_)) => Err(format!("field '{key}' must not be empty")),
        Some(RawValue::List { .. }) => Err(format!("field '{key}' must be a scalar")),
        None => Err(format!("missing required field '{key}'")),
    }
}

fn take_optional_scalar(
    values: &mut BTreeMap<String, RawValue>,
    key: &str,
) -> Result<Option<String>, String> {
    match values.remove(key) {
        Some(RawValue::Scalar(value)) if !value.value().is_empty() => Ok(Some(value.into_value())),
        Some(RawValue::Scalar(_)) => Err(format!("field '{key}' must not be empty")),
        Some(RawValue::List { .. }) => Err(format!("field '{key}' must be a scalar")),
        None => Ok(None),
    }
}

fn take_required_list(
    values: &mut BTreeMap<String, RawValue>,
    key: &str,
) -> Result<Vec<String>, String> {
    match values.remove(key) {
        Some(RawValue::List {
            items,
            explicit_empty,
        }) if !items.is_empty() || explicit_empty => {
            Ok(items.into_iter().map(ScalarToken::into_value).collect())
        }
        Some(RawValue::List { .. }) => Err(format!(
            "field '{key}' has no list items; use [] for an empty list"
        )),
        Some(RawValue::Scalar(_)) => Err(format!("field '{key}' must be a block list or []")),
        None => Err(format!("missing required field '{key}'")),
    }
}

fn take_optional_list(
    values: &mut BTreeMap<String, RawValue>,
    key: &str,
) -> Result<Vec<String>, String> {
    match values.remove(key) {
        Some(RawValue::List {
            items,
            explicit_empty,
        }) if !items.is_empty() || explicit_empty => {
            Ok(items.into_iter().map(ScalarToken::into_value).collect())
        }
        Some(RawValue::List { .. }) => Err(format!(
            "field '{key}' has no list items; use [] for an empty list"
        )),
        Some(RawValue::Scalar(_)) => Err(format!("field '{key}' must be a block list or []")),
        None => Ok(Vec::new()),
    }
}

fn ensure_unique(field: &str, entries: &mut [String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        if !seen.insert(entry.as_str()) {
            return Err(format!(
                "field '{field}' contains duplicate entry '{entry}'"
            ));
        }
    }
    Ok(())
}

fn ensure_unique_links(field: &str, entries: &mut [String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        let identity = normalized_link_identity(entry);
        if !seen.insert(identity) {
            return Err(format!(
                "field '{field}' contains duplicate repository target '{entry}'"
            ));
        }
    }
    Ok(())
}

fn normalized_link_identity(target: &str) -> String {
    let target = target.trim().trim_matches(['<', '>']);
    if target.starts_with("https://")
        || target.starts_with("http://")
        || target.starts_with("mailto:")
    {
        return target.to_owned();
    }

    let (path_and_query, fragment) = target
        .split_once('#')
        .map_or((target, None), |(path, fragment)| (path, Some(fragment)));
    let (path, query) = path_and_query
        .split_once('?')
        .map_or((path_and_query, None), |(path, query)| (path, Some(query)));
    let normalized = normalize_path(PathBuf::from(
        path.replace('/', std::path::MAIN_SEPARATOR_STR),
    ));
    let mut identity = git_path(&normalized);
    if let Some(query) = query {
        identity.push('?');
        identity.push_str(query);
    }
    if let Some(fragment) = fragment {
        identity.push('#');
        identity.push_str(fragment);
    }
    identity
}

fn parse_scalar_token(value: &str, line_number: usize) -> Result<ScalarToken, String> {
    let first = value.chars().next();
    let last = value.chars().next_back();
    match (first, last) {
        (Some('"'), Some('"')) | (Some('\''), Some('\'')) if value.len() >= 2 => {
            Ok(ScalarToken::Quoted(value[1..value.len() - 1].to_owned()))
        }
        (Some('"' | '\''), _) | (_, Some('"' | '\'')) => Err(format!(
            "line {line_number}: quoted scalar must use matching delimiters"
        )),
        _ if matches!(value, "null" | "Null" | "NULL" | "~") => Err(format!(
            "line {line_number}: unquoted YAML null values are not allowed"
        )),
        _ => Ok(ScalarToken::Plain(value.to_owned())),
    }
}

fn validate_date(field: &str, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return Err(format!("field '{field}' must use YYYY-MM-DD"));
    }
    let year: u16 = value[0..4]
        .parse()
        .map_err(|_| format!("field '{field}' has an invalid year"))?;
    let month: u8 = value[5..7]
        .parse()
        .map_err(|_| format!("field '{field}' has an invalid month"))?;
    let day: u8 = value[8..10]
        .parse()
        .map_err(|_| format!("field '{field}' has an invalid day"))?;
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return Err(format!("field '{field}' has an invalid month")),
    };
    if day == 0 || day > days_in_month {
        return Err(format!("field '{field}' has an invalid day"));
    }
    Ok(())
}

fn parse_heading(body: &str) -> Option<(u16, String)> {
    let line = body.lines().find(|line| !line.trim().is_empty())?;
    let rest = line.strip_prefix("# ADR ")?;
    let (id, title) = rest.split_once(": ")?;
    if id.len() != 4
        || !id.bytes().all(|byte| byte.is_ascii_digit())
        || title.is_empty()
        || title != title.trim()
    {
        return None;
    }
    let id = id.parse().ok()?;
    (id != 0).then(|| (id, title.to_owned()))
}

fn check_contiguous_ids(adrs: &BTreeMap<u16, Adr>, violations: &mut Vec<Violation>) {
    let Some(maximum) = adrs.keys().next_back().copied() else {
        violations.push(violation("ADR008", ADR_DIRECTORY, "no ADR files found"));
        return;
    };
    for id in 1..=maximum {
        if !adrs.contains_key(&id) {
            violations.push(violation(
                "ADR008",
                ADR_DIRECTORY,
                format!("ADR sequence is missing {id:04}"),
            ));
        }
    }
}

#[derive(Debug, Clone)]
enum GitWorktree {
    Available,
    NotRepository,
    Unavailable(String),
}

#[derive(Debug)]
struct RepositoryPathPolicy {
    root: PathBuf,
    git_worktree: GitWorktree,
    resolved_targets: BTreeMap<PathBuf, Result<PathBuf, String>>,
    ignored: BTreeMap<PathBuf, Result<bool, String>>,
    tracked_or_staged: BTreeMap<PathBuf, Result<bool, String>>,
}

impl RepositoryPathPolicy {
    fn new(root: &Path) -> Self {
        let git_worktree = if has_git_metadata(root) {
            match Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["rev-parse", "--is-inside-work-tree"])
                .output()
            {
                Ok(output)
                    if output.status.success()
                        && String::from_utf8_lossy(&output.stdout).trim() == "true" =>
                {
                    GitWorktree::Available
                }
                Ok(output) => GitWorktree::Unavailable(format!(
                    "git rev-parse failed with status {}",
                    display_status(&output.status)
                )),
                Err(error) => GitWorktree::Unavailable(format!("cannot execute git: {error}")),
            }
        } else {
            GitWorktree::NotRepository
        };
        Self {
            root: root.to_path_buf(),
            git_worktree,
            resolved_targets: BTreeMap::new(),
            ignored: BTreeMap::new(),
            tracked_or_staged: BTreeMap::new(),
        }
    }

    fn resolve_target(&mut self, relative: &Path) -> Result<PathBuf, String> {
        if let Some(result) = self.resolved_targets.get(relative) {
            return result.clone();
        }

        let result = resolve_existing_repository_target(&self.root, relative);
        self.resolved_targets
            .insert(relative.to_path_buf(), result.clone());
        result
    }

    fn prime_ignored(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let paths = paths
            .into_iter()
            .filter(|path| !path.as_os_str().is_empty())
            .collect::<BTreeSet<_>>();
        let pending = paths
            .into_iter()
            .filter(|path| !self.ignored.contains_key(path))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return;
        }

        match &self.git_worktree {
            GitWorktree::NotRepository => {
                for path in pending {
                    self.ignored.insert(path, Ok(false));
                }
            }
            GitWorktree::Unavailable(message) => {
                for path in pending {
                    self.ignored.insert(path, Err(message.clone()));
                }
            }
            GitWorktree::Available => self.run_batched_ignore_query(pending),
        }
    }

    fn run_batched_ignore_query(&mut self, pending: Vec<PathBuf>) {
        let mut child = match Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["check-ignore", "--stdin", "-z"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let message = format!("cannot execute git check-ignore: {error}");
                for path in pending {
                    self.ignored.insert(path, Err(message.clone()));
                }
                return;
            }
        };

        let write_result = child.stdin.take().map_or_else(
            || Err("git check-ignore stdin is unavailable".to_owned()),
            |mut stdin| {
                for path in &pending {
                    stdin
                        .write_all(git_path(path).as_bytes())
                        .and_then(|()| stdin.write_all(&[0]))
                        .map_err(|error| format!("cannot write git check-ignore input: {error}"))?;
                }
                Ok(())
            },
        );
        if let Err(message) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            for path in pending {
                self.ignored.insert(path, Err(message.clone()));
            }
            return;
        }

        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(error) => {
                let message = format!("cannot read git check-ignore output: {error}");
                for path in pending {
                    self.ignored.insert(path, Err(message.clone()));
                }
                return;
            }
        };
        if !matches!(output.status.code(), Some(0 | 1)) {
            let message = format!(
                "git check-ignore failed with status {}",
                display_status(&output.status)
            );
            for path in pending {
                self.ignored.insert(path, Err(message.clone()));
            }
            return;
        }

        for path in &pending {
            self.ignored.insert(path.clone(), Ok(false));
        }
        for raw_path in output.stdout.split(|byte| *byte == 0) {
            if raw_path.is_empty() {
                continue;
            }
            let path = PathBuf::from(
                String::from_utf8_lossy(raw_path).replace('/', std::path::MAIN_SEPARATOR_STR),
            );
            self.ignored.insert(normalize_path(path), Ok(true));
        }
    }

    fn is_ignored(&mut self, relative: &Path) -> Result<bool, String> {
        if relative.as_os_str().is_empty() {
            return Ok(false);
        }
        if let Some(result) = self.ignored.get(relative) {
            return result.clone();
        }

        let result = match &self.git_worktree {
            GitWorktree::Available => Command::new("git")
                .arg("-C")
                .arg(&self.root)
                .args(["check-ignore", "--quiet", "--"])
                .arg(relative)
                .output()
                .map_err(|error| format!("cannot execute git check-ignore: {error}"))
                .and_then(|output| match output.status.code() {
                    Some(0) => Ok(true),
                    Some(1) => Ok(false),
                    _ => Err(format!(
                        "git check-ignore failed with status {}",
                        display_status(&output.status)
                    )),
                }),
            GitWorktree::NotRepository => Ok(false),
            GitWorktree::Unavailable(message) => Err(message.clone()),
        };
        self.ignored.insert(relative.to_path_buf(), result.clone());
        result
    }

    fn is_tracked_or_staged(&mut self, relative: &Path) -> Result<bool, String> {
        if let Some(result) = self.tracked_or_staged.get(relative) {
            return result.clone();
        }

        let result = match &self.git_worktree {
            GitWorktree::Available => Command::new("git")
                .arg("-C")
                .arg(&self.root)
                .args(["ls-files", "--error-unmatch", "--"])
                .arg(relative)
                .output()
                .map_err(|error| format!("cannot execute git ls-files: {error}"))
                .and_then(|output| match output.status.code() {
                    Some(0) => Ok(true),
                    Some(1) => Ok(false),
                    _ => Err(format!(
                        "git ls-files failed with status {}",
                        display_status(&output.status)
                    )),
                }),
            GitWorktree::NotRepository => Err(
                "cannot prove that the implementation tracer is tracked outside a Git worktree"
                    .to_owned(),
            ),
            GitWorktree::Unavailable(message) => Err(message.clone()),
        };
        self.tracked_or_staged
            .insert(relative.to_path_buf(), result.clone());
        result
    }
}

fn resolve_existing_repository_target(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    verify_exact_path_case(root, relative)?;
    let lexical_target = root.join(relative);
    let physical_target = fs::canonicalize(&lexical_target)
        .map_err(|error| format!("target does not exist or cannot be resolved: {error}"))?;
    if !physical_target.starts_with(root) {
        return Err(
            "target resolves outside the repository root through a symlink or junction".into(),
        );
    }
    Ok(physical_target)
}

fn verify_exact_path_case(root: &Path, relative: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(expected) = component else {
            return Err("repository target is not a normalized relative path".to_owned());
        };
        let entries = fs::read_dir(&current)
            .map_err(|error| format!("cannot inspect repository path casing: {error}"))?;
        let mut case_insensitive_match = None;
        let mut exact_match = false;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("cannot inspect repository entry: {error}"))?;
            let actual = entry.file_name();
            if actual == expected {
                exact_match = true;
                break;
            }
            if actual
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.to_string_lossy())
            {
                case_insensitive_match = Some(actual);
            }
        }
        if !exact_match {
            return match case_insensitive_match {
                Some(actual) => Err(format!(
                    "path component '{}' has incorrect casing; repository entry is '{}'",
                    expected.to_string_lossy(),
                    actual.to_string_lossy()
                )),
                None => Err(format!(
                    "target path component '{}' does not exist",
                    expected.to_string_lossy()
                )),
            };
        }
        current.push(expected);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("cannot inspect repository target metadata: {error}"))?;
        if is_symlink_or_reparse_point(&metadata) {
            return Err(format!(
                "repository target component '{}' is a symbolic link or reparse point",
                expected.to_string_lossy()
            ));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn is_symlink_or_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn is_symlink_or_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn has_git_metadata(root: &Path) -> bool {
    root.ancestors()
        .any(|candidate| candidate.join(".git").exists())
}

fn display_status(status: &std::process::ExitStatus) -> String {
    status.code().map_or_else(
        || "terminated by signal".to_owned(),
        |code| code.to_string(),
    )
}

fn has_forbidden_repository_component(relative: &Path) -> bool {
    relative.components().any(|component| {
        let Component::Normal(component) = component else {
            return false;
        };
        let component = component.to_string_lossy();
        component.eq_ignore_ascii_case("repo-ref") || component.eq_ignore_ascii_case(".git")
    })
}

fn check_adr_links(
    root: &Path,
    adrs: &BTreeMap<u16, Adr>,
    path_policy: &mut RepositoryPathPolicy,
    violations: &mut Vec<Violation>,
) {
    let mut local_paths = Vec::new();
    for adr in adrs.values() {
        for (_, target) in adr.front_matter.link_fields() {
            if let Ok(LinkTarget::Local { relative, .. }) =
                resolve_link(root, &adr.path, target, LinkBase::RepositoryRoot)
            {
                local_paths.push(relative);
            }
        }
        if let Ok(targets) = markdown_links(&adr.body) {
            for target in targets {
                if let Ok(LinkTarget::Local { relative, .. }) =
                    resolve_link(root, &adr.path, &target, LinkBase::SourceDirectory)
                {
                    local_paths.push(relative);
                }
            }
        }
    }
    path_policy.prime_ignored(local_paths);

    for adr in adrs.values() {
        for (field, target) in adr.front_matter.link_fields() {
            check_link(
                root,
                LinkCheck {
                    source: &adr.path,
                    target,
                    link_base: LinkBase::RepositoryRoot,
                    require_local: true,
                    field,
                },
                path_policy,
                violations,
            );
        }
        match markdown_links(&adr.body) {
            Ok(targets) => {
                for target in targets {
                    check_link(
                        root,
                        LinkCheck {
                            source: &adr.path,
                            target: &target,
                            link_base: LinkBase::SourceDirectory,
                            require_local: false,
                            field: "Markdown",
                        },
                        path_policy,
                        violations,
                    );
                }
            }
            Err(message) => violations.push(violation("ADR009", &adr.path, message)),
        }
    }
}

struct LinkCheck<'a> {
    source: &'a Path,
    target: &'a str,
    link_base: LinkBase,
    require_local: bool,
    field: &'a str,
}

fn check_link(
    root: &Path,
    check: LinkCheck<'_>,
    path_policy: &mut RepositoryPathPolicy,
    violations: &mut Vec<Violation>,
) {
    let LinkCheck {
        source,
        target,
        link_base,
        require_local,
        field,
    } = check;
    match resolve_link(root, source, target, link_base) {
        Ok(LinkTarget::External) if require_local => violations.push(violation(
            "ADR009",
            source,
            format!("{field} must use a repository-local evidence path, found '{target}'"),
        )),
        Ok(LinkTarget::External) => {}
        Ok(LinkTarget::Anchor(fragment)) => {
            check_fragment(
                root,
                source,
                &fragment,
                path_policy,
                field,
                target,
                violations,
            );
        }
        Ok(LinkTarget::Local { relative, fragment }) => {
            if has_forbidden_repository_component(&relative) {
                violations.push(violation(
                    "ADR009",
                    source,
                    format!(
                        "{field} points into forbidden repository metadata or reference storage ('{target}')"
                    ),
                ));
            } else {
                let physical_target = match path_policy.resolve_target(&relative) {
                    Ok(path) => path,
                    Err(message) => {
                        violations.push(violation(
                            "ADR009",
                            source,
                            format!("{field} target is not a valid repository path: '{target}': {message}"),
                        ));
                        return;
                    }
                };
                match path_policy.is_ignored(&relative) {
                    Ok(true) => violations.push(violation(
                        "ADR009",
                        source,
                        format!(
                            "{field} target is Git-ignored and is not durable in a clean checkout: '{target}'"
                        ),
                    )),
                    Ok(false) => {}
                    Err(message) => violations.push(violation(
                        "ADR009",
                        source,
                        format!("cannot verify whether {field} target '{target}' is durable: {message}"),
                    )),
                }
                if let Some(fragment) = fragment {
                    check_fragment_path(
                        source,
                        &physical_target,
                        &fragment,
                        field,
                        target,
                        violations,
                    );
                }
            }
        }
        Err(message) => violations.push(violation(
            "ADR009",
            source,
            format!("invalid {field} target '{target}': {message}"),
        )),
    }
}

fn check_fragment(
    root: &Path,
    source: &Path,
    fragment: &str,
    path_policy: &mut RepositoryPathPolicy,
    field: &str,
    target: &str,
    violations: &mut Vec<Violation>,
) {
    let relative = normalize_path(source.to_path_buf());
    let physical_target = match path_policy.resolve_target(&relative) {
        Ok(path) => path,
        Err(message) => {
            violations.push(violation(
                "ADR009",
                source,
                format!("cannot validate {field} anchor '{target}': {message}"),
            ));
            return;
        }
    };
    debug_assert!(physical_target.starts_with(root));
    check_fragment_path(
        source,
        &physical_target,
        fragment,
        field,
        target,
        violations,
    );
}

fn check_fragment_path(
    source: &Path,
    physical_target: &Path,
    fragment: &str,
    field: &str,
    target: &str,
    violations: &mut Vec<Violation>,
) {
    if physical_target.extension().and_then(|value| value.to_str()) != Some("md") {
        violations.push(violation(
            "ADR009",
            source,
            format!("{field} fragment is unverifiable for a non-Markdown target: '{target}'"),
        ));
        return;
    }
    let content = match fs::read_to_string(physical_target) {
        Ok(content) => content,
        Err(error) => {
            violations.push(violation(
                "ADR009",
                source,
                format!("cannot read {field} fragment target '{target}': {error}"),
            ));
            return;
        }
    };
    match markdown_heading_anchors(&content) {
        Ok(anchors) if anchors.contains(fragment) => {}
        Ok(_) => violations.push(violation(
            "ADR009",
            source,
            format!("{field} target has no heading anchor '#{fragment}': '{target}'"),
        )),
        Err(message) => violations.push(violation(
            "ADR009",
            source,
            format!("cannot validate {field} fragment target '{target}': {message}"),
        )),
    }
}

fn check_authority_graph(
    root: &Path,
    adrs: &BTreeMap<u16, Adr>,
    path_policy: &mut RepositoryPathPolicy,
    violations: &mut Vec<Violation>,
) {
    let by_path: BTreeMap<PathBuf, u16> = adrs
        .values()
        .map(|adr| (adr.path.clone(), adr.id))
        .collect();
    let mut dependencies = BTreeMap::<u16, Vec<u16>>::new();

    for adr in adrs.values() {
        check_status_shape(adr, violations);
        if DEFERRED_TRACER_ADRS.contains(&adr.id)
            && !adr
                .front_matter
                .promotion_requires
                .iter()
                .any(|requirement| requirement == "implementation_tracer")
        {
            violations.push(violation(
                "ADR010",
                &adr.path,
                "ADR 0037 and ADR 0038 must retain promotion_requires: implementation_tracer",
            ));
        }
        let tracer_required = DEFERRED_TRACER_ADRS.contains(&adr.id)
            || adr
                .front_matter
                .promotion_requires
                .iter()
                .any(|requirement| requirement == "implementation_tracer");
        if adr.front_matter.status == Status::Accepted
            && tracer_required
            && adr.front_matter.tracers.is_empty()
        {
            violations.push(violation(
                "ADR010",
                &adr.path,
                "Accepted promotion requires at least one repository-local implementation tracer",
            ));
        } else if adr.front_matter.status == Status::Accepted && tracer_required {
            check_implementation_tracers(root, adr, path_policy, violations);
        }

        let mut target_dependencies = Vec::new();
        for target in &adr.front_matter.depends_on {
            if let Some(target_adr) =
                resolve_adr_target(root, adr, target, &by_path, adrs, violations)
            {
                target_dependencies.push(target_adr.id);
                if adr.front_matter.status == Status::Accepted
                    && target_adr.front_matter.status != Status::Accepted
                {
                    violations.push(violation(
                        "ADR011",
                        &adr.path,
                        format!(
                            "Accepted ADR {:04} normatively depends on non-Accepted ADR {:04} ({})",
                            adr.id,
                            target_adr.id,
                            target_adr.front_matter.status.index_label()
                        ),
                    ));
                }
            }
        }
        dependencies.insert(adr.id, target_dependencies);

        for target in &adr.front_matter.supersedes {
            if let Some(target_adr) =
                resolve_adr_target(root, adr, target, &by_path, adrs, violations)
            {
                if target_adr.front_matter.status != Status::Superseded {
                    violations.push(violation(
                        "ADR012",
                        &adr.path,
                        format!(
                            "supersedes target ADR {:04} must have status superseded",
                            target_adr.id
                        ),
                    ));
                }
                if !relation_points_to(
                    root,
                    target_adr,
                    &target_adr.front_matter.superseded_by,
                    &adr.path,
                ) {
                    violations.push(violation(
                        "ADR012",
                        &adr.path,
                        format!(
                            "ADR {:04} supersedes ADR {:04}, but the target lacks reciprocal superseded_by",
                            adr.id, target_adr.id
                        ),
                    ));
                }
            }
        }
        for target in &adr.front_matter.superseded_by {
            if let Some(target_adr) =
                resolve_adr_target(root, adr, target, &by_path, adrs, violations)
            {
                if target_adr.front_matter.status != Status::Accepted {
                    violations.push(violation(
                        "ADR012",
                        &adr.path,
                        format!(
                            "superseded_by target ADR {:04} must have status accepted",
                            target_adr.id
                        ),
                    ));
                }
                if !relation_points_to(
                    root,
                    target_adr,
                    &target_adr.front_matter.supersedes,
                    &adr.path,
                ) {
                    violations.push(violation(
                        "ADR012",
                        &adr.path,
                        format!(
                            "ADR {:04} is superseded by ADR {:04}, but the target lacks reciprocal supersedes",
                            adr.id, target_adr.id
                        ),
                    ));
                }
            }
        }
    }

    check_dependency_cycles(adrs, &dependencies, violations);
}

fn check_implementation_tracers(
    root: &Path,
    adr: &Adr,
    path_policy: &mut RepositoryPathPolicy,
    violations: &mut Vec<Violation>,
) {
    for target in &adr.front_matter.tracers {
        let relative = match resolve_link(root, &adr.path, target, LinkBase::RepositoryRoot) {
            Ok(LinkTarget::Local {
                relative,
                fragment: None,
            }) => relative,
            _ => {
                violations.push(violation(
                    "ADR010",
                    &adr.path,
                    format!("implementation tracer must be a durable repository file: '{target}'"),
                ));
                continue;
            }
        };

        if has_forbidden_repository_component(&relative) {
            violations.push(violation(
                "ADR010",
                &adr.path,
                format!("implementation tracer uses a forbidden repository path: '{target}'"),
            ));
            continue;
        }
        let physical_target = match path_policy.resolve_target(&relative) {
            Ok(path) => path,
            Err(message) => {
                violations.push(violation(
                    "ADR010",
                    &adr.path,
                    format!("implementation tracer is not a valid repository path: '{target}': {message}"),
                ));
                continue;
            }
        };
        if !physical_target.is_file() {
            violations.push(violation(
                "ADR010",
                &adr.path,
                format!("implementation tracer must resolve to a regular file: '{target}'"),
            ));
            continue;
        }
        match path_policy.is_ignored(&relative) {
            Ok(true) => {
                violations.push(violation(
                    "ADR010",
                    &adr.path,
                    format!("implementation tracer is Git-ignored: '{target}'"),
                ));
                continue;
            }
            Ok(false) => {}
            Err(message) => {
                violations.push(violation(
                    "ADR010",
                    &adr.path,
                    format!(
                        "cannot verify implementation tracer durability for '{target}': {message}"
                    ),
                ));
                continue;
            }
        }
        match path_policy.is_tracked_or_staged(&relative) {
            Ok(true) => {}
            Ok(false) => violations.push(violation(
                "ADR010",
                &adr.path,
                format!("implementation tracer is not tracked or staged: '{target}'"),
            )),
            Err(message) => violations.push(violation(
                "ADR010",
                &adr.path,
                format!(
                    "cannot verify that implementation tracer '{target}' is tracked or staged: {message}"
                ),
            )),
        }
    }
}

fn check_status_shape(adr: &Adr, violations: &mut Vec<Violation>) {
    match adr.front_matter.status {
        Status::Accepted => {
            if !adr.front_matter.superseded_by.is_empty() {
                violations.push(violation(
                    "ADR013",
                    &adr.path,
                    "Accepted ADR must not declare superseded_by",
                ));
            }
        }
        Status::Superseded => {
            if adr.front_matter.superseded_by.is_empty() {
                violations.push(violation(
                    "ADR013",
                    &adr.path,
                    "Superseded ADR must declare at least one superseded_by target",
                ));
            }
            if !adr.front_matter.supersedes.is_empty() {
                violations.push(violation(
                    "ADR013",
                    &adr.path,
                    "Superseded ADR must not declare supersedes; authority belongs to Accepted ADRs",
                ));
            }
        }
        Status::Proposed | Status::Rejected => {
            if !adr.front_matter.supersedes.is_empty() || !adr.front_matter.superseded_by.is_empty()
            {
                violations.push(violation(
                    "ADR013",
                    &adr.path,
                    "Proposed and Rejected ADRs must not declare supersession authority",
                ));
            }
        }
    }
}

fn resolve_adr_target<'a>(
    root: &Path,
    source: &Adr,
    target: &str,
    by_path: &BTreeMap<PathBuf, u16>,
    adrs: &'a BTreeMap<u16, Adr>,
    violations: &mut Vec<Violation>,
) -> Option<&'a Adr> {
    let relative = match resolve_link(root, &source.path, target, LinkBase::RepositoryRoot) {
        Ok(LinkTarget::Local {
            relative,
            fragment: None,
        }) => relative,
        _ => {
            violations.push(violation(
                "ADR014",
                &source.path,
                format!("authority relation must target an ADR file: '{target}'"),
            ));
            return None;
        }
    };
    let Some(id) = by_path.get(&relative) else {
        violations.push(violation(
            "ADR014",
            &source.path,
            format!("authority relation does not target an indexed ADR: '{target}'"),
        ));
        return None;
    };
    if *id == source.id {
        violations.push(violation(
            "ADR014",
            &source.path,
            "ADR must not have an authority relation to itself",
        ));
        return None;
    }
    adrs.get(id)
}

fn relation_points_to(root: &Path, source: &Adr, relations: &[String], expected: &Path) -> bool {
    relations.iter().any(|target| {
        matches!(
            resolve_link(root, &source.path, target, LinkBase::RepositoryRoot),
            Ok(LinkTarget::Local { relative, fragment: None }) if relative == expected
        )
    })
}

fn check_dependency_cycles(
    adrs: &BTreeMap<u16, Adr>,
    dependencies: &BTreeMap<u16, Vec<u16>>,
    violations: &mut Vec<Violation>,
) {
    let mut states = BTreeMap::<u16, VisitState>::new();
    let mut stack = Vec::new();
    for id in dependencies.keys().copied() {
        visit_dependency(id, adrs, dependencies, &mut states, &mut stack, violations);
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum VisitState {
    Active,
    Complete,
}

fn visit_dependency(
    id: u16,
    adrs: &BTreeMap<u16, Adr>,
    dependencies: &BTreeMap<u16, Vec<u16>>,
    states: &mut BTreeMap<u16, VisitState>,
    stack: &mut Vec<u16>,
    violations: &mut Vec<Violation>,
) {
    match states.get(&id) {
        Some(VisitState::Complete) => return,
        Some(VisitState::Active) => {
            if let Some(adr) = adrs.get(&id) {
                let start = stack.iter().position(|entry| *entry == id).unwrap_or(0);
                let mut cycle = stack[start..]
                    .iter()
                    .map(|entry| format!("{entry:04}"))
                    .collect::<Vec<_>>();
                cycle.push(format!("{id:04}"));
                violations.push(violation(
                    "ADR015",
                    &adr.path,
                    format!("depends_on contains a cycle: {}", cycle.join(" -> ")),
                ));
            }
            return;
        }
        None => {}
    }

    states.insert(id, VisitState::Active);
    stack.push(id);
    if let Some(targets) = dependencies.get(&id) {
        for target in targets {
            visit_dependency(*target, adrs, dependencies, states, stack, violations);
        }
    }
    stack.pop();
    states.insert(id, VisitState::Complete);
}

fn check_index(
    root: &Path,
    adrs: &BTreeMap<u16, Adr>,
    path_policy: &mut RepositoryPathPolicy,
    violations: &mut Vec<Violation>,
) {
    let index_path = match path_policy.resolve_target(Path::new(INDEX_FILE)) {
        Ok(path) if path.is_file() => path,
        Ok(_) => {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                "ADR index must resolve to a regular repository file",
            ));
            return;
        }
        Err(message) => {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!("ADR index is not a valid repository path: {message}"),
            ));
            return;
        }
    };
    let content = match fs::read_to_string(&index_path) {
        Ok(content) => content,
        Err(error) => {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!("cannot read ADR index: {error}"),
            ));
            return;
        }
    };
    let parsed_links = markdown_links(&content);
    let local_paths = parsed_links
        .as_ref()
        .into_iter()
        .flat_map(|targets| targets.iter())
        .filter_map(|target| {
            match resolve_link(
                root,
                Path::new(INDEX_FILE),
                target,
                LinkBase::SourceDirectory,
            ) {
                Ok(LinkTarget::Local { relative, .. }) => Some(relative),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    path_policy.prime_ignored(local_paths);

    match parsed_links {
        Ok(targets) => {
            for target in targets {
                check_link(
                    root,
                    LinkCheck {
                        source: Path::new(INDEX_FILE),
                        target: &target,
                        link_base: LinkBase::SourceDirectory,
                        require_local: false,
                        field: "Markdown",
                    },
                    path_policy,
                    violations,
                );
            }
        }
        Err(message) => violations.push(violation("ADR009", INDEX_FILE, message)),
    }

    let parsed_rows = match parse_index_table(&content) {
        Ok(rows) => rows,
        Err(message) => {
            violations.push(violation("ADR016", INDEX_FILE, message));
            return;
        }
    };
    let mut rows = BTreeMap::<u16, Vec<IndexRow>>::new();
    for row in parsed_rows {
        rows.entry(row.id).or_default().push(row);
    }

    for (id, adr) in adrs {
        let Some(entries) = rows.get(id) else {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!("ADR {id:04} is missing from the index table"),
            ));
            continue;
        };
        if entries.len() != 1 {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!(
                    "ADR {id:04} appears {} times in the index table",
                    entries.len()
                ),
            ));
            continue;
        }
        let row = &entries[0];
        let expected_target = adr
            .path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if row.target != expected_target {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!(
                    "line {}: ADR {id:04} links to '{}', expected '{expected_target}'",
                    row.line, row.target
                ),
            ));
        }
        if row.title != adr.title {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!(
                    "line {}: ADR {id:04} title '{}' does not match '{}'",
                    row.line, row.title, adr.title
                ),
            ));
        }
        if row.status != adr.front_matter.status.index_label() {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!(
                    "line {}: ADR {id:04} status '{}' does not match '{}'",
                    row.line,
                    row.status,
                    adr.front_matter.status.index_label()
                ),
            ));
        }
        if row.authority != adr.front_matter.status.authority_label() {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!(
                    "line {}: ADR {id:04} authority '{}' does not match '{}'",
                    row.line,
                    row.authority,
                    adr.front_matter.status.authority_label()
                ),
            ));
        }
    }
    for id in rows.keys() {
        if !adrs.contains_key(id) {
            violations.push(violation(
                "ADR016",
                INDEX_FILE,
                format!("index contains ADR {id:04}, but no matching ADR file exists"),
            ));
        }
    }
}

fn parse_index_table(content: &str) -> Result<Vec<IndexRow>, String> {
    const HEADER: &str = "| ADR | Title | Status | Authority |";
    const SEPARATOR: &str = "| --- | --- | --- | --- |";

    let lines = markdown_visible_lines(content)?;
    let headers = lines
        .iter()
        .enumerate()
        .filter_map(|(index, (_, line))| (*line == HEADER).then_some(index))
        .collect::<Vec<_>>();
    let [header_index] = headers.as_slice() else {
        return Err(format!(
            "ADR index must contain exactly one canonical '{HEADER}' table header"
        ));
    };
    let Some((separator_line, separator)) = lines.get(header_index + 1) else {
        return Err("ADR index table is missing its separator row".to_owned());
    };
    if *separator != SEPARATOR {
        return Err(format!(
            "line {separator_line}: ADR index table must use canonical separator '{SEPARATOR}'"
        ));
    }

    let mut rows = Vec::new();
    for (line_number, line) in lines.iter().skip(header_index + 2) {
        if line.is_empty() || !line.starts_with('|') {
            break;
        }
        let row = parse_index_row(*line_number, line).ok_or_else(|| {
            format!("line {line_number}: malformed row in canonical ADR index table")
        })?;
        rows.push(row);
    }
    if rows.is_empty() {
        return Err("canonical ADR index table contains no ADR rows".to_owned());
    }
    Ok(rows)
}

#[derive(Debug)]
struct IndexRow {
    id: u16,
    target: String,
    title: String,
    status: String,
    authority: String,
    line: usize,
}

fn parse_index_row(line_number: usize, line: &str) -> Option<IndexRow> {
    let columns = line.split('|').map(str::trim).collect::<Vec<_>>();
    if columns.len() != 6 || !columns[0].is_empty() || !columns[5].is_empty() {
        return None;
    }
    let (label, target) = first_markdown_link(columns[1])?;
    if columns[1] != format!("[{label}]({target})") {
        return None;
    }
    if label.len() != 4 || !label.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(IndexRow {
        id: label.parse().ok()?,
        target,
        title: columns[2].to_owned(),
        status: columns[3].to_owned(),
        authority: columns[4].to_owned(),
        line: line_number,
    })
}

fn markdown_links(markdown: &str) -> Result<Vec<String>, String> {
    let mut links = Vec::new();
    for (line_number, line) in markdown_visible_lines(markdown)? {
        for segment in markdown_prose_segments(line)
            .map_err(|message| format!("line {line_number}: {message}"))?
        {
            if contains_reference_style_syntax(segment) {
                return Err(
                    "reference-style Markdown links and definitions are not allowed; use inline [label](target) links"
                        .to_owned(),
                );
            }

            let mut remainder = segment;
            while let Some(close_label) = remainder.find("](") {
                let after = &remainder[close_label + 2..];
                let Some(close_target) = after.find(')') else {
                    return Err(format!(
                        "line {line_number}: unclosed inline Markdown link target"
                    ));
                };
                let raw_target = after[..close_target].trim();
                let target = if let Some(angle_target) = raw_target.strip_prefix('<') {
                    let Some((target, suffix)) = angle_target.split_once('>') else {
                        return Err(format!(
                            "line {line_number}: angle-bracket Markdown link target is missing '>'"
                        ));
                    };
                    if !suffix.trim().is_empty() {
                        return Err(format!(
                            "line {line_number}: text after an angle-bracket Markdown link target is not supported"
                        ));
                    }
                    target
                } else {
                    if raw_target.chars().any(char::is_whitespace) {
                        return Err(format!(
                            "line {line_number}: whitespace or link titles in a raw Markdown target are not supported; use one angle-bracket target"
                        ));
                    }
                    raw_target
                };
                if target.is_empty() {
                    return Err(format!(
                        "line {line_number}: inline Markdown link target must not be empty"
                    ));
                }
                links.push(target.to_owned());
                remainder = &after[close_target + 1..];
            }
        }
    }
    Ok(links)
}

fn markdown_visible_lines(markdown: &str) -> Result<Vec<(usize, &str)>, String> {
    let mut lines = Vec::new();
    let mut fence: Option<MarkdownFence> = None;
    for (line_index, line) in markdown.lines().enumerate() {
        let line_number = line_index + 1;
        if let Some(open) = fence {
            if markdown_fence_run(line).is_some_and(|candidate| {
                candidate.marker == open.marker
                    && candidate.length >= open.length
                    && candidate.suffix.trim().is_empty()
            }) {
                fence = None;
            }
            continue;
        }
        let (_, content) = markdown_indentation(line);
        if content.contains("<!--") || is_unsupported_html_block_start(content) {
            return Err(format!(
                "line {line_number}: raw HTML blocks and comments are not allowed in governed Markdown"
            ));
        }
        if let Some(candidate) = markdown_fence_run(line) {
            if candidate.marker == b'`' && candidate.suffix.contains('`') {
                return Err(format!(
                    "line {line_number}: a backtick fence info string must not contain backticks"
                ));
            }
            fence = Some(MarkdownFence {
                marker: candidate.marker,
                length: candidate.length,
                line: line_number,
            });
            continue;
        }
        lines.push((line_number, line));
    }
    if let Some(open) = fence {
        let marker = char::from(open.marker).to_string().repeat(open.length);
        return Err(format!(
            "unclosed '{marker}' fenced code block starting on line {}",
            open.line
        ));
    }
    Ok(lines)
}

fn is_unsupported_html_block_start(content: &str) -> bool {
    let Some(after_open) = content.strip_prefix('<') else {
        return false;
    };
    after_open.starts_with("!--")
        || after_open.starts_with(['!', '?', '/'])
        || after_open
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
}

#[derive(Clone, Copy)]
struct MarkdownFence {
    marker: u8,
    length: usize,
    line: usize,
}

struct MarkdownFenceRun<'a> {
    marker: u8,
    length: usize,
    suffix: &'a str,
}

fn markdown_fence_run(line: &str) -> Option<MarkdownFenceRun<'_>> {
    let (indent, content) = markdown_indentation(line);
    if indent > 3 {
        return None;
    }
    let marker = *content.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = content.bytes().take_while(|byte| *byte == marker).count();
    (length >= 3).then(|| MarkdownFenceRun {
        marker,
        length,
        suffix: &content[length..],
    })
}

fn markdown_indentation(line: &str) -> (usize, &str) {
    let mut columns = 0;
    let mut byte_index = 0;
    for (index, character) in line.char_indices() {
        match character {
            ' ' => columns += 1,
            '\t' => columns += 4 - (columns % 4),
            _ => {
                byte_index = index;
                return (columns, &line[byte_index..]);
            }
        }
        byte_index = index + character.len_utf8();
    }
    (columns, &line[byte_index..])
}

fn markdown_prose_segments(line: &str) -> Result<Vec<&str>, String> {
    let mut segments = Vec::new();
    let mut segment_start = 0;
    let mut cursor = 0;
    while cursor < line.len() {
        let Some(open_offset) = line[cursor..].find('`') else {
            break;
        };
        let open = cursor + open_offset;
        if is_escaped(line, open) {
            cursor = open + 1;
            continue;
        }
        let delimiter_length = line[open..]
            .bytes()
            .take_while(|byte| *byte == b'`')
            .count();
        let content_start = open + delimiter_length;
        let close = find_matching_backtick_run(line, content_start, delimiter_length)
            .ok_or_else(|| "unclosed inline code span".to_owned())?;
        if segment_start < open {
            segments.push(&line[segment_start..open]);
        }
        cursor = close + delimiter_length;
        segment_start = cursor;
    }
    if segment_start < line.len() {
        segments.push(&line[segment_start..]);
    }
    Ok(segments)
}

fn markdown_heading_anchors(markdown: &str) -> Result<BTreeSet<String>, String> {
    let mut anchors = BTreeSet::new();
    let mut occurrences = BTreeMap::<String, usize>::new();
    for (_, line) in markdown_visible_lines(markdown)? {
        let trimmed = line.trim_start();
        let marker_count = trimmed.bytes().take_while(|byte| *byte == b'#').count();
        if !(1..=6).contains(&marker_count) || trimmed.as_bytes().get(marker_count) != Some(&b' ') {
            continue;
        }
        let heading = trimmed[marker_count + 1..]
            .trim()
            .trim_end_matches('#')
            .trim_end();
        let Some(base) = markdown_heading_slug(heading) else {
            continue;
        };
        let occurrence = occurrences.entry(base.clone()).or_default();
        let anchor = if *occurrence == 0 {
            base
        } else {
            format!("{base}-{occurrence}")
        };
        *occurrence += 1;
        anchors.insert(anchor);
    }
    Ok(anchors)
}

fn markdown_heading_slug(heading: &str) -> Option<String> {
    if heading.is_empty()
        || !heading.chars().all(|character| {
            character.is_alphanumeric()
                || character.is_whitespace()
                || matches!(character, '_' | '-')
        })
    {
        return None;
    }
    let mut slug = String::new();
    for character in heading.chars() {
        if character.is_alphanumeric() || matches!(character, '_' | '-') {
            slug.extend(character.to_lowercase());
        } else if character.is_whitespace() {
            slug.push('-');
        }
    }
    (!slug.is_empty()).then_some(slug)
}

fn find_matching_backtick_run(
    value: &str,
    mut cursor: usize,
    expected_length: usize,
) -> Option<usize> {
    while cursor < value.len() {
        let offset = value[cursor..].find('`')?;
        let start = cursor + offset;
        let length = value[start..]
            .bytes()
            .take_while(|byte| *byte == b'`')
            .count();
        if length == expected_length {
            return Some(start);
        }
        cursor = start + length;
    }
    None
}

fn contains_reference_style_syntax(line: &str) -> bool {
    let mut search_from = 0;
    while let Some(open_offset) = line[search_from..].find('[') {
        let open = search_from + open_offset;
        if is_escaped(line, open) {
            search_from = open + 1;
            continue;
        }
        let mut close_search = open + 1;
        let mut found_close = false;
        while let Some(close_offset) = line[close_search..].find(']') {
            let close = close_search + close_offset;
            if is_escaped(line, close) {
                close_search = close + 1;
                continue;
            }
            found_close = true;
            let after = line[close + 1..].trim_start_matches([' ', '\t']);
            if after.starts_with('[') || after.starts_with(':') {
                return true;
            }
            search_from = close + 1;
            break;
        }
        if !found_close {
            break;
        }
    }
    false
}

fn is_escaped(value: &str, byte_index: usize) -> bool {
    value[..byte_index]
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
}

fn first_markdown_link(markdown: &str) -> Option<(String, String)> {
    let open_label = markdown.find('[')?;
    let close_label = markdown[open_label + 1..].find("](")? + open_label + 1;
    let after = &markdown[close_label + 2..];
    let close_target = after.find(')')?;
    Some((
        markdown[open_label + 1..close_label].to_owned(),
        after[..close_target].trim().to_owned(),
    ))
}

enum LinkTarget {
    External,
    Anchor(String),
    Local {
        relative: PathBuf,
        fragment: Option<String>,
    },
}

#[derive(Clone, Copy)]
enum LinkBase {
    RepositoryRoot,
    SourceDirectory,
}

fn resolve_link(
    root: &Path,
    source: &Path,
    target: &str,
    link_base: LinkBase,
) -> Result<LinkTarget, String> {
    let target = target.trim().trim_matches(['<', '>']);
    if target.is_empty() {
        return Err("empty link".to_owned());
    }
    if target.starts_with('#') {
        let fragment = target.trim_start_matches('#');
        if fragment.is_empty() || fragment.contains('#') {
            return Err("anchor fragment must contain one non-empty identifier".to_owned());
        }
        return Ok(LinkTarget::Anchor(fragment.to_owned()));
    }
    if target.starts_with("https://")
        || target.starts_with("http://")
        || target.starts_with("mailto:")
    {
        return Ok(LinkTarget::External);
    }
    if target.contains("://") || target.starts_with("javascript:") || target.starts_with("data:") {
        return Err("unsupported URI scheme".to_owned());
    }

    let (path_and_query, fragment) = target
        .split_once('#')
        .map_or((target, None), |(path, fragment)| (path, Some(fragment)));
    if fragment.is_some_and(str::is_empty) || fragment.is_some_and(|value| value.contains('#')) {
        return Err("anchor fragment must contain one non-empty identifier".to_owned());
    }
    if path_and_query.contains('?') {
        return Err("query parameters are not supported for repository-local links".to_owned());
    }
    let path_text = path_and_query.replace('/', std::path::MAIN_SEPARATOR_STR);
    if path_text.is_empty() {
        return fragment.map_or_else(
            || Err("empty link".to_owned()),
            |fragment| Ok(LinkTarget::Anchor(fragment.to_owned())),
        );
    }
    let target_path = Path::new(&path_text);
    if target_path.is_absolute() {
        return Err("absolute filesystem paths are not reproducible".to_owned());
    }

    let source_absolute = root.join(source);
    let base = match link_base {
        LinkBase::RepositoryRoot => root.to_path_buf(),
        LinkBase::SourceDirectory => source_absolute.parent().unwrap_or(root).to_path_buf(),
    };
    let resolved = normalize_path(base.join(target_path));
    if !resolved.starts_with(root) {
        return Err("path escapes the repository root".to_owned());
    }
    let relative = resolved
        .strip_prefix(root)
        .map_err(|_| "path escapes the repository root".to_owned())?
        .to_path_buf();
    Ok(LinkTarget::Local {
        relative,
        fragment: fragment.map(str::to_owned),
    })
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn violation(code: &'static str, path: impl AsRef<Path>, message: impl Into<String>) -> Violation {
    Violation {
        code,
        path: path.as_ref().to_path_buf(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMPORARY_REPOSITORY_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn accepts_coherent_fixture() {
        let repository = TestRepository::new();
        repository.write_fixture();

        let report = check_repository(&repository.root);

        assert_eq!(report.adrs_checked, 3);
        assert!(report.violations.is_empty(), "{:#?}", report.violations);
    }

    #[test]
    fn resolves_metadata_from_root_and_markdown_from_source_directory() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement()
                .replace("## Decision\n", "## Decision\n\n[ADR index](README.md)\n"),
        );

        let report = check_repository(&repository.root);

        assert!(report.violations.is_empty(), "{:#?}", report.violations);
    }

    #[test]
    fn rejects_malformed_frontmatter() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0003-deferred-boundary.md",
            &proposed_adr().replace("status: proposed", "status proposed"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR006");
    }

    #[test]
    fn rejects_duplicate_frontmatter_key() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0003-deferred-boundary.md",
            &proposed_adr().replace("status: proposed", "status: proposed\nstatus: accepted"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR006");
    }

    #[test]
    fn rejects_unknown_status() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0003-deferred-boundary.md",
            &proposed_adr().replace("status: proposed", "status: implemented"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR006");
    }

    #[test]
    fn rejects_unknown_frontmatter_field() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0003-deferred-boundary.md",
            &proposed_adr().replace(
                "decision_type: architecture",
                "decision_type: architecture\nstate: draft",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR006");
    }

    #[test]
    fn rejects_null_in_place_of_empty_list() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("related: []", "related:"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR006");
    }

    #[test]
    fn rejects_items_after_explicit_empty_list() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("related: []", "related: []\n  - Cargo.toml"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR006");
    }

    #[test]
    fn rejects_every_unquoted_yaml_null_alias() {
        for null_alias in ["null", "Null", "NULL", "~"] {
            let scalar_repository = TestRepository::new();
            scalar_repository.write_fixture();
            scalar_repository.write(
                "docs/adr/0002-canonical-decision.md",
                &accepted_replacement().replace(
                    "decision_type: architecture",
                    &format!("decision_type: {null_alias}"),
                ),
            );
            assert_has_code(&check_repository(&scalar_repository.root), "ADR006");

            let list_repository = TestRepository::new();
            list_repository.write_fixture();
            list_repository.write(
                "docs/adr/0002-canonical-decision.md",
                &accepted_replacement().replacen("  - Test", &format!("  - {null_alias}"), 1),
            );
            assert_has_code(&check_repository(&list_repository.root), "ADR006");
        }
    }

    #[test]
    fn preserves_plain_and_quoted_scalar_token_types() {
        assert!(matches!(
            parse_scalar_token("architecture", 1),
            Ok(ScalarToken::Plain(value)) if value == "architecture"
        ));
        assert!(matches!(
            parse_scalar_token("\"null\"", 1),
            Ok(ScalarToken::Quoted(value)) if value == "null"
        ));
    }

    #[test]
    fn rejects_missing_reciprocal_supersession() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "supersedes:\n  - docs/adr/0001-old-decision.md\n",
                "supersedes: []\n",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR012");
    }

    #[test]
    fn rejects_broken_markdown_link() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "## Decision\n",
                "## Decision\n\n[Missing evidence](../testing/missing.md)\n",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR009");
    }

    #[test]
    fn rejects_reference_style_markdown_links_and_definitions() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "Canonical.",
                "Canonical.\n\n[Missing evidence][missing]\n\n[missing]: ../testing/missing.md",
            ),
        );

        let report = check_repository(&repository.root);
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR009" && violation.message.contains("reference-style Markdown")
        }));
    }

    #[test]
    fn ignores_reference_style_examples_inside_fenced_code() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "Canonical.",
                "Canonical.\n\n`[Inline][reference]` and `[label](target)` are examples.\n\n```markdown\n[Example][reference]\n[reference]: missing.md\n```",
            ),
        );

        let report = check_repository(&repository.root);
        assert!(report.violations.is_empty(), "{:#?}", report.violations);
    }

    #[test]
    fn rejects_unclosed_inline_link_and_fenced_code() {
        let link_repository = TestRepository::new();
        link_repository.write_fixture();
        link_repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("Canonical.", "Canonical.\n\n[Unclosed](../guide.md"),
        );
        assert_has_code(&check_repository(&link_repository.root), "ADR009");

        let fence_repository = TestRepository::new();
        fence_repository.write_fixture();
        fence_repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("Canonical.", "Canonical.\n\n```markdown\nexample"),
        );
        assert_has_code(&check_repository(&fence_repository.root), "ADR009");
    }

    #[test]
    fn four_space_indented_code_cannot_hide_a_broken_link() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "Canonical.",
                "Canonical.\n\n    ```\n\n[Missing](../missing.md)\n\n    ```",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR009");
    }

    #[test]
    fn rejects_ambiguous_indented_markdown_blocks() {
        for body in [
            "Canonical paragraph.\n    [Missing](../missing.md)",
            "1. List item\n\n    [Missing](../missing.md)",
        ] {
            let repository = TestRepository::new();
            repository.write_fixture();
            repository.write(
                "docs/adr/0002-canonical-decision.md",
                &accepted_replacement().replace("Canonical.", body),
            );

            assert_has_code(&check_repository(&repository.root), "ADR009");
        }
    }

    #[test]
    fn shorter_fence_does_not_close_a_longer_fence() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "Canonical.",
                "Canonical.\n\n````markdown\n[Hidden](../missing.md)\n```\n[Still hidden](../missing.md)\n````",
            ),
        );

        let report = check_repository(&repository.root);
        assert!(report.violations.is_empty(), "{:#?}", report.violations);
    }

    #[test]
    fn rejects_unclosed_angle_bracket_link_target() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement()
                .replace("Canonical.", "Canonical.\n\n[Workspace](<../../Cargo.toml)"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR009");
    }

    #[test]
    fn rejects_unsupported_markdown_link_target_suffixes() {
        for link in [
            "[Workspace](<../../Cargo.toml>garbage)",
            "[Workspace](../../Cargo.toml garbage)",
        ] {
            let repository = TestRepository::new();
            repository.write_fixture();
            repository.write(
                "docs/adr/0002-canonical-decision.md",
                &accepted_replacement().replace("Canonical.", &format!("Canonical.\n\n{link}")),
            );

            assert_has_code(&check_repository(&repository.root), "ADR009");
        }
    }

    #[test]
    fn validates_local_markdown_fragments() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write("docs/guide.md", "# Existing Heading\n");
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "Canonical.",
                "Canonical.\n\n[Valid](../guide.md#existing-heading)\n[Missing](../guide.md#missing)\n[Pure](#missing)",
            ),
        );

        let report = check_repository(&repository.root);
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR009" && violation.message.contains("#missing")
        }));
        assert!(!report.violations.iter().any(|violation| {
            violation.code == "ADR009" && violation.message.contains("existing-heading")
        }));
    }

    #[test]
    fn fragments_require_plain_atx_heading_targets() {
        for heading in [
            "# [Existing Heading](other.md)\n",
            "Existing Heading\n================\n",
        ] {
            let repository = TestRepository::new();
            repository.write_fixture();
            repository.write("docs/guide.md", heading);
            repository.write(
                "docs/adr/0002-canonical-decision.md",
                &accepted_replacement().replace(
                    "Canonical.",
                    "Canonical.\n\n[Unsupported anchor](../guide.md#existing-heading)",
                ),
            );

            assert_has_code(&check_repository(&repository.root), "ADR009");
        }
    }

    #[test]
    fn rejects_fragments_for_non_markdown_targets() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("  - Cargo.toml", "  - Cargo.toml#workspace"),
        );

        let report = check_repository(&repository.root);
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR009" && violation.message.contains("non-Markdown")
        }));
    }

    #[test]
    fn rejects_accepted_dependency_on_proposed_adr() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "related: []",
                "depends_on:\n  - docs/adr/0003-deferred-boundary.md\nrelated: []",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR011");
    }

    #[test]
    fn rejects_dependency_cycle() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "related: []",
                "depends_on:\n  - docs/adr/0003-deferred-boundary.md\nrelated: []",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR015");
    }

    #[test]
    fn rejects_gitignored_reference_tree_link() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "## Decision\n",
                "## Decision\n\n[Local reference](../../repo-ref/assimp/Readme.md)\n",
            ),
        );

        let report = check_repository(&repository.root);
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR009"
                && violation.message.contains("forbidden repository metadata")
        }));
    }

    #[test]
    fn rejects_forbidden_path_components_without_case_sensitivity() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write("repo-ref/reference.md", "local only\n");
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "Canonical.",
                "Canonical.\n\n[Local reference](../../REPO-REF/reference.md)",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR009");
    }

    #[test]
    fn rejects_repository_paths_with_incorrect_case() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("  - Cargo.toml", "  - cargo.toml"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR009");
    }

    #[test]
    fn rejects_repository_links_that_physically_escape_the_root() {
        let repository = TestRepository::new();
        repository.write_fixture();
        let identifier = TEMPORARY_REPOSITORY_ID.fetch_add(1, Ordering::Relaxed);
        let outside = std::env::temp_dir().join(format!(
            "baozi-xtask-outside-{}-{identifier}.md",
            std::process::id()
        ));
        fs::write(&outside, "# Outside\n").expect("write outside fixture");
        let link = repository.root.join("escape.md");
        if let Err(error) = create_file_symlink(&outside, &link) {
            fs::remove_file(&outside).expect("remove outside fixture after symlink skip");
            eprintln!("skipping symlink assertion because fixture creation failed: {error}");
            return;
        }
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("  - Cargo.toml", "  - escape.md"),
        );

        let report = check_repository(&repository.root);
        fs::remove_file(&outside).expect("remove outside fixture");
        assert_has_code(&report, "ADR009");
    }

    #[test]
    fn rejects_repository_internal_symlink_aliases() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write("repo-ref/private.md", "# Private\n");
        let link = repository.root.join("evidence.md");
        if let Err(error) = create_file_symlink(&repository.root.join("repo-ref/private.md"), &link)
        {
            eprintln!("skipping symlink assertion because fixture creation failed: {error}");
            return;
        }
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("  - Cargo.toml", "  - evidence.md"),
        );

        let report = check_repository(&repository.root);
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR009"
                && violation.message.contains("symbolic link or reparse point")
        }));
    }

    #[test]
    fn enforces_durable_git_state_for_links_and_promotion_tracers() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(".gitignore", "/target\n");
        repository.write("target/generated.txt", "generated\n");
        repository.write("target/ignored.rs", "pub fn ignored_tracer() {}\n");
        repository.write("src/untracked.rs", "pub fn untracked() {}\n");
        repository.write("src/staged.rs", "pub fn staged() {}\n");
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement()
                .replace("  - Cargo.toml", "  - Cargo.toml\n  - target/generated.txt"),
        );
        repository.write(
            "docs/adr/0003-deferred-boundary.md",
            &accepted_deferred_adr_with_identity(3, "Deferred Boundary", "src/untracked.rs"),
        );
        repository.write(
            "docs/adr/0004-ignored-boundary.md",
            &accepted_deferred_adr_with_identity(4, "Ignored Boundary", "target/ignored.rs"),
        );
        repository.write(
            "docs/adr/0005-staged-boundary.md",
            &accepted_deferred_adr_with_identity(5, "Staged Boundary", "src/staged.rs"),
        );
        repository.write_index(&[
            index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
            index_row(
                2,
                "0002-canonical-decision.md",
                "Canonical Decision",
                "Accepted",
            ),
            index_row(
                3,
                "0003-deferred-boundary.md",
                "Deferred Boundary",
                "Accepted",
            ),
            index_row(
                4,
                "0004-ignored-boundary.md",
                "Ignored Boundary",
                "Accepted",
            ),
            index_row(5, "0005-staged-boundary.md", "Staged Boundary", "Accepted"),
        ]);
        repository.initialize_git();
        repository.stage("src/staged.rs");

        let report = check_repository(&repository.root);
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR009" && violation.message.contains("Git-ignored")
        }));
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR010"
                && violation.message.contains("src/untracked.rs")
                && violation.message.contains("not tracked or staged")
        }));
        assert!(report.violations.iter().any(|violation| {
            violation.code == "ADR010"
                && violation.message.contains("target/ignored.rs")
                && violation.message.contains("Git-ignored")
        }));
        assert!(
            !report
                .violations
                .iter()
                .any(|violation| violation.message.contains("src/staged.rs"))
        );
    }

    #[test]
    fn rejects_deferred_accepted_adr_without_tracer() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0037-archive-contract.md",
            &deferred_accepted_adr(),
        );
        repository.write_index(&[
            index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
            index_row(
                2,
                "0002-canonical-decision.md",
                "Canonical Decision",
                "Accepted",
            ),
            index_row(
                3,
                "0003-deferred-boundary.md",
                "Deferred Boundary",
                "Proposed",
            ),
            index_row(
                37,
                "0037-archive-contract.md",
                "Archive Contract",
                "Accepted",
            ),
        ]);

        assert_has_code(&check_repository(&repository.root), "ADR010");
    }

    #[test]
    fn rejects_duplicate_index_entry() {
        let repository = TestRepository::new();
        repository.write_fixture();
        let duplicate = index_row(
            2,
            "0002-canonical-decision.md",
            "Canonical Decision",
            "Accepted",
        );
        repository.write_index(&[
            index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
            duplicate.clone(),
            duplicate,
            index_row(
                3,
                "0003-deferred-boundary.md",
                "Deferred Boundary",
                "Proposed",
            ),
        ]);

        assert_has_code(&check_repository(&repository.root), "ADR016");
    }

    #[test]
    fn fenced_code_cannot_supply_an_adr_index_row() {
        let repository = TestRepository::new();
        repository.write_fixture();
        let content = format!(
            "# ADR Index\n\n| ADR | Title | Status | Authority |\n| --- | --- | --- | --- |\n{}\n{}\n\n```markdown\n{}\n```\n",
            index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
            index_row(
                3,
                "0003-deferred-boundary.md",
                "Deferred Boundary",
                "Proposed"
            ),
            index_row(
                2,
                "0002-canonical-decision.md",
                "Canonical Decision",
                "Accepted"
            )
        );
        repository.write("docs/adr/README.md", &content);

        let report = check_repository(&repository.root);
        assert!(
            report.violations.iter().any(|violation| {
                violation.code == "ADR016" && violation.message.contains("0002")
            })
        );
    }

    #[test]
    fn four_space_indented_code_cannot_supply_the_adr_index_table() {
        let repository = TestRepository::new();
        repository.write_fixture();
        let content = format!(
            "# ADR Index\n\n    | ADR | Title | Status | Authority |\n    | --- | --- | --- | --- |\n    {}\n    {}\n    {}\n",
            index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
            index_row(
                2,
                "0002-canonical-decision.md",
                "Canonical Decision",
                "Accepted"
            ),
            index_row(
                3,
                "0003-deferred-boundary.md",
                "Deferred Boundary",
                "Proposed"
            )
        );
        repository.write("docs/adr/README.md", &content);

        assert_has_code(&check_repository(&repository.root), "ADR016");
    }

    #[test]
    fn raw_html_cannot_hide_the_adr_index_table() {
        for (open, close) in [
            ("<!--", "-->"),
            ("Visible prefix <!--", "-->"),
            ("<pre>", "</pre>"),
        ] {
            let repository = TestRepository::new();
            repository.write_fixture();
            let content = format!(
                "# ADR Index\n\n{open}\n| ADR | Title | Status | Authority |\n| --- | --- | --- | --- |\n{}\n{}\n{}\n{close}\n",
                index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
                index_row(
                    2,
                    "0002-canonical-decision.md",
                    "Canonical Decision",
                    "Accepted"
                ),
                index_row(
                    3,
                    "0003-deferred-boundary.md",
                    "Deferred Boundary",
                    "Proposed"
                )
            );
            repository.write("docs/adr/README.md", &content);

            assert_has_code(&check_repository(&repository.root), "ADR016");
        }
    }

    #[test]
    fn rejects_a_symlinked_adr_index_source() {
        let repository = TestRepository::new();
        repository.write_fixture();
        let index_path = repository.root.join("docs/adr/README.md");
        let content = fs::read_to_string(&index_path).expect("read canonical index fixture");
        repository.write("repo-ref/generated-index.md", &content);
        fs::remove_file(&index_path).expect("remove canonical index fixture");
        if let Err(error) = create_file_symlink(
            &repository.root.join("repo-ref/generated-index.md"),
            &index_path,
        ) {
            eprintln!("skipping symlink assertion because fixture creation failed: {error}");
            return;
        }

        assert_has_code(&check_repository(&repository.root), "ADR016");
    }

    #[test]
    fn rejects_noncanonical_adr_index_cells() {
        for malformed_row in [
            "| [0002](0002-canonical-decision.md) extra | Canonical Decision | Accepted | Normative |",
            "| [0002](0002-canonical-decision.md) | Canonical Decision | Accepted | Normative | extra |",
        ] {
            let repository = TestRepository::new();
            repository.write_fixture();
            repository.write_index(&[
                index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
                malformed_row.to_owned(),
                index_row(
                    3,
                    "0003-deferred-boundary.md",
                    "Deferred Boundary",
                    "Proposed",
                ),
            ]);

            assert_has_code(&check_repository(&repository.root), "ADR016");
        }
    }

    #[test]
    fn rejects_uppercase_markdown_extension_instead_of_ignoring_adr() {
        let repository = TestRepository::new();
        repository.write_fixture();
        fs::remove_file(repository.root.join("docs/adr/0002-canonical-decision.md"))
            .expect("remove canonical lowercase ADR");
        repository.write(
            "docs/adr/0002-canonical-decision.MD",
            &accepted_replacement(),
        );

        assert_has_code(&check_repository(&repository.root), "ADR003");
    }

    #[test]
    fn rejects_lexically_duplicate_frontmatter_paths() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace("  - Cargo.toml", "  - Cargo.toml\n  - ./Cargo.toml"),
        );

        assert_has_code(&check_repository(&repository.root), "ADR006");
    }

    #[test]
    fn rejects_zero_adr_identifier() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0000-zero-decision.md",
            "---\nstatus: accepted\ndate: 2026-07-10\nauthors:\n  - Test\ndecision_type: architecture\n---\n\n# ADR 0000: Zero Decision\n",
        );

        assert_has_code(&check_repository(&repository.root), "ADR003");
    }

    #[test]
    fn rejects_unpadded_heading_identifier() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "# ADR 0002: Canonical Decision",
                "# ADR 2: Canonical Decision",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR007");
    }

    #[test]
    fn rejects_heading_hidden_inside_fenced_code() {
        let repository = TestRepository::new();
        repository.write_fixture();
        repository.write(
            "docs/adr/0002-canonical-decision.md",
            &accepted_replacement().replace(
                "# ADR 0002: Canonical Decision",
                "# Example\n\n```text\n# ADR 0002: Canonical Decision\n```",
            ),
        );

        assert_has_code(&check_repository(&repository.root), "ADR007");
    }

    fn assert_has_code(report: &CheckReport, expected: &str) {
        assert!(
            report
                .violations
                .iter()
                .any(|violation| violation.code == expected),
            "expected {expected}, got {:#?}",
            report.violations
        );
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    struct TestRepository {
        root: PathBuf,
    }

    impl TestRepository {
        fn new() -> Self {
            let identifier = TEMPORARY_REPOSITORY_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "baozi-xtask-adr-test-{}-{identifier}",
                std::process::id()
            ));
            if root.exists() {
                fs::remove_dir_all(&root).expect("remove stale test repository");
            }
            fs::create_dir_all(root.join("docs/adr")).expect("create test ADR directory");
            fs::write(root.join("Cargo.toml"), "[workspace]\n").expect("write test workspace");
            Self { root }
        }

        fn write_fixture(&self) {
            self.write("docs/adr/0001-old-decision.md", &superseded_adr());
            self.write(
                "docs/adr/0002-canonical-decision.md",
                &accepted_replacement(),
            );
            self.write("docs/adr/0003-deferred-boundary.md", &proposed_adr());
            self.write_index(&[
                index_row(1, "0001-old-decision.md", "Old Decision", "Superseded"),
                index_row(
                    2,
                    "0002-canonical-decision.md",
                    "Canonical Decision",
                    "Accepted",
                ),
                index_row(
                    3,
                    "0003-deferred-boundary.md",
                    "Deferred Boundary",
                    "Proposed",
                ),
            ]);
        }

        fn write_index(&self, rows: &[String]) {
            let mut content = String::from(
                "# ADR Index\n\n| ADR | Title | Status | Authority |\n| --- | --- | --- | --- |\n",
            );
            for row in rows {
                content.push_str(row);
                content.push('\n');
            }
            self.write("docs/adr/README.md", &content);
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create fixture parent");
            }
            fs::write(path, content).expect("write fixture");
        }

        fn initialize_git(&self) {
            let output = Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&self.root)
                .output()
                .expect("execute git init");
            assert!(
                output.status.success(),
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn stage(&self, relative: &str) {
            let output = Command::new("git")
                .args(["add", "--"])
                .arg(relative)
                .current_dir(&self.root)
                .output()
                .expect("execute git add");
            assert!(
                output.status.success(),
                "git add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let temporary_root = std::env::temp_dir();
            assert!(self.root.starts_with(&temporary_root));
            assert!(
                self.root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("baozi-xtask-adr-test-"))
            );
            fs::remove_dir_all(&self.root).expect("remove test repository");
        }
    }

    fn superseded_adr() -> String {
        "---\nstatus: superseded\ndate: 2026-07-10\nauthors:\n  - Test\ndecision_type: architecture\nsuperseded_by:\n  - docs/adr/0002-canonical-decision.md\n---\n\n# ADR 0001: Old Decision\n\n## Decision\n\nHistorical.\n"
            .to_owned()
    }

    fn accepted_replacement() -> String {
        "---\nstatus: accepted\ndate: 2026-07-10\nauthors:\n  - Test\ndecision_type: architecture\nrelated: []\nsupersedes:\n  - docs/adr/0001-old-decision.md\nevidence:\n  - Cargo.toml\n---\n\n# ADR 0002: Canonical Decision\n\n## Decision\n\nCanonical.\n"
            .to_owned()
    }

    fn proposed_adr() -> String {
        "---\nstatus: proposed\ndate: 2026-07-10\nauthors:\n  - Test\ndecision_type: architecture\ndepends_on:\n  - docs/adr/0002-canonical-decision.md\npromotion_requires:\n  - implementation_tracer\n---\n\n# ADR 0003: Deferred Boundary\n\n## Decision\n\nDeferred.\n"
            .to_owned()
    }

    fn accepted_deferred_adr_with_identity(id: u16, title: &str, tracer: &str) -> String {
        proposed_adr()
            .replace("status: proposed", "status: accepted")
            .replace(
                "# ADR 0003: Deferred Boundary",
                &format!("# ADR {id:04}: {title}"),
            )
            .replace(
                "promotion_requires:\n",
                &format!("tracers:\n  - {tracer}\npromotion_requires:\n"),
            )
    }

    fn deferred_accepted_adr() -> String {
        "---\nstatus: accepted\ndate: 2026-07-10\nauthors:\n  - Test\ndecision_type: architecture\npromotion_requires:\n  - implementation_tracer\n---\n\n# ADR 0037: Archive Contract\n\n## Decision\n\nPremature.\n"
            .to_owned()
    }

    fn index_row(id: u16, target: &str, title: &str, status: &str) -> String {
        let authority = match status {
            "Accepted" => "Normative",
            "Proposed" => "Non-authoritative",
            _ => "Historical",
        };
        format!("| [{id:04}]({target}) | {title} | {status} | {authority} |")
    }
}
