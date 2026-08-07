use crate::backend::{Action, Backend, JobAction, Kind, NewMr, Row, Tab};
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub enum Msg {
    Fetched { tab: Tab, page: u32, result: Result<Vec<Row>, String> },
    Acted { tab: Tab, result: Result<(), String> },
    Diff { result: Result<String, String> },
    Comments { result: Result<String, String> },
    Jobs { result: Result<String, String> },
    JobLog { result: Result<String, String> },
    JobActed { result: Result<(), String> },
    Repos { result: Result<Vec<String>, String> },
    /// Hosts the chosen CLI is configured for, and where that list came from.
    Hosts { kind: Kind, hosts: Vec<String>, origin: String },
    /// A newly opened MR/PR or issue: the CLI's output (its URL) or the failure.
    Created { result: Result<String, String> },
    /// Branch names for the new-MR form.
    Branches { result: Result<Vec<String>, String> },
}

pub fn spawn(backend: Arc<dyn Backend>, tab: Tab, page: u32, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.list(tab, page).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Fetched { tab, page, result });
    });
}

/// Branch names for the new-MR form's source/target dropdowns.
pub fn spawn_branches(backend: Arc<dyn Backend>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend
            .list(Tab::Branches, 1)
            .map(|rows| rows.into_iter().map(|r| r.id).filter(|b| !b.is_empty()).collect())
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Branches { result });
    });
}

pub fn spawn_add_member(backend: Arc<dyn Backend>, user: String, role: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.add_member(&user, &role).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Created { result });
    });
}

pub fn spawn_create_issue(backend: Arc<dyn Backend>, title: String, body: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.create_issue(&title, &body).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Created { result });
    });
}

pub fn spawn_act(backend: Arc<dyn Backend>, tab: Tab, action: Action, id: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.act(action, &id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Acted { tab, result });
    });
}

pub fn spawn_diff(backend: Arc<dyn Backend>, tab: Tab, id: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.diff(tab, &id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Diff { result });
    });
}

pub fn spawn_commit_diff(backend: Arc<dyn Backend>, sha: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.commit_diff(&sha).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Diff { result });
    });
}

pub fn spawn_comments(backend: Arc<dyn Backend>, tab: Tab, id: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.comments(tab, &id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Comments { result });
    });
}

pub fn spawn_jobs(backend: Arc<dyn Backend>, pipeline_id: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.jobs(&pipeline_id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Jobs { result });
    });
}

pub fn spawn_job_log(backend: Arc<dyn Backend>, job_id: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.job_log(&job_id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::JobLog { result });
    });
}

pub fn spawn_job_act(backend: Arc<dyn Backend>, action: JobAction, job_id: String, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.job_act(action, &job_id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::JobActed { result });
    });
}

pub fn spawn_create_mr(backend: Arc<dyn Backend>, mr: NewMr, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.create_mr(&mr).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Created { result });
    });
}

/// `auth status` shells out and talks to the host, so it runs off the UI thread.
pub fn spawn_hosts(kind: Kind, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let found = crate::backend::auth_hosts(kind);
        let _ = tx.send(Msg::Hosts { kind, hosts: found.hosts, origin: found.origin });
    });
}

pub fn spawn_repos(backend: Arc<dyn Backend>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let result = backend.repos().map_err(|e| format!("{e:#}"));
        let _ = tx.send(Msg::Repos { result });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, Row, Tab};
    use std::sync::mpsc;

    struct FakeOk;
    impl Backend for FakeOk {
        fn list(&self, _tab: Tab, _page: u32) -> anyhow::Result<Vec<Row>> {
            Ok(vec![Row { id: String::new(), cells: vec!["x".into()], web_url: String::new(), raw: serde_json::Value::Null }])
        }

        fn act(&self, _action: Action, _id: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn diff(&self, _t: Tab, _id: &str) -> anyhow::Result<String> {
            Ok("diff --git a/x b/x\n".into())
        }

        fn comments(&self, _t: Tab, _id: &str) -> anyhow::Result<String> {
            Ok("[]".into())
        }

        fn jobs(&self, _pid: &str) -> anyhow::Result<String> {
            Ok("[]".into())
        }

        fn job_log(&self, _jid: &str) -> anyhow::Result<String> {
            Ok("log line\n".into())
        }

        fn job_act(&self, _a: JobAction, _jid: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn repos(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec!["o/a".into(), "o/b".into()])
        }
    }

    struct FakeErr;
    impl Backend for FakeErr {
        fn list(&self, _tab: Tab, _page: u32) -> anyhow::Result<Vec<Row>> {
            anyhow::bail!("boom")
        }

        fn diff(&self, _t: Tab, _id: &str) -> anyhow::Result<String> {
            anyhow::bail!("diff error")
        }
    }

    #[test]
    fn spawn_delivers_fetched_ok() {
        let (tx, rx) = mpsc::channel();
        spawn(Arc::new(FakeOk), Tab::Issues, 1, tx);
        match rx.recv().unwrap() {
            Msg::Fetched { tab, page, result } => {
                assert_eq!(page, 1);
                assert_eq!(tab, Tab::Issues);
                assert_eq!(result.unwrap().len(), 1);
            }
            _ => panic!("expected Fetched"),
        }
    }

    #[test]
    fn spawn_delivers_err_as_string() {
        let (tx, rx) = mpsc::channel();
        spawn(Arc::new(FakeErr), Tab::Runners, 1, tx);
        match rx.recv().unwrap() {
            Msg::Fetched { result, .. } => assert_eq!(result.unwrap_err(), "boom"),
            _ => panic!("expected Fetched"),
        }
    }

    #[test]
    fn spawn_act_delivers_acted() {
        let (tx, rx) = mpsc::channel();
        spawn_act(Arc::new(FakeOk), Tab::MergeRequests, crate::backend::Action::MrApprove, "1".into(), tx);
        match rx.recv().unwrap() {
            Msg::Acted { tab, result } => {
                assert_eq!(tab, Tab::MergeRequests);
                assert!(result.is_ok());
            }
            _ => panic!("expected Acted"),
        }
    }

    #[test]
    fn spawn_diff_delivers_diff() {
        let (tx, rx) = mpsc::channel();
        spawn_diff(Arc::new(FakeOk), Tab::MergeRequests, "1".into(), tx);
        match rx.recv().unwrap() {
            Msg::Diff { result } => assert!(result.is_ok()),
            _ => panic!("expected Diff"),
        }
    }

    #[test]
    fn spawn_comments_delivers_comments() {
        let (tx, rx) = mpsc::channel();
        spawn_comments(Arc::new(FakeOk), Tab::Issues, "7".into(), tx);
        match rx.recv().unwrap() {
            Msg::Comments { result } => assert_eq!(result.unwrap(), "[]"),
            _ => panic!("expected Comments"),
        }
    }

    #[test]
    fn spawn_repos_delivers_list() {
        let (tx, rx) = mpsc::channel();
        spawn_repos(Arc::new(FakeOk), tx);
        match rx.recv().unwrap() {
            Msg::Repos { result } => assert_eq!(result.unwrap().len(), 2),
            _ => panic!("expected Repos"),
        }
    }

    #[test]
    fn spawn_jobs_and_log_and_act_deliver() {
        let (tx, rx) = mpsc::channel();
        spawn_jobs(Arc::new(FakeOk), "5".into(), tx.clone());
        assert!(matches!(rx.recv().unwrap(), Msg::Jobs { result } if result.is_ok()));
        spawn_job_log(Arc::new(FakeOk), "88".into(), tx.clone());
        assert!(matches!(rx.recv().unwrap(), Msg::JobLog { result } if result.as_deref() == Ok("log line\n")));
        spawn_job_act(Arc::new(FakeOk), JobAction::Retry, "88".into(), tx);
        assert!(matches!(rx.recv().unwrap(), Msg::JobActed { result } if result.is_ok()));
    }
}
