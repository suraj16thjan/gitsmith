use crate::backend::json::{nested_str, num, s};
use crate::backend::{Backend, Row, Tab};
use crate::format::relative_time_str;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

pub struct GhBackend;

fn to_row(tab: Tab, v: &Value, now: DateTime<Utc>) -> Row {
    let updated = relative_time_str(&s(v, "updated_at"), now);
    let cells = match tab {
        Tab::Issues | Tab::MergeRequests => vec![
            num(v, "number"),
            s(v, "title"),
            s(v, "state"),
            nested_str(v, "user", "login"),
            updated,
        ],
        Tab::Pipelines => vec![num(v, "id"), s(v, "status"), s(v, "head_branch"), updated],
        Tab::Runners => vec![num(v, "id"), s(v, "name"), s(v, "status")],
        Tab::Releases => vec![
            s(v, "tag_name"),
            s(v, "name"),
            relative_time_str(&s(v, "published_at"), now),
        ],
        Tab::Tags => vec![
            s(v, "name"),
            nested_str(v, "commit", "sha").chars().take(8).collect(),
            String::new(),
        ],
        Tab::Branches => vec![
            s(v, "name"),
            protected_label(v),
            nested_str(v, "commit", "sha").chars().take(8).collect(),
        ],
        Tab::Commits => vec![
            s(v, "sha").chars().take(8).collect(),
            first_line(&nested_str(v, "commit", "message")),
            v.get("commit").map(|c| nested_str(c, "author", "name")).unwrap_or_default(),
            relative_time_str(&v.get("commit").map(|c| nested_str(c, "author", "date")).unwrap_or_default(), now),
        ],
        Tab::Todos => vec![
            nested_str(v, "repository", "full_name"),
            nested_str(v, "subject", "title"),
            relative_time_str(&s(v, "updated_at"), now),
        ],
        Tab::Milestones => vec![s(v, "title"), s(v, "state"), s(v, "due_on")],
    };
    let id = match tab {
        Tab::Issues | Tab::MergeRequests => num(v, "number"),
        Tab::Pipelines | Tab::Runners | Tab::Todos => num(v, "id"),
        Tab::Releases => s(v, "tag_name"),
        Tab::Tags | Tab::Branches => s(v, "name"),
        Tab::Commits => s(v, "sha"),
        Tab::Milestones => num(v, "number"),
    };
    Row { id, cells, web_url: s(v, "html_url"), raw: v.clone() }
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

/// gh sometimes wraps the array in an object; return the array Value for this tab.
fn array_of(tab: Tab, value: &Value) -> Option<&Vec<Value>> {
    match tab {
        Tab::Pipelines => value.get("workflow_runs").and_then(Value::as_array),
        Tab::Runners => value.get("runners").and_then(Value::as_array),
        _ => value.as_array(),
    }
}

pub fn parse(tab: Tab, json: &str, now: DateTime<Utc>) -> Result<Vec<Row>> {
    let value: Value = serde_json::from_str(json).context("invalid JSON from gh")?;
    let arr = array_of(tab, &value).context("expected a JSON array from gh")?;
    Ok(arr.iter().map(|v| to_row(tab, v, now)).collect())
}

fn gh_path(tab: Tab) -> &'static str {
    // ponytail: relies on gh's current-repo detection; add owner/repo override later.
    match tab {
        Tab::Issues => "repos/{owner}/{repo}/issues",
        Tab::MergeRequests => "repos/{owner}/{repo}/pulls",
        Tab::Pipelines => "repos/{owner}/{repo}/actions/runs",
        Tab::Runners => "repos/{owner}/{repo}/actions/runners",
        Tab::Releases => "repos/{owner}/{repo}/releases",
        Tab::Tags => "repos/{owner}/{repo}/tags",
        Tab::Branches => "repos/{owner}/{repo}/branches",
        Tab::Commits => "repos/{owner}/{repo}/commits",
        Tab::Todos => "notifications",
        Tab::Milestones => "repos/{owner}/{repo}/milestones",
    }
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn gh_act_args(action: crate::backend::Action, id: &str) -> Vec<String> {
    use crate::backend::Action::*;
    match action {
        IssueClose => owned(&["issue", "close", id]),
        IssueReopen => owned(&["issue", "reopen", id]),
        MrApprove => owned(&["pr", "review", "--approve", id]),
        MrMerge => owned(&["pr", "merge", id, "--merge"]),
        MrClose => owned(&["pr", "close", id]),
        PipelineRetry => owned(&["run", "rerun", id]),
        PipelineCancel => owned(&["run", "cancel", id]),
    }
}

fn gh_comment_args(id: &str, body: &str) -> Vec<String> {
    owned(&["issue", "comment", id, "-b", body])
}

fn gh_diff_args(id: &str) -> Vec<String> {
    owned(&["pr", "diff", id])
}

/// `gh api` comments endpoint — PR conversation comments live under issues too.
fn gh_comments_args(id: &str) -> Vec<String> {
    vec!["api".to_string(), format!("repos/{{owner}}/{{repo}}/issues/{id}/comments")]
}

fn gh_jobs_args(pipeline_id: &str) -> Vec<String> {
    vec!["api".to_string(), format!("repos/{{owner}}/{{repo}}/actions/runs/{pipeline_id}/jobs")]
}

fn gh_job_log_args(job_id: &str) -> Vec<String> {
    owned(&["run", "view", "--job", job_id, "--log"])
}

fn gh_job_retry_args(job_id: &str) -> Vec<String> {
    owned(&["run", "rerun", "--job", job_id])
}

impl Backend for GhBackend {
    fn list(&self, tab: Tab) -> Result<Vec<Row>> {
        let out = crate::backend::run("gh", &["api", gh_path(tab)])?;
        parse(tab, &out, Utc::now())
    }

    fn act(&self, action: crate::backend::Action, id: &str) -> Result<()> {
        let args = gh_act_args(action, id);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)?;
        Ok(())
    }

    fn comment(&self, id: &str, body: &str) -> Result<()> {
        let args = gh_comment_args(id, body);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)?;
        Ok(())
    }

    fn diff(&self, _tab: crate::backend::Tab, id: &str) -> Result<String> {
        let args = gh_diff_args(id);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)
    }

    fn comments(&self, _tab: crate::backend::Tab, id: &str) -> Result<String> {
        let args = gh_comments_args(id);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)
    }

    fn jobs(&self, pipeline_id: &str) -> Result<String> {
        let args = gh_jobs_args(pipeline_id);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)
    }

    fn job_log(&self, job_id: &str) -> Result<String> {
        let args = gh_job_log_args(job_id);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)
    }

    fn job_act(&self, action: crate::backend::JobAction, job_id: &str) -> Result<()> {
        use crate::backend::JobAction::*;
        match action {
            Retry => {
                let args = gh_job_retry_args(job_id);
                let argv: Vec<&str> = args.iter().map(String::as_str).collect();
                crate::backend::run("gh", &argv)?;
                Ok(())
            }
            // GitHub Actions has no single-job cancel — only whole-run.
            Cancel => anyhow::bail!("GitHub can't cancel a single job (only the whole run)"),
        }
    }

    fn repos(&self) -> Result<Vec<String>> {
        let out = crate::backend::run("gh", &["repo", "list", "--limit", "200", "--json", "nameWithOwner"])?;
        Ok(parse_repos_gh(&out))
    }

    fn commit_diff(&self, sha: &str) -> Result<String> {
        // The diff media type returns a ready-made unified diff.
        crate::backend::run(
            "gh",
            &["api", &format!("repos/{{owner}}/{{repo}}/commits/{sha}"), "-H", "Accept: application/vnd.github.diff"],
        )
    }
}

/// Extract `nameWithOwner` strings from `gh repo list --json nameWithOwner`.
fn parse_repos_gh(json: &str) -> Vec<String> {
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(json) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|r| r.get("nameWithOwner").and_then(Value::as_str).map(String::from))
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
    fn parses_pulls() {
        let json = r#"[
          {"number":42,"title":"Add feature","state":"open",
           "user":{"login":"bob"},"updated_at":"2026-07-30T11:30:00Z",
           "html_url":"https://github.com/o/r/pull/42"}
        ]"#;
        let rows = parse(Tab::MergeRequests, json, now()).unwrap();
        assert_eq!(rows[0].id, "42");
        assert_eq!(rows[0].cells, vec!["42", "Add feature", "open", "bob", "30m ago"]);
        assert_eq!(rows[0].web_url, "https://github.com/o/r/pull/42");
        // raw JSON is retained for the detail view
        assert_eq!(rows[0].raw["title"], "Add feature");
    }

    #[test]
    fn parses_actions_runs_wrapper() {
        let json = r#"{"workflow_runs":[
          {"id":9,"status":"completed","head_branch":"main","updated_at":"2026-07-30T09:00:00Z",
           "html_url":"https://github.com/o/r/actions/runs/9"}
        ]}"#;
        let rows = parse(Tab::Pipelines, json, now()).unwrap();
        assert_eq!(rows[0].cells, vec!["9", "completed", "main", "3h ago"]);
    }

    #[test]
    fn act_args_mapping() {
        use crate::backend::Action::*;
        assert_eq!(gh_act_args(MrMerge, "12"), vec!["pr", "merge", "12", "--merge"]);
        assert_eq!(gh_act_args(MrApprove, "12"), vec!["pr", "review", "--approve", "12"]);
        assert_eq!(gh_act_args(IssueClose, "7"), vec!["issue", "close", "7"]);
        assert_eq!(gh_act_args(PipelineRetry, "99"), vec!["run", "rerun", "99"]);
        assert_eq!(gh_act_args(PipelineCancel, "99"), vec!["run", "cancel", "99"]);
    }

    #[test]
    fn comment_args_mapping() {
        assert_eq!(gh_comment_args("7", "hi"), vec!["issue", "comment", "7", "-b", "hi"]);
    }

    #[test]
    fn diff_args_mapping() {
        assert_eq!(gh_diff_args("12"), vec!["pr", "diff", "12"]);
    }

    #[test]
    fn comments_args_mapping() {
        assert_eq!(gh_comments_args("7"), vec!["api", "repos/{owner}/{repo}/issues/7/comments"]);
    }

    #[test]
    fn parses_tags() {
        let json = r#"[{"name":"v1.2.0","commit":{"sha":"abcdef1234567890"}}]"#;
        let rows = parse(Tab::Tags, json, now()).unwrap();
        assert_eq!(rows[0].id, "v1.2.0");
        assert_eq!(rows[0].cells, vec!["v1.2.0", "abcdef12", ""]);
    }

    #[test]
    fn parses_branches_and_commits() {
        let b = parse(Tab::Branches, r#"[{"name":"main","protected":true,"commit":{"sha":"deadbeef1234"}}]"#, now()).unwrap();
        assert_eq!(b[0].id, "main");
        assert_eq!(b[0].cells, vec!["main", "protected", "deadbeef"]);

        let c = parse(Tab::Commits, r#"[{"sha":"abc123def456","commit":{"message":"fix: bug\n\nbody","author":{"name":"bob","date":"2026-07-30T11:30:00Z"}}}]"#, now()).unwrap();
        assert_eq!(c[0].id, "abc123def456"); // full sha for the diff
        assert_eq!(c[0].cells, vec!["abc123de", "fix: bug", "bob", "30m ago"]);
    }

    #[test]
    fn parses_repo_list() {
        let json = r#"[{"nameWithOwner":"octo/hello"},{"nameWithOwner":"octo/world"}]"#;
        assert_eq!(parse_repos_gh(json), vec!["octo/hello", "octo/world"]);
        assert!(parse_repos_gh("not json").is_empty());
    }

    #[test]
    fn job_args_mapping() {
        assert_eq!(gh_jobs_args("5"), vec!["api", "repos/{owner}/{repo}/actions/runs/5/jobs"]);
        assert_eq!(gh_job_log_args("88"), vec!["run", "view", "--job", "88", "--log"]);
        assert_eq!(gh_job_retry_args("88"), vec!["run", "rerun", "--job", "88"]);
    }

    #[test]
    fn gh_job_cancel_is_unsupported() {
        assert!(GhBackend.job_act(crate::backend::JobAction::Cancel, "88").is_err());
    }
}
