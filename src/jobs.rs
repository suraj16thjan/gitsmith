use crate::backend::json::{num, s};
use crate::backend::Kind;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub stage: String,
    pub status: String,
    /// For a bridge/trigger job: the downstream pipeline id it points to. Opening
    /// such a job drills into that pipeline's jobs rather than fetching a log.
    pub downstream: Option<String>,
}

/// Parse a pipeline's jobs JSON. glab returns a bare array; gh wraps it in
/// `{ "jobs": [...] }` and splits state across `status`/`conclusion` (folded
/// here to one word). Junk/empty input yields no jobs.
pub fn parse(kind: Kind, json: &str) -> Vec<Job> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return vec![];
    };
    let arr = match kind {
        Kind::Glab => v.as_array().cloned().unwrap_or_default(),
        Kind::Gh => v.get("jobs").and_then(Value::as_array).cloned().unwrap_or_default(),
    };
    arr.iter()
        .map(|j| match kind {
            Kind::Glab => Job {
                id: num(j, "id"),
                name: s(j, "name"),
                stage: s(j, "stage"),
                status: s(j, "status"),
                downstream: j
                    .get("downstream_pipeline")
                    .filter(|d| !d.is_null())
                    .map(|d| num(d, "id"))
                    .filter(|s| !s.is_empty()),
            },
            Kind::Gh => {
                // completed jobs carry the outcome in `conclusion`; running ones in `status`.
                let concl = s(j, "conclusion");
                let status = if concl.is_empty() { s(j, "status") } else { concl };
                Job { id: num(j, "id"), name: s(j, "name"), stage: String::new(), status, downstream: None }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glab_jobs() {
        let json = r#"[
          {"id":11,"name":"build","stage":"build","status":"success"},
          {"id":12,"name":"test","stage":"test","status":"running"}
        ]"#;
        let jobs = parse(Kind::Glab, json);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, "11");
        assert_eq!(jobs[0].name, "build");
        assert_eq!(jobs[0].stage, "build");
        assert_eq!(jobs[1].status, "running");
    }

    #[test]
    fn gh_jobs_fold_conclusion() {
        let json = r#"{"jobs":[
          {"id":21,"name":"lint","status":"completed","conclusion":"success"},
          {"id":22,"name":"deploy","status":"in_progress","conclusion":null}
        ]}"#;
        let jobs = parse(Kind::Gh, json);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, "21");
        assert_eq!(jobs[0].status, "success"); // folded from conclusion
        assert_eq!(jobs[1].status, "in_progress"); // no conclusion yet
        assert_eq!(jobs[0].stage, ""); // gh has no stages
    }

    #[test]
    fn empty_and_junk() {
        assert!(parse(Kind::Glab, "").is_empty());
        assert!(parse(Kind::Gh, "not json").is_empty());
        assert!(parse(Kind::Gh, "{}").is_empty());
    }
}
