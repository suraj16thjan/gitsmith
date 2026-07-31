use crate::backend::json::{nested_str, num, s};
use crate::backend::{Backend, Row, Tab};
use crate::format::relative_time_str;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

pub struct GlabBackend;

fn to_row(tab: Tab, v: &Value, now: DateTime<Utc>) -> Row {
    let updated = relative_time_str(&s(v, "updated_at"), now);
    let cells = match tab {
        Tab::Issues | Tab::MergeRequests => vec![
            num(v, "iid"),
            s(v, "title"),
            s(v, "state"),
            nested_str(v, "author", "username"),
            updated,
        ],
        Tab::Pipelines => vec![num(v, "id"), s(v, "status"), s(v, "ref"), updated],
        Tab::Runners => vec![num(v, "id"), s(v, "description"), s(v, "status")],
        Tab::Releases => vec![
            s(v, "tag_name"),
            s(v, "name"),
            relative_time_str(&s(v, "released_at"), now),
        ],
        Tab::Tags => vec![
            s(v, "name"),
            nested_str(v, "commit", "short_id"),
            nested_str(v, "commit", "title"),
        ],
        Tab::Branches => vec![
            s(v, "name"),
            protected_label(v),
            nested_str(v, "commit", "short_id"),
        ],
        Tab::Commits => vec![
            s(v, "short_id"),
            first_line(&s(v, "title")),
            s(v, "author_name"),
            relative_time_str(&s(v, "created_at"), now),
        ],
        Tab::Todos => vec![
            nested_str(v, "project", "name"),
            s(v, "body"),
            updated,
        ],
        Tab::Milestones => vec![s(v, "title"), s(v, "state"), s(v, "due_date")],
    };
    let id = match tab {
        Tab::Issues | Tab::MergeRequests | Tab::Milestones => num(v, "iid"),
        Tab::Pipelines | Tab::Runners | Tab::Todos => num(v, "id"),
        Tab::Releases => s(v, "tag_name"),
        Tab::Tags | Tab::Branches => s(v, "name"),
        Tab::Commits => s(v, "id"), // full sha, for the commit diff
    };
    Row { id, cells, web_url: s(v, "web_url"), raw: v.clone() }
}

/// "protected" (for coloring) when a branch is protected, else "".
fn protected_label(v: &Value) -> String {
    if v.get("protected").and_then(Value::as_bool) == Some(true) {
        "protected".to_string()
    } else {
        String::new()
    }
}

/// First line of a (possibly multi-line) commit message.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

pub fn parse(tab: Tab, json: &str, now: DateTime<Utc>) -> Result<Vec<Row>> {
    let value: Value = serde_json::from_str(json).context("invalid JSON from glab")?;
    let arr = value.as_array().context("expected a JSON array from glab")?;
    Ok(arr.iter().map(|v| to_row(tab, v, now)).collect())
}

/// Extract a GitLab project path (`group/subgroup/repo`) from a git remote URL.
/// Handles `https://host/a/b/c(.git)`, `ssh://git@host:22/a/b/c`, and scp-style
/// `git@host:a/b/c(.git)`.
fn project_path_from_url(url: &str) -> Option<String> {
    crate::backend::repo_from_url(url)
}

/// The current repo's GitLab project path. Honors an active repo override (set by
/// the `P` picker); otherwise reads `origin` or the first git remote.
fn current_project() -> Result<String> {
    if let Some(repo) = crate::backend::repo_override() {
        return Ok(repo);
    }
    let get = |name: &str| crate::backend::run("git", &["remote", "get-url", name]).ok();
    let url = get("origin")
        .or_else(|| {
            let remotes = crate::backend::run("git", &["remote"]).ok()?;
            get(remotes.split_whitespace().next()?)
        })
        .context("no git remote found — run glabtui inside your GitLab repo")?;
    project_path_from_url(&url).context("could not parse project path from git remote URL")
}

/// Run a `glab` command, scoping subcommands to the repo override with `-R` when
/// one is active. `api` calls and any call that already scopes itself (`--project`)
/// are left alone — they embed the override-aware project path themselves.
fn glab_run(args: &[&str]) -> Result<String> {
    if args.first() != Some(&"api")
        && !args.contains(&"--project")
        && let Some(repo) = crate::backend::repo_override()
    {
        let mut scoped: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        scoped.push("-R".into());
        scoped.push(repo);
        let argv: Vec<&str> = scoped.iter().map(String::as_str).collect();
        return crate::backend::run("glab", &argv);
    }
    crate::backend::run("glab", args)
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn glab_act_args(action: crate::backend::Action, id: &str) -> Option<Vec<String>> {
    use crate::backend::Action::*;
    Some(match action {
        IssueClose => owned(&["issue", "close", id]),
        IssueReopen => owned(&["issue", "reopen", id]),
        MrApprove => owned(&["mr", "approve", id]),
        MrMerge => owned(&["mr", "merge", id, "--yes"]),
        MrClose => owned(&["mr", "close", id]),
        PipelineCancel => owned(&["ci", "cancel", "pipeline", id]),
        PipelineRetry => return None, // no CLI pipeline-retry; act() uses the API
    })
}

/// URL-encode a GitLab project path for use in an `api projects/<path>/...` call.
fn encode_project(path: &str) -> String {
    path.replace('/', "%2F")
}

/// The `glab api` endpoint to retry a pipeline (GitLab has no CLI pipeline-retry).
fn glab_retry_path(project: &str, id: &str) -> String {
    format!("projects/{}/pipelines/{}/retry", encode_project(project), id)
}

fn glab_comment_args(id: &str, body: &str) -> Vec<String> {
    owned(&["issue", "note", id, "-m", body])
}

fn glab_diff_args(id: &str) -> Vec<String> {
    owned(&["mr", "diff", id])
}

/// `glab api` notes endpoint for an issue or MR's comment thread.
fn glab_comments_path(tab: Tab, enc_project: &str, id: &str) -> String {
    let resource = if tab == Tab::MergeRequests { "merge_requests" } else { "issues" };
    format!("projects/{enc_project}/{resource}/{id}/notes")
}

fn glab_jobs_path(enc_project: &str, pipeline_id: &str) -> String {
    format!("projects/{enc_project}/pipelines/{pipeline_id}/jobs?per_page=100")
}

/// Trigger/bridge jobs of a pipeline (parent pipelines with downstream/child
/// pipelines expose these instead of `jobs`).
fn glab_bridges_path(enc_project: &str, pipeline_id: &str) -> String {
    format!("projects/{enc_project}/pipelines/{pipeline_id}/bridges?per_page=100")
}

fn glab_job_trace_path(enc_project: &str, job_id: &str) -> String {
    format!("projects/{enc_project}/jobs/{job_id}/trace")
}

fn glab_job_act_path(enc_project: &str, job_id: &str, action: crate::backend::JobAction) -> String {
    let verb = match action {
        crate::backend::JobAction::Retry => "retry",
        crate::backend::JobAction::Cancel => "cancel",
    };
    format!("projects/{enc_project}/jobs/{job_id}/{verb}")
}

impl Backend for GlabBackend {
    fn list(&self, tab: Tab) -> Result<Vec<Row>> {
        // High-level `glab <res> list --output json` subcommands resolve the
        // current repo's project and emit the raw API JSON array the parser
        // consumes. `glab api projects/...` is avoided (placeholder substitution
        // is unreliable on self-hosted hosts); `todos` has no project scope so it
        // stays on the api endpoint; `milestone list` is the one subcommand that
        // does NOT auto-resolve the repo, so it needs an explicit --project.
        // ponytail: defaults to open items / first page; add state+pagination later.
        let args = match tab {
            Tab::Issues => owned(&["issue", "list", "--output", "json"]),
            Tab::MergeRequests => owned(&["mr", "list", "--output", "json"]),
            Tab::Pipelines => owned(&["ci", "list", "--output", "json"]),
            Tab::Runners => owned(&["runner", "list", "--output", "json"]),
            Tab::Releases => owned(&["release", "list", "--output", "json"]),
            Tab::Tags => vec![
                "api".into(),
                format!("projects/{}/repository/tags", encode_project(&current_project()?)),
            ],
            Tab::Branches => vec![
                "api".into(),
                format!("projects/{}/repository/branches", encode_project(&current_project()?)),
            ],
            Tab::Commits => vec![
                "api".into(),
                format!("projects/{}/repository/commits", encode_project(&current_project()?)),
            ],
            Tab::Todos => owned(&["api", "todos"]),
            Tab::Milestones => vec![
                "milestone".into(),
                "list".into(),
                "--project".into(),
                current_project()?,
                "--output".into(),
                "json".into(),
            ],
        };
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = glab_run(&argv)?;
        parse(tab, &out, Utc::now())
    }

    fn act(&self, action: crate::backend::Action, id: &str) -> Result<()> {
        let args: Vec<String> = match glab_act_args(action, id) {
            Some(args) => args,
            None => {
                // PipelineRetry: POST the pipeline-retry API endpoint.
                let project = current_project()?;
                let path = glab_retry_path(&project, id);
                owned(&["api", "--method", "POST", &path])
            }
        };
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        glab_run(&argv)?;
        Ok(())
    }

    fn comment(&self, id: &str, body: &str) -> Result<()> {
        let args = glab_comment_args(id, body);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        glab_run(&argv)?;
        Ok(())
    }

    fn diff(&self, _tab: crate::backend::Tab, id: &str) -> Result<String> {
        let args = glab_diff_args(id);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        glab_run(&argv)
    }

    fn comments(&self, tab: Tab, id: &str) -> Result<String> {
        let project = current_project()?;
        let path = glab_comments_path(tab, &encode_project(&project), id);
        crate::backend::run("glab", &["api", &path])
    }

    fn jobs(&self, pipeline_id: &str) -> Result<String> {
        let enc = encode_project(&current_project()?);
        let jobs = crate::backend::run("glab", &["api", &glab_jobs_path(&enc, pipeline_id)])?;
        // A parent pipeline (with downstream/child pipelines) has no direct jobs —
        // its trigger jobs are under `/bridges`. Fall back to those so it's not empty.
        let has_jobs = serde_json::from_str::<Value>(&jobs)
            .ok()
            .and_then(|v| v.as_array().map(|a| !a.is_empty()))
            .unwrap_or(false);
        if has_jobs {
            Ok(jobs)
        } else {
            crate::backend::run("glab", &["api", &glab_bridges_path(&enc, pipeline_id)]).or(Ok(jobs))
        }
    }

    fn job_log(&self, job_id: &str) -> Result<String> {
        let enc = encode_project(&current_project()?);
        crate::backend::run("glab", &["api", &glab_job_trace_path(&enc, job_id)])
    }

    fn job_act(&self, action: crate::backend::JobAction, job_id: &str) -> Result<()> {
        let enc = encode_project(&current_project()?);
        let path = glab_job_act_path(&enc, job_id, action);
        crate::backend::run("glab", &["api", "--method", "POST", &path])?;
        Ok(())
    }

    fn repos(&self) -> Result<Vec<String>> {
        let out = crate::backend::run(
            "glab",
            &["api", "projects?membership=true&per_page=100&order_by=last_activity_at"],
        )?;
        Ok(parse_repos_glab(&out))
    }

    fn commit_diff(&self, sha: &str) -> Result<String> {
        let enc = encode_project(&current_project()?);
        let out = crate::backend::run("glab", &["api", &format!("projects/{enc}/repository/commits/{sha}/diff")])?;
        Ok(reconstruct_unified(&out))
    }
}

/// GitLab's commit-diff API returns a per-file array without the `diff --git`
/// headers our parser keys on; synthesize them so the unified parser sees files.
fn reconstruct_unified(json: &str) -> String {
    let Ok(Value::Array(files)) = serde_json::from_str::<Value>(json) else {
        return String::new();
    };
    let mut out = String::new();
    for f in &files {
        let old = s(f, "old_path");
        let new = s(f, "new_path");
        out.push_str(&format!("diff --git a/{old} b/{new}\n"));
        out.push_str(&s(f, "diff"));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Extract `path_with_namespace` strings from a GitLab projects JSON array.
fn parse_repos_glab(json: &str) -> Vec<String> {
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(json) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|p| p.get("path_with_namespace").and_then(Value::as_str).map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Tab;
    use chrono::{TimeZone, Utc};

    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 30, 12, 0, 0).unwrap()
    }

    #[test]
    fn parses_issues() {
        let json = r#"[
          {"iid":7,"title":"Bug in parser","state":"opened",
           "author":{"username":"alice"},"updated_at":"2026-07-30T09:00:00Z",
           "web_url":"https://gitlab.com/o/r/-/issues/7"}
        ]"#;
        let rows = parse(Tab::Issues, json, now()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "7");
        assert_eq!(rows[0].cells, vec!["7", "Bug in parser", "opened", "alice", "3h ago"]);
        assert_eq!(rows[0].web_url, "https://gitlab.com/o/r/-/issues/7");
        // raw JSON is retained for the detail view
        assert_eq!(rows[0].raw["title"], "Bug in parser");
    }

    #[test]
    fn missing_fields_degrade_to_empty() {
        let rows = parse(Tab::Issues, r#"[{}]"#, now()).unwrap();
        assert_eq!(rows[0].id, "");
        assert_eq!(rows[0].cells, vec!["", "", "", "", ""]);
        assert_eq!(rows[0].web_url, "");
    }

    #[test]
    fn non_array_is_error() {
        assert!(parse(Tab::Issues, r#"{"message":"404"}"#, now()).is_err());
    }

    #[test]
    fn project_path_parsing() {
        let deep = "innovate-tech/jelajah/apollo/apollo-frontend/mst-enduser";
        assert_eq!(
            project_path_from_url(&format!("git@gitlab.innovatetech.io:{deep}.git")).as_deref(),
            Some(deep)
        );
        assert_eq!(
            project_path_from_url(&format!("https://gitlab.innovatetech.io/{deep}.git")).as_deref(),
            Some(deep)
        );
        assert_eq!(
            project_path_from_url("ssh://git@gitlab.innovatetech.io:22/o/r").as_deref(),
            Some("o/r")
        );
        assert_eq!(
            project_path_from_url("https://host:8080/o/r").as_deref(),
            Some("o/r")
        );
        assert_eq!(project_path_from_url("not-a-url").as_deref(), None);
    }

    #[test]
    fn act_args_mapping() {
        use crate::backend::Action::*;
        assert_eq!(glab_act_args(MrMerge, "12"), Some(vec!["mr".to_string(), "merge".into(), "12".into(), "--yes".into()]));
        assert_eq!(glab_act_args(MrApprove, "12"), Some(vec!["mr".to_string(), "approve".into(), "12".into()]));
        assert_eq!(glab_act_args(IssueClose, "7"), Some(vec!["issue".to_string(), "close".into(), "7".into()]));
        assert_eq!(glab_act_args(PipelineCancel, "99"), Some(vec!["ci".to_string(), "cancel".into(), "pipeline".into(), "99".into()]));
        assert_eq!(glab_act_args(PipelineRetry, "99"), None);
    }

    #[test]
    fn retry_uses_api_path() {
        assert_eq!(encode_project("g/sub/repo"), "g%2Fsub%2Frepo");
        assert_eq!(glab_retry_path("g/sub/repo", "99"), "projects/g%2Fsub%2Frepo/pipelines/99/retry");
    }

    #[test]
    fn comment_args_mapping() {
        assert_eq!(glab_comment_args("7", "hi"), vec!["issue", "note", "7", "-m", "hi"]);
    }

    #[test]
    fn diff_args_mapping() {
        assert_eq!(glab_diff_args("12"), vec!["mr", "diff", "12"]);
    }

    #[test]
    fn comments_path_mapping() {
        assert_eq!(glab_comments_path(Tab::Issues, "g%2Fr", "7"), "projects/g%2Fr/issues/7/notes");
        assert_eq!(glab_comments_path(Tab::MergeRequests, "g%2Fr", "12"), "projects/g%2Fr/merge_requests/12/notes");
    }

    #[test]
    fn parses_tags() {
        let json = r#"[{"name":"v1.2.0","message":"rel","commit":{"short_id":"abc123","title":"bump"}}]"#;
        let rows = parse(Tab::Tags, json, now()).unwrap();
        assert_eq!(rows[0].id, "v1.2.0");
        assert_eq!(rows[0].cells, vec!["v1.2.0", "abc123", "bump"]);
    }

    #[test]
    fn parses_branches_and_commits() {
        let b = parse(Tab::Branches, r#"[{"name":"dev","protected":false,"commit":{"short_id":"abc123"}}]"#, now()).unwrap();
        assert_eq!(b[0].cells, vec!["dev", "", "abc123"]);
        let c = parse(Tab::Commits, r#"[{"id":"fullsha0000","short_id":"fulls","title":"init","author_name":"al","created_at":"2026-07-30T11:30:00Z"}]"#, now()).unwrap();
        assert_eq!(c[0].id, "fullsha0000");
        assert_eq!(c[0].cells[0], "fulls");
    }

    #[test]
    fn reconstructs_unified_diff() {
        let json = r#"[{"old_path":"a.rs","new_path":"a.rs","diff":"@@ -1 +1 @@\n-x\n+y\n"}]"#;
        let u = reconstruct_unified(json);
        assert!(u.starts_with("diff --git a/a.rs b/a.rs\n"));
        assert!(u.contains("@@ -1 +1 @@"));
        assert!(reconstruct_unified("{}").is_empty());
    }

    #[test]
    fn parses_repo_list() {
        let json = r#"[{"path_with_namespace":"grp/a"},{"path_with_namespace":"grp/sub/b"}]"#;
        assert_eq!(parse_repos_glab(json), vec!["grp/a", "grp/sub/b"]);
        assert!(parse_repos_glab("{}").is_empty());
    }

    #[test]
    fn job_path_mapping() {
        use crate::backend::JobAction::*;
        assert_eq!(glab_jobs_path("g%2Fr", "5"), "projects/g%2Fr/pipelines/5/jobs?per_page=100");
        assert_eq!(glab_bridges_path("g%2Fr", "5"), "projects/g%2Fr/pipelines/5/bridges?per_page=100");
        assert_eq!(glab_job_trace_path("g%2Fr", "88"), "projects/g%2Fr/jobs/88/trace");
        assert_eq!(glab_job_act_path("g%2Fr", "88", Retry), "projects/g%2Fr/jobs/88/retry");
        assert_eq!(glab_job_act_path("g%2Fr", "88", Cancel), "projects/g%2Fr/jobs/88/cancel");
    }
}
