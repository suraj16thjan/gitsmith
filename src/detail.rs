use crate::backend::json::{join, nested_str, num, s};
use crate::backend::{Kind, Tab};
use crate::format::relative_time_str;
use chrono::{DateTime, Utc};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Comment {
    pub author: String,
    pub when: String,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct Detail {
    pub title: String,
    pub fields: Vec<(String, String)>,
    pub body: String,
    pub has_comments: bool,
}

impl Detail {
    /// Push a `(label, value)` field, skipping empty values so the pane stays tidy.
    fn push(&mut self, label: &str, value: String) {
        if !value.is_empty() {
            self.fields.push((label.to_string(), value));
        }
    }
}

/// Item id as shown in the title: glab uses `iid`, gh uses `number`.
fn item_id(kind: Kind, v: &Value) -> String {
    match kind {
        Kind::Glab => num(v, "iid"),
        Kind::Gh => num(v, "number"),
    }
}

/// The description/body text, under the key each platform uses.
fn body_of(kind: Kind, v: &Value) -> String {
    match kind {
        Kind::Glab => s(v, "description"),
        Kind::Gh => s(v, "body"),
    }
}

/// Build the instant part of a detail view (title + fields + body) from a row's
/// raw JSON. Never fails — missing fields collapse to empty and are omitted.
pub fn from_raw(kind: Kind, tab: Tab, v: &Value, now: DateTime<Utc>) -> Detail {
    let rel = |key: &str| relative_time_str(&s(v, key), now);
    let author = match kind {
        Kind::Glab => nested_str(v, "author", "username"),
        Kind::Gh => nested_str(v, "user", "login"),
    };

    match (kind, tab) {
        (_, Tab::Issues) | (_, Tab::MergeRequests) => {
            let mut d = Detail {
                title: format!("#{} · {}", item_id(kind, v), s(v, "title")),
                body: body_of(kind, v),
                has_comments: true,
                ..Default::default()
            };
            d.push("State", s(v, "state"));
            d.push("Author", author);
            let assignees = match kind {
                Kind::Glab => join(v, "assignees", Some("username")),
                Kind::Gh => join(v, "assignees", Some("login")),
            };
            d.push("Assignees", assignees);
            let labels = match kind {
                Kind::Glab => join(v, "labels", None),
                Kind::Gh => join(v, "labels", Some("name")),
            };
            d.push("Labels", labels);
            d.push("Milestone", nested_str(v, "milestone", "title"));
            if tab == Tab::MergeRequests {
                let (src, dst) = match kind {
                    Kind::Glab => (s(v, "source_branch"), s(v, "target_branch")),
                    Kind::Gh => (nested_str(v, "head", "ref"), nested_str(v, "base", "ref")),
                };
                if !src.is_empty() || !dst.is_empty() {
                    d.push("Branch", format!("{src} → {dst}"));
                }
            }
            d.push("Created", rel("created_at"));
            d.push("Updated", rel("updated_at"));
            d
        }
        (_, Tab::Pipelines) => {
            let mut d = Detail { title: format!("Pipeline #{}", num(v, "id")), ..Default::default() };
            d.push("Status", s(v, "status"));
            match kind {
                Kind::Glab => {
                    d.push("Ref", s(v, "ref"));
                    d.push("SHA", short_sha(&s(v, "sha")));
                    d.push("Source", s(v, "source"));
                }
                Kind::Gh => {
                    d.push("Conclusion", s(v, "conclusion"));
                    d.push("Ref", s(v, "head_branch"));
                    d.push("SHA", short_sha(&s(v, "head_sha")));
                    d.push("Event", s(v, "event"));
                }
            }
            d.push("Created", rel("created_at"));
            d.push("Updated", rel("updated_at"));
            d
        }
        (_, Tab::Runners) => {
            let mut d = Detail::default();
            match kind {
                Kind::Glab => {
                    d.title = format!("Runner #{} · {}", num(v, "id"), s(v, "description"));
                    d.push("Status", s(v, "status"));
                    d.push("Tags", join(v, "tag_list", None));
                    d.push("Version", s(v, "version"));
                    d.push("IP", s(v, "ip_address"));
                    d.push("Contacted", rel("contacted_at"));
                }
                Kind::Gh => {
                    d.title = format!("Runner #{} · {}", num(v, "id"), s(v, "name"));
                    d.push("Status", s(v, "status"));
                    d.push("OS", s(v, "os"));
                    d.push("Busy", v.get("busy").and_then(Value::as_bool).map(|b| b.to_string()).unwrap_or_default());
                    d.push("Labels", join(v, "labels", Some("name")));
                }
            }
            d
        }
        (_, Tab::Releases) => {
            let author = match kind {
                Kind::Glab => nested_str(v, "author", "username"),
                Kind::Gh => nested_str(v, "author", "login"),
            };
            let mut d = Detail {
                title: format!("{} · {}", s(v, "tag_name"), s(v, "name")),
                body: body_of(kind, v),
                ..Default::default()
            };
            d.push("Tag", s(v, "tag_name"));
            d.push("Author", author);
            d.push("Released", rel(if kind == Kind::Glab { "released_at" } else { "published_at" }));
            d
        }
        (_, Tab::Todos) => {
            let mut d = Detail::default();
            match kind {
                Kind::Glab => {
                    d.title = nested_str(v, "target", "title");
                    if d.title.is_empty() {
                        d.title = s(v, "body");
                    }
                    d.push("Project", nested_str(v, "project", "name"));
                    d.push("Action", s(v, "action_name"));
                    d.push("Author", nested_str(v, "author", "username"));
                    d.push("Target", s(v, "target_type"));
                    d.body = s(v, "body");
                }
                Kind::Gh => {
                    d.title = nested_str(v, "subject", "title");
                    d.push("Repo", nested_str(v, "repository", "full_name"));
                    d.push("Type", nested_str(v, "subject", "type"));
                    d.push("Reason", s(v, "reason"));
                }
            }
            d
        }
        (_, Tab::Tags) => {
            let mut d = Detail { title: s(v, "name"), ..Default::default() };
            match kind {
                Kind::Glab => {
                    d.push("Commit", nested_str(v, "commit", "short_id"));
                    d.push("Author", nested_str(v, "commit", "author_name"));
                    d.push("Date", relative_time_str(&nested_str(v, "commit", "created_at"), now));
                    let msg = s(v, "message");
                    d.body = if msg.is_empty() { nested_str(v, "commit", "title") } else { msg };
                }
                Kind::Gh => d.push("Commit", nested_str(v, "commit", "sha")),
            }
            d
        }
        (_, Tab::Branches) => {
            let mut d = Detail { title: s(v, "name"), ..Default::default() };
            let protected = v.get("protected").and_then(Value::as_bool) == Some(true);
            d.push("Protected", if protected { "yes".into() } else { "no".into() });
            if v.get("default").and_then(Value::as_bool) == Some(true) {
                d.push("Default", "yes".into());
            }
            let sha = match kind {
                Kind::Glab => nested_str(v, "commit", "short_id"),
                Kind::Gh => nested_str(v, "commit", "sha"),
            };
            d.push("Commit", sha);
            d
        }
        (_, Tab::Commits) => {
            // Enter on a commit opens its diff, not this pane; kept for exhaustiveness.
            let title = match kind {
                Kind::Glab => s(v, "short_id"),
                Kind::Gh => s(v, "sha"),
            };
            Detail { title, ..Default::default() }
        }
        (_, Tab::Milestones) => {
            let mut d = Detail {
                title: s(v, "title"),
                body: s(v, "description"),
                ..Default::default()
            };
            d.push("State", s(v, "state"));
            match kind {
                Kind::Glab => {
                    d.push("Start", s(v, "start_date"));
                    d.push("Due", s(v, "due_date"));
                }
                Kind::Gh => d.push("Due", s(v, "due_on")),
            }
            d
        }
    }
}

/// First 8 chars of a commit SHA (empty stays empty).
fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// Parse a comments/notes JSON array into `Comment`s. GitLab "system" notes
/// (auto-generated activity like "changed the milestone") are filtered out.
pub fn parse_comments(kind: Kind, json: &str, now: DateTime<Utc>) -> Vec<Comment> {
    let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(json) else {
        return vec![];
    };
    arr.iter()
        .filter_map(|c| {
            if kind == Kind::Glab && c.get("system").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let author = match kind {
                Kind::Glab => nested_str(c, "author", "username"),
                Kind::Gh => nested_str(c, "user", "login"),
            };
            Some(Comment { author, when: relative_time_str(&s(c, "created_at"), now), body: s(c, "body") })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 31, 12, 0, 0).unwrap()
    }

    fn field<'a>(d: &'a Detail, label: &str) -> Option<&'a str> {
        d.fields.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str())
    }

    #[test]
    fn glab_issue_detail() {
        let v = json!({
            "iid": 7, "title": "Bug", "state": "opened",
            "author": {"username": "alice"},
            "assignees": [{"username": "bob"}],
            "labels": ["ui", "p1"],
            "milestone": {"title": "v2"},
            "description": "It broke.",
            "created_at": "2026-07-31T09:00:00Z", "updated_at": "2026-07-31T11:00:00Z"
        });
        let d = from_raw(Kind::Glab, Tab::Issues, &v, now());
        assert_eq!(d.title, "#7 · Bug");
        assert_eq!(d.body, "It broke.");
        assert!(d.has_comments);
        assert_eq!(field(&d, "State"), Some("opened"));
        assert_eq!(field(&d, "Author"), Some("alice"));
        assert_eq!(field(&d, "Assignees"), Some("bob"));
        assert_eq!(field(&d, "Labels"), Some("ui, p1"));
        assert_eq!(field(&d, "Milestone"), Some("v2"));
    }

    #[test]
    fn gh_pr_detail_has_branch() {
        let v = json!({
            "number": 42, "title": "Add feature", "state": "open",
            "user": {"login": "bob"},
            "labels": [{"name": "enhancement"}],
            "head": {"ref": "feat"}, "base": {"ref": "main"},
            "body": "Adds a thing."
        });
        let d = from_raw(Kind::Gh, Tab::MergeRequests, &v, now());
        assert_eq!(d.title, "#42 · Add feature");
        assert_eq!(d.body, "Adds a thing.");
        assert!(d.has_comments);
        assert_eq!(field(&d, "Author"), Some("bob"));
        assert_eq!(field(&d, "Labels"), Some("enhancement"));
        assert_eq!(field(&d, "Branch"), Some("feat → main"));
    }

    #[test]
    fn glab_pipeline_detail() {
        let v = json!({ "id": 99, "status": "success", "ref": "main", "sha": "abcdef1234567890", "source": "push" });
        let d = from_raw(Kind::Glab, Tab::Pipelines, &v, now());
        assert_eq!(d.title, "Pipeline #99");
        assert_eq!(field(&d, "Status"), Some("success"));
        assert_eq!(field(&d, "Ref"), Some("main"));
        assert_eq!(field(&d, "SHA"), Some("abcdef12"));
        assert!(!d.has_comments);
    }

    #[test]
    fn gh_release_detail() {
        let v = json!({ "tag_name": "v1.0", "name": "First", "author": {"login": "bob"}, "published_at": "2026-07-31T09:00:00Z", "body": "Notes." });
        let d = from_raw(Kind::Gh, Tab::Releases, &v, now());
        assert_eq!(d.title, "v1.0 · First");
        assert_eq!(d.body, "Notes.");
        assert_eq!(field(&d, "Author"), Some("bob"));
        assert_eq!(field(&d, "Tag"), Some("v1.0"));
    }

    #[test]
    fn glab_tag_detail() {
        let v = json!({ "name": "v2.0", "message": "release notes", "commit": {"short_id": "abc123", "author_name": "alice"} });
        let d = from_raw(Kind::Glab, Tab::Tags, &v, now());
        assert_eq!(d.title, "v2.0");
        assert_eq!(d.body, "release notes");
        assert_eq!(field(&d, "Commit"), Some("abc123"));
        assert_eq!(field(&d, "Author"), Some("alice"));
        assert!(!d.has_comments);
    }

    #[test]
    fn glab_milestone_detail() {
        let v = json!({ "title": "Sprint 3", "state": "active", "due_date": "2026-08-15", "description": "goals" });
        let d = from_raw(Kind::Glab, Tab::Milestones, &v, now());
        assert_eq!(d.title, "Sprint 3");
        assert_eq!(d.body, "goals");
        assert_eq!(field(&d, "Due"), Some("2026-08-15"));
    }

    #[test]
    fn parse_comments_glab_skips_system_notes() {
        let json = r#"[
          {"author":{"username":"alice"},"created_at":"2026-07-31T09:00:00Z","body":"real comment","system":false},
          {"author":{"username":"bot"},"created_at":"2026-07-31T10:00:00Z","body":"changed milestone","system":true}
        ]"#;
        let cs = parse_comments(Kind::Glab, json, now());
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].author, "alice");
        assert_eq!(cs[0].body, "real comment");
    }

    #[test]
    fn parse_comments_gh_and_junk() {
        let json = r#"[{"user":{"login":"bob"},"created_at":"2026-07-31T11:30:00Z","body":"hi"}]"#;
        let cs = parse_comments(Kind::Gh, json, now());
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].author, "bob");
        assert!(parse_comments(Kind::Gh, "not json", now()).is_empty());
        assert!(parse_comments(Kind::Gh, "{}", now()).is_empty());
    }

    #[test]
    fn empty_object_degrades_without_panic() {
        let d = from_raw(Kind::Glab, Tab::Issues, &json!({}), now());
        assert_eq!(d.title, "# · "); // ids/title empty
        assert!(d.fields.is_empty());
        assert!(d.body.is_empty());
        assert!(d.has_comments); // still an issue
    }
}
