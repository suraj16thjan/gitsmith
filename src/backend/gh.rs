use crate::backend::json::{join, nested_str, num, s};
use crate::backend::{Backend, NewMr, Row, Tab};
use crate::format::relative_time_str;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

pub struct GhBackend;

fn to_row(tab: Tab, v: &Value, now: DateTime<Utc>) -> Row {
    let updated = relative_time_str(&s(v, "updated_at"), now);
    let cells = match tab {
        Tab::Issues => vec![
            num(v, "number"),
            s(v, "title"),
            join(v, "labels", Some("name")), // GitHub labels are objects
            s(v, "state"),
            nested_str(v, "user", "login"),
            updated,
        ],
        Tab::MergeRequests => {
            let src = nested_str(v, "head", "ref");
            let dst = nested_str(v, "base", "ref");
            vec![
                num(v, "number"),
                s(v, "title"),
                if src.is_empty() && dst.is_empty() { String::new() } else { format!("{src} → {dst}") },
                join(v, "labels", Some("name")),
                s(v, "state"),
                nested_str(v, "user", "login"),
                updated,
            ]
        }
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
            // GitHub's branch list carries only name/sha/protected — no author.
            // Left blank rather than costing one API call per branch.
            String::new(),
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
        Tab::Members => vec![s(v, "login"), String::new(), gh_role(v), s(v, "type")],
    };
    let id = match tab {
        Tab::Issues | Tab::MergeRequests => num(v, "number"),
        Tab::Pipelines | Tab::Runners | Tab::Todos => num(v, "id"),
        Tab::Releases => s(v, "tag_name"),
        Tab::Tags | Tab::Branches => s(v, "name"),
        Tab::Commits => s(v, "sha"),
        Tab::Milestones => num(v, "number"),
        Tab::Members => s(v, "login"),
    };
    Row { id, cells, web_url: s(v, "html_url"), raw: v.clone() }
}

/// A collaborator's role: `role_name` when the API sends it, else the strongest
/// permission flag it did send.
fn gh_role(v: &Value) -> String {
    let role = s(v, "role_name");
    if !role.is_empty() {
        return role;
    }
    let perm = |k: &str| v.get("permissions").and_then(|p| p.get(k)).and_then(Value::as_bool) == Some(true);
    for (flag, name) in [("admin", "admin"), ("maintain", "maintain"), ("push", "write"), ("triage", "triage")] {
        if perm(flag) {
            return name.to_string();
        }
    }
    "read".to_string()
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
    let mut rows: Vec<Row> = arr
        .iter()
        // GitHub's issues endpoint also returns pull requests; they have their
        // own tab, and listing them twice makes the numbering look wrong.
        .filter(|v| tab != Tab::Issues || v.get("pull_request").is_none())
        .map(|v| to_row(tab, v, now))
        .collect();
    crate::backend::sort_rows(tab, &mut rows);
    Ok(rows)
}

fn gh_path(tab: Tab) -> &'static str {
    // ponytail: relies on gh's current-repo detection; add owner/repo override later.
    match tab {
        // Both endpoints default to open only; closed items are the ones you'd
        // want to reopen, so ask for every state.
        Tab::Issues => "repos/{owner}/{repo}/issues?state=all",
        Tab::MergeRequests => "repos/{owner}/{repo}/pulls?state=all",
        Tab::Pipelines => "repos/{owner}/{repo}/actions/runs",
        Tab::Runners => "repos/{owner}/{repo}/actions/runners",
        Tab::Releases => "repos/{owner}/{repo}/releases",
        Tab::Tags => "repos/{owner}/{repo}/tags",
        Tab::Branches => "repos/{owner}/{repo}/branches",
        Tab::Commits => "repos/{owner}/{repo}/commits",
        Tab::Todos => "notifications",
        Tab::Milestones => "repos/{owner}/{repo}/milestones",
        Tab::Members => "repos/{owner}/{repo}/collaborators",
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
    // `gh run view --job --log` refuses to print a log while its run is still in
    // progress; the jobs logs endpoint returns partial logs for a running job.
    vec![
        "api".into(),
        format!("repos/{{owner}}/{{repo}}/actions/jobs/{job_id}/logs"),
        "--allow-escape-sequences".into(),
    ]
}

/// `gh pr create` args. Passing both title and body keeps gh non-interactive.
fn gh_create_args(mr: &NewMr) -> Vec<String> {
    let mut args = owned(&["pr", "create", "--title", &mr.title, "--body", &mr.description]);
    if !mr.source.is_empty() {
        args.push("--head".into());
        args.push(mr.source.clone());
    }
    if !mr.target.is_empty() {
        args.push("--base".into());
        args.push(mr.target.clone());
    }
    args
}

/// `gh issue create` args. Title and body together keep gh non-interactive.
fn gh_issue_create_args(title: &str, description: &str) -> Vec<String> {
    owned(&["issue", "create", "--title", title, "--body", description])
}

fn gh_job_retry_args(job_id: &str) -> Vec<String> {
    owned(&["run", "rerun", "--job", job_id])
}

/// Add paging query params to a REST path — every endpoint gitsmith lists is a
/// paginated collection.
fn paged_path(path: &str, page: u32) -> String {
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}per_page={}&page={page}", crate::backend::PER_PAGE)
}

impl Backend for GhBackend {
    fn list(&self, tab: Tab, page: u32) -> Result<Vec<Row>> {
        let out = crate::backend::run("gh", &["api", &paged_path(gh_path(tab), page)])?;
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
        // The repo list is account-wide, so it must NOT be scoped to a repo.
        let out = crate::backend::run("gh", &["repo", "list", "--limit", "200", "--json", "nameWithOwner"])?;
        Ok(parse_repos_gh(&out))
    }

    fn create_mr(&self, mr: &NewMr) -> Result<String> {
        // `gh pr create` works without a checkout (unlike glab's), but with no
        // --head and no current branch it would fall back to prompting, which
        // would hang the TUI. Resolve the branch here or say why we can't.
        let source = if mr.source.is_empty() {
            crate::backend::current_branch().context(
                "no source branch — fill in the Source branch field (there's no checkout here)",
            )?
        } else {
            mr.source.clone()
        };
        let args = gh_create_args(&NewMr { source, ..mr.clone() });
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = crate::backend::run("gh", &argv)?;
        Ok(crate::backend::created_url(&out))
    }

    fn add_member(&self, user: &str, role: &str) -> Result<String> {
        // GitHub invites by username directly; the role is its permission name.
        let path = format!("repos/{{owner}}/{{repo}}/collaborators/{user}");
        let args = owned(&["api", "--method", "PUT", &path, "-f", &format!("permission={role}")]);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)?;
        Ok(format!("{user} invited as {role}"))
    }

    fn create_issue(&self, title: &str, description: &str) -> Result<String> {
        let args = gh_issue_create_args(title, description);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = crate::backend::run("gh", &argv)?;
        Ok(crate::backend::created_url(&out))
    }

    fn create_pipeline(&self, _branch: &str) -> Result<String> {
        // GitHub Actions runs are triggered by a workflow_dispatch (which needs a
        // specific workflow), not by naming a branch — there's no "run this ref".
        anyhow::bail!("GitHub Actions runs are triggered by workflow_dispatch, not created from a branch");
    }

    fn create_tag(&self, branch: &str, name: &str) -> Result<String> {
        // An empty branch means the project's default branch.
        let branch = if branch.is_empty() {
            let out = crate::backend::run("gh", &["api", "repos/{owner}/{repo}"])?;
            serde_json::from_str::<Value>(&out)
                .ok()
                .and_then(|v| v.get("default_branch").and_then(Value::as_str).map(str::to_string))
                .filter(|s| !s.is_empty())
                .context("could not determine the default branch")?
        } else {
            branch.to_string()
        };
        // A tag is a ref pointing at the branch's commit; look the SHA up first.
        // Slashes in a branch name (`feature/x`) are path separators, so encode them.
        let enc = branch.replace('/', "%2F");
        let heads = format!("repos/{{owner}}/{{repo}}/branches/{enc}");
        let out = crate::backend::run("gh", &["api", &heads])?;
        let sha = serde_json::from_str::<Value>(&out)
            .ok()
            .and_then(|v| v.get("commit").and_then(|c| c.get("sha")).and_then(Value::as_str).map(str::to_string))
            .filter(|s| !s.is_empty())
            .context("could not read the branch's commit SHA")?;
        let path = "repos/{owner}/{repo}/git/refs";
        let args = owned(&[
            "api",
            "--method",
            "POST",
            path,
            "-f",
            &format!("ref=refs/tags/{name}"),
            "-f",
            &format!("sha={sha}"),
        ]);
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        crate::backend::run("gh", &argv)?;
        Ok(name.to_string())
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
    fn parses_collaborators_with_their_role() {
        // role_name when the API sends it…
        let json = r#"[{"login":"alice","role_name":"admin","type":"User"}]"#;
        let rows = parse(Tab::Members, json, now()).unwrap();
        assert_eq!(rows[0].id, "alice");
        assert_eq!(rows[0].cells, vec!["alice", "", "admin", "User"]);

        // …else the strongest permission flag it did send
        let json = r#"[{"login":"bob","permissions":{"push":true,"pull":true},"type":"User"}]"#;
        let rows = parse(Tab::Members, json, now()).unwrap();
        assert_eq!(rows[0].cells[2], "write");

        let json = r#"[{"login":"carol","permissions":{"pull":true},"type":"User"}]"#;
        let rows = parse(Tab::Members, json, now()).unwrap();
        assert_eq!(rows[0].cells[2], "read");
    }

    #[test]
    fn pr_create_never_relies_on_a_prompt() {
        // both title and body are always passed, so gh can't stop to ask
        let bare = NewMr { title: "T".into(), ..Default::default() };
        let args = gh_create_args(&bare);
        assert!(args.contains(&"--title".to_string()) && args.contains(&"--body".to_string()));
        assert_eq!(gh_issue_create_args("T", ""), vec!["issue", "create", "--title", "T", "--body", ""]);
        // a resolved head branch is passed explicitly rather than inferred
        let with_head = NewMr { source: "feat/x".into(), title: "T".into(), ..Default::default() };
        let args = gh_create_args(&with_head);
        assert_eq!(args.iter().position(|a| a == "--head").map(|i| &args[i + 1]), Some(&"feat/x".to_string()));
    }

    #[test]
    fn paging_adds_query_params() {
        assert_eq!(paged_path("repos/o/r/branches", 2), "repos/o/r/branches?per_page=100&page=2");
        assert_eq!(paged_path("notifications?all=true", 1), "notifications?all=true&per_page=100&page=1");
    }

    #[test]
    fn issue_create_args_are_non_interactive() {
        assert_eq!(
            gh_issue_create_args("Broken", "steps to repro"),
            vec!["issue", "create", "--title", "Broken", "--body", "steps to repro"]
        );
    }

    #[test]
    fn create_args_are_non_interactive() {
        let mr = NewMr {
            source: "feature/x".into(),
            target: "main".into(),
            title: "Add x".into(),
            description: "why x".into(),
        };
        assert_eq!(
            gh_create_args(&mr),
            vec!["pr", "create", "--title", "Add x", "--body", "why x", "--head", "feature/x", "--base", "main"]
        );
        let bare = NewMr { title: "T".into(), ..Default::default() };
        assert_eq!(gh_create_args(&bare), vec!["pr", "create", "--title", "T", "--body", ""]);
    }

    #[test]
    fn pull_requests_are_kept_out_of_the_issues_tab() {
        // GitHub's issues endpoint returns PRs too — they have their own tab
        let json = r#"[
          {"number":5,"title":"a real issue","state":"open","updated_at":"2026-07-30T09:00:00Z"},
          {"number":6,"title":"actually a PR","state":"open","updated_at":"2026-07-31T09:00:00Z",
           "pull_request":{"url":"https://api.github.com/repos/o/r/pulls/6"}}
        ]"#;
        let rows = parse(Tab::Issues, json, now()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "5");
        // …but the MRs/PRs tab still lists everything it's given
        let rows = parse(Tab::MergeRequests, json, now()).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn every_state_is_requested() {
        // closed items must come back, or there's nothing to reopen
        assert!(gh_path(Tab::Issues).contains("state=all"));
        assert!(gh_path(Tab::MergeRequests).contains("state=all"));
        // and paging still appends correctly onto that query
        assert_eq!(
            paged_path(gh_path(Tab::Issues), 2),
            "repos/{owner}/{repo}/issues?state=all&per_page=100&page=2"
        );
    }

    #[test]
    fn parse_orders_rows_newest_first() {
        let json = r#"[
          {"number":1,"title":"old","updated_at":"2026-07-01T09:00:00Z"},
          {"number":3,"title":"newest","updated_at":"2026-07-30T09:00:00Z"},
          {"number":2,"title":"middle","updated_at":"2026-07-20T09:00:00Z"}
        ]"#;
        let rows = parse(Tab::Issues, json, now()).unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["3", "2", "1"]);
    }

    #[test]
    fn parses_pulls() {
        let json = r#"[
          {"number":42,"title":"Add feature","state":"open",
           "labels":[{"name":"enhancement"},{"name":"p1"}],
           "user":{"login":"bob"},"updated_at":"2026-07-30T11:30:00Z",
           "head":{"ref":"dev"},"base":{"ref":"main"},
           "html_url":"https://github.com/o/r/pull/42"}
        ]"#;
        let rows = parse(Tab::MergeRequests, json, now()).unwrap();
        assert_eq!(rows[0].id, "42");
        assert_eq!(rows[0].cells, vec!["42", "Add feature", "dev → main", "enhancement, p1", "open", "bob", "30m ago"]);
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
        // GitHub's branch list has no author, so the owner column stays blank
        assert_eq!(b[0].cells, vec!["main", "", "protected", "deadbeef"]);

        let c = parse(Tab::Commits, r#"[{"sha":"abc123def456","commit":{"message":"fix: bug\n\nbody","author":{"name":"bob","date":"2026-07-30T11:30:00Z"}}}]"#, now()).unwrap();
        assert_eq!(c[0].id, "abc123def456"); // full sha for the diff
        assert_eq!(c[0].cells, vec!["abc123de", "fix: bug", "bob", "30m ago"]);
    }

    #[test]
    fn parses_repo_list() {
        let json = r#"[{"nameWithOwner":"octo/hello"},{"nameWithOwner":"octo/world"}]"#;
        assert_eq!(parse_repos_gh(json), vec!["octo/hello", "octo/world"]);
    }

    #[test]
    fn job_args_mapping() {
        assert_eq!(gh_jobs_args("5"), vec!["api", "repos/{owner}/{repo}/actions/runs/5/jobs"]);
        assert_eq!(gh_job_log_args("88"), vec!["api", "repos/{owner}/{repo}/actions/jobs/88/logs", "--allow-escape-sequences"]);
        assert_eq!(gh_job_retry_args("88"), vec!["run", "rerun", "--job", "88"]);
    }

    #[test]
    fn gh_job_cancel_is_unsupported() {
        assert!(GhBackend.job_act(crate::backend::JobAction::Cancel, "88").is_err());
    }
}
