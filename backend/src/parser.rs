use crate::cost::{calculate_cost, normalize_model};
use crate::models::{ProxyLogEntry, RawEvent, UsageRecord};
use chrono::{DateTime, Utc};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct RequestAggregate {
    event: Option<RawEvent>,
    touched_paths: BTreeSet<String>,
}

#[derive(Clone, Default)]
struct RepoLayout {
    nested_repos: Vec<PathBuf>,
    has_multiple_repos: bool,
}

/// Parse a JSONL file and return deduplicated UsageRecords.
///
/// Claude Code streams each API response as multiple JSONL lines sharing the
/// same `requestId`, each carrying the **cumulative** token totals so far.
/// We keep only the LAST event per requestId (the final, complete totals).
///
/// `seen` is a global set shared across all files — the same requestId can
/// appear in both a main session file and its subagent file; `seen` prevents
/// double-counting those cross-file duplicates.
pub fn parse_jsonl_file(path: &Path, seen: &mut HashMap<String, RawEvent>) -> Vec<UsageRecord> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Cannot open {:?}: {}", path, e);
            return vec![];
        }
    };

    // Collect last event per requestId within this file first
    let mut by_request: HashMap<String, RequestAggregate> = HashMap::new();

    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            _ => continue,
        };

        let event: RawEvent = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let request_id = event
            .request_id
            .clone()
            .or_else(|| event.message.as_ref().and_then(|m| m.id.clone()))
            .unwrap_or_else(|| {
                event.timestamp.clone().unwrap_or_else(|| format!("anon-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0)))
            });

        let aggregate = by_request.entry(request_id).or_default();
        aggregate.touched_paths.extend(extract_paths_from_event(&event));

        if event.event_type.as_deref() != Some("assistant") {
            continue;
        }

        if event.is_api_error {
            continue;
        }

        if event.message.as_ref().and_then(|m| m.usage.as_ref()).is_none() {
            continue;
        }

        let model = event.message.as_ref().and_then(|m| m.model.as_deref()).unwrap_or("");
        if model == "<synthetic>" || model.is_empty() {
            continue;
        }

        // Last assistant event per requestId wins for token totals while
        // touched paths are accumulated across all request events.
        aggregate.event = Some(event);
    }

    // Merge into global seen map; skip any requestId already recorded
    let mut new_records = Vec::new();
    for (rid, aggregate) in by_request {
        let Some(event) = aggregate.event else {
            continue;
        };

        if !seen.contains_key(&rid) {
            seen.insert(rid, event.clone());
            if let Some(record) = event_to_record(event, aggregate.touched_paths.into_iter().collect()) {
                new_records.push(record);
            }
        }
    }
    new_records
}

fn event_to_record(event: RawEvent, touched_paths: Vec<String>) -> Option<UsageRecord> {
    let message = event.message?;
    let usage = message.usage?;

    let raw_model = message.model.as_deref().unwrap_or("unknown");
    let model = normalize_model(raw_model);

    let input_tokens       = usage.input_tokens.unwrap_or(0);
    let output_tokens      = usage.output_tokens.unwrap_or(0);
    let cache_write_tokens = usage.cache_creation_input_tokens.unwrap_or(0);
    let cache_read_tokens  = usage.cache_read_input_tokens.unwrap_or(0);
    let cache_write_1h_tokens = usage
        .cache_creation
        .as_ref()
        .map(|c| c.ephemeral_1h_input_tokens)
        .unwrap_or(0);

    let (cost_input, cost_output, cost_cache_write, cost_cache_read) =
        calculate_cost(input_tokens, output_tokens, cache_write_tokens, cache_write_1h_tokens, cache_read_tokens, &model);

    let total_cost = cost_input + cost_output + cost_cache_write + cost_cache_read;

    let timestamp = event
        .timestamp
        .as_deref()
        .and_then(|t| t.parse::<DateTime<Utc>>().ok())
        .unwrap_or_else(Utc::now);

    // Claude Desktop's agent mode spawns Claude Code as a subprocess and writes
    // to ~/.claude/projects/ just like the CLI does, but those calls are billed
    // against the user's monthly Claude.ai subscription, not the API key. Tag
    // them so the API-usage view can exclude them.
    let source = match event.entrypoint.as_deref() {
        Some("claude-desktop") => "claude-desktop".to_string(),
        _ => "claude-code".to_string(),
    };

    let workspace_root = event.cwd.clone().unwrap_or_default();
    let project = if workspace_root.is_empty() {
        "unknown".into()
    } else {
        extract_project_name(&workspace_root)
    };

    let request_id = event
        .request_id
        .or_else(|| message.id)
        .unwrap_or_else(|| format!("anon-{}", timestamp.timestamp_nanos_opt().unwrap_or(0)));

    Some(UsageRecord {
        request_id,
        session_id: event.session_id.unwrap_or_default(),
        project,
        source,
        workspace_root,
        touched_paths,
        subprojects: Vec::new(),
        model,
        input_tokens,
        output_tokens,
        cache_write_tokens,
        cache_read_tokens,
        cost_input,
        cost_output,
        cost_cache_write,
        cost_cache_read,
        total_cost,
        timestamp,
    })
}

fn extract_paths_from_event(event: &RawEvent) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();

    if let Some(message) = &event.message {
        if let Some(content) = &message.content {
            for item in content {
                collect_paths(item, &mut paths);
            }
        }
    }

    if let Some(tool_use_result) = &event.tool_use_result {
        collect_paths(tool_use_result, &mut paths);
    }

    paths
}

fn collect_paths(value: &serde_json::Value, paths: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_paths(item, paths);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, nested) in map {
                let key = key.to_ascii_lowercase();
                if is_path_key(&key) {
                    collect_path_value(nested, paths);
                } else {
                    collect_paths(nested, paths);
                }
            }
        }
        serde_json::Value::String(text) => {
            for path in extract_patch_paths(text) {
                paths.insert(path);
            }
        }
        _ => {}
    }
}

fn collect_path_value(value: &serde_json::Value, paths: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::String(path) => add_path_candidate(path, paths),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_path_value(item, paths);
            }
        }
        serde_json::Value::Object(map) => {
            for nested in map.values() {
                collect_path_value(nested, paths);
            }
        }
        _ => {}
    }
}

fn is_path_key(key: &str) -> bool {
    matches!(key, "file_path" | "filepath" | "path" | "paths" | "files")
}

fn add_path_candidate(candidate: &str, paths: &mut BTreeSet<String>) {
    if candidate.starts_with('/') {
        paths.insert(candidate.to_string());
    }

    for path in extract_patch_paths(candidate) {
        paths.insert(path);
    }
}

fn extract_patch_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        for prefix in ["*** Update File:", "*** Add File:", "*** Delete File:"] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let path = rest.trim().split(" -> ").next().unwrap_or("").trim();
                if path.starts_with('/') {
                    paths.push(path.to_string());
                }
            }
        }
    }

    paths
}

/// "/Users/alice/Development/org/project" → "org/project"
fn extract_project_name(cwd: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let rel = if !home.is_empty() && cwd.starts_with(&home) {
        cwd[home.len()..].trim_start_matches('/')
    } else {
        cwd.trim_start_matches('/')
    };

    let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => "unknown".into(),
        1 => parts[0].into(),
        n => format!("{}/{}", parts[n - 2], parts[n - 1]),
    }
}

/// Scan all JSONL files under ~/.claude/projects/ including subagent files.
/// Uses a global `seen` map to deduplicate requestIds across files —
/// the same requestId can appear in both a main session file and its
/// subagent file; the global map prevents double-counting.
pub fn scan_all_records() -> Vec<UsageRecord> {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return vec![],
    };
    let projects_dir = std::path::PathBuf::from(&home).join(".claude").join("projects");

    let mut all = Vec::new();
    let mut seen: HashMap<String, RawEvent> = HashMap::new();

    let project_dirs = match std::fs::read_dir(&projects_dir) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("Cannot read {:?}: {}", projects_dir, e);
            return vec![];
        }
    };

    for proj_entry in project_dirs.flatten() {
        let proj_path = proj_entry.path();
        if !proj_path.is_dir() {
            continue;
        }

        let entries = match std::fs::read_dir(&proj_path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().map_or(false, |e| e == "jsonl") {
                all.extend(parse_jsonl_file(&p, &mut seen));
            } else if p.is_dir() {
                let subagents = p.join("subagents");
                if subagents.is_dir() {
                    for agent_entry in std::fs::read_dir(&subagents).into_iter().flatten().flatten() {
                        let ap = agent_entry.path();
                        if ap.extension().map_or(false, |e| e == "jsonl") {
                            all.extend(parse_jsonl_file(&ap, &mut seen));
                        }
                    }
                }
            }
        }
    }

    // Also scan proxy logs (Copilot via Anthropic API proxy)
    all.extend(scan_proxy_records(&mut seen));

    apply_subproject_attribution(&mut all);
    all
}

/// Return the path to the proxy log directory (~/.cctrack/proxy/).
pub fn proxy_log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".cctrack").join("proxy")
}

/// Parse proxy JSONL log files written by the Anthropic proxy.
fn scan_proxy_records(seen: &mut HashMap<String, RawEvent>) -> Vec<UsageRecord> {
    let dir = proxy_log_dir();
    if !dir.is_dir() {
        return vec![];
    }

    let mut records = Vec::new();
    let mut proxy_seen: HashSet<String> = HashSet::new();

    // Collect already-seen request IDs from Claude Code logs
    for rid in seen.keys() {
        proxy_seen.insert(rid.clone());
    }

    let entries = match std::fs::read_dir(&dir) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("Cannot read proxy log dir {:?}: {}", dir, e);
            return vec![];
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "jsonl") {
            continue;
        }
        records.extend(parse_proxy_file(&path, &mut proxy_seen));
    }

    records
}

fn parse_proxy_file(path: &Path, seen: &mut HashSet<String>) -> Vec<UsageRecord> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Cannot open proxy log {:?}: {}", path, e);
            return vec![];
        }
    };

    let mut records = Vec::new();

    for line in BufReader::new(file).lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            _ => continue,
        };

        let entry: ProxyLogEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if seen.contains(&entry.request_id) {
            continue;
        }
        seen.insert(entry.request_id.clone());

        let model = normalize_model(&entry.model);
        let input_tokens = entry.input_tokens.unwrap_or(0);
        let output_tokens = entry.output_tokens.unwrap_or(0);
        let cache_write_tokens = entry.cache_creation_input_tokens.unwrap_or(0);
        let cache_read_tokens = entry.cache_read_input_tokens.unwrap_or(0);

        let (cost_input, cost_output, cost_cache_write, cost_cache_read) =
            calculate_cost(input_tokens, output_tokens, cache_write_tokens, 0, cache_read_tokens, &model);

        let total_cost = cost_input + cost_output + cost_cache_write + cost_cache_read;

        let timestamp = entry
            .timestamp
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now());

        let source = entry.source.unwrap_or_else(|| "copilot-proxy".into());

        records.push(UsageRecord {
            request_id: entry.request_id,
            session_id: String::new(),
            project: format!("copilot-proxy"),
            source,
            workspace_root: String::new(),
            touched_paths: Vec::new(),
            subprojects: Vec::new(),
            model,
            input_tokens,
            output_tokens,
            cache_write_tokens,
            cache_read_tokens,
            cost_input,
            cost_output,
            cost_cache_write,
            cost_cache_read,
            total_cost,
            timestamp,
        });
    }

    records
}

fn apply_subproject_attribution(records: &mut [UsageRecord]) {
    let mut grouped: HashMap<(String, String, String), Vec<usize>> = HashMap::new();
    for (idx, record) in records.iter().enumerate() {
        if record.workspace_root.is_empty() {
            continue;
        }

        grouped
            .entry((
                record.project.clone(),
                record.workspace_root.clone(),
                record.session_id.clone(),
            ))
            .or_default()
            .push(idx);
    }

    let mut layouts: HashMap<String, RepoLayout> = HashMap::new();

    for ((_, workspace_root, _), indexes) in grouped {
        let layout = layouts
            .entry(workspace_root.clone())
            .or_insert_with(|| discover_repo_layout(Path::new(&workspace_root)))
            .clone();

        if !layout.has_multiple_repos {
            continue;
        }

        let workspace_path = Path::new(&workspace_root);
        let mut indexes = indexes;
        indexes.sort_by_key(|idx| records[*idx].timestamp);

        let explicit: Vec<Vec<String>> = indexes
            .iter()
            .map(|idx| resolve_subprojects(workspace_path, &layout.nested_repos, &records[*idx].touched_paths))
            .collect();

        let mut session_subprojects = BTreeSet::new();
        for names in &explicit {
            for name in names {
                session_subprojects.insert(name.clone());
            }
        }
        let session_subprojects: Vec<String> = session_subprojects.into_iter().collect();

        let mut last_known = Vec::new();
        for (position, idx) in indexes.into_iter().enumerate() {
            let assigned = if !explicit[position].is_empty() {
                explicit[position].clone()
            } else if !last_known.is_empty() {
                last_known.clone()
            } else if !session_subprojects.is_empty() {
                session_subprojects.clone()
            } else {
                vec!["(workspace)".to_string()]
            };

            records[idx].subprojects = assigned.clone();
            last_known = assigned;
        }
    }
}

fn resolve_subprojects(
    workspace_root: &Path,
    nested_repos: &[PathBuf],
    touched_paths: &[String],
) -> Vec<String> {
    let mut names = BTreeSet::new();

    for touched in touched_paths {
        let touched_path = Path::new(touched);
        let matched = nested_repos
            .iter()
            .filter(|repo| touched_path.starts_with(repo))
            .max_by_key(|repo| repo.components().count());

        if let Some(repo_root) = matched {
            if let Ok(relative) = repo_root.strip_prefix(workspace_root) {
                let name = relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
                if !name.is_empty() {
                    names.insert(name);
                }
            }
        }
    }

    names.into_iter().collect()
}

const MAX_NESTED_REPO_DEPTH: usize = 4;

fn discover_repo_layout(workspace_root: &Path) -> RepoLayout {
    let root_is_repo = is_git_root(workspace_root);

    // Skip discovery for overly broad roots (home dir, etc.) — walking the
    // entire $HOME to find nested repos would take forever and is pointless.
    let home = std::env::var("HOME").unwrap_or_default();
    let is_too_broad = !home.is_empty()
        && (workspace_root == Path::new(&home)
            || workspace_root.starts_with(Path::new(&home).join("."))
            || workspace_root == Path::new("/"));
    if is_too_broad {
        return RepoLayout {
            has_multiple_repos: false,
            nested_repos: Vec::new(),
        };
    }

    let mut nested_repos = Vec::new();
    collect_nested_repos(workspace_root, workspace_root, &mut nested_repos, 0);

    RepoLayout {
        has_multiple_repos: nested_repos.len() + usize::from(root_is_repo) > 1,
        nested_repos,
    }
}

fn collect_nested_repos(workspace_root: &Path, current: &Path, nested_repos: &mut Vec<PathBuf>, depth: usize) {
    if depth > MAX_NESTED_REPO_DEPTH {
        return;
    }

    let entries = match std::fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if should_skip_dir(name) {
            continue;
        }

        if path != workspace_root && is_git_root(&path) {
            nested_repos.push(path);
            continue;
        }

        collect_nested_repos(workspace_root, &path, nested_repos, depth + 1);
    }
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | ".next"
            | "target"
            | "dist"
            | "build"
            | "vendor"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".cache"
            | ".cargo"
            | ".rustup"
            | ".npm"
            | ".pnpm-store"
            | "Library"
            | "Applications"
            | ".Trash"
    )
}

fn is_git_root(path: &Path) -> bool {
    let git_path = path.join(".git");
    git_path.is_dir() || git_path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RawMessage, RawUsage};
    use std::fs;

    #[test]
    fn collects_paths_from_tool_content_and_patches() {
        let event = RawEvent {
            event_type: Some("assistant".into()),
            request_id: Some("req-1".into()),
            is_api_error: false,
            entrypoint: None,
            session_id: Some("session-1".into()),
            cwd: Some("/tmp/workspace".into()),
            timestamp: Some("2026-03-31T00:00:00Z".into()),
            tool_use_result: None,
            message: Some(RawMessage {
                model: Some("claude-sonnet-4-6".into()),
                id: Some("msg-1".into()),
                content: Some(vec![serde_json::json!({
                    "type": "tool_use",
                    "input": {
                        "file_path": "/tmp/workspace/app/main.rs",
                        "patch": "*** Update File: /tmp/workspace/service/api.rs\n*** End Patch"
                    }
                })]),
                usage: Some(RawUsage {
                    input_tokens: Some(1),
                    output_tokens: Some(1),
                    cache_creation_input_tokens: Some(0),
                    cache_read_input_tokens: Some(0),
                    cache_creation: None,
                }),
            }),
        };

        let paths = extract_paths_from_event(&event);
        assert!(paths.contains("/tmp/workspace/app/main.rs"));
        assert!(paths.contains("/tmp/workspace/service/api.rs"));
    }

    #[test]
    fn attributes_records_to_nested_git_repos() {
        let workspace_root = std::env::temp_dir().join(format!(
            "cc-cost-parser-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let sub_a = workspace_root.join("repo-a");
        let sub_b = workspace_root.join("repo-b");

        fs::create_dir_all(sub_a.join(".git")).unwrap();
        fs::create_dir_all(sub_b.join(".git")).unwrap();
        fs::create_dir_all(sub_a.join("src")).unwrap();
        fs::create_dir_all(sub_b.join("src")).unwrap();

        let mut records = vec![
            UsageRecord {
                request_id: "req-1".into(),
                session_id: "session-1".into(),
                project: "org/workspace".into(),
                workspace_root: workspace_root.to_string_lossy().to_string(),
                touched_paths: vec![sub_a.join("src/main.rs").to_string_lossy().to_string()],
                subprojects: Vec::new(),
                model: "sonnet".into(),
                input_tokens: 1,
                output_tokens: 1,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                cost_input: 0.1,
                cost_output: 0.1,
                cost_cache_write: 0.0,
                cost_cache_read: 0.0,
                total_cost: 0.2,
                timestamp: "2026-03-31T00:00:00Z".parse().unwrap(),
            },
            UsageRecord {
                request_id: "req-2".into(),
                session_id: "session-1".into(),
                project: "org/workspace".into(),
                workspace_root: workspace_root.to_string_lossy().to_string(),
                touched_paths: vec![],
                subprojects: Vec::new(),
                model: "sonnet".into(),
                input_tokens: 1,
                output_tokens: 1,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                cost_input: 0.1,
                cost_output: 0.1,
                cost_cache_write: 0.0,
                cost_cache_read: 0.0,
                total_cost: 0.2,
                timestamp: "2026-03-31T00:01:00Z".parse().unwrap(),
            },
        ];

        apply_subproject_attribution(&mut records);

        assert_eq!(records[0].subprojects, vec!["repo-a".to_string()]);
        assert_eq!(records[1].subprojects, vec!["repo-a".to_string()]);

        let _ = fs::remove_dir_all(&workspace_root);
    }
}
