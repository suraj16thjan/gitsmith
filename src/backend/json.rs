use serde_json::Value;

pub fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

pub fn num(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

pub fn nested_str(v: &Value, key: &str, sub: &str) -> String {
    v.get(key).and_then(|o| o.get(sub)).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Join an array field into a ", "-separated string. Elements are either plain
/// strings (`sub = None`) or objects from which `sub` is read (e.g. labels as
/// `[{ "name": .. }]`). Missing field or empty elements collapse away.
pub fn join(v: &Value, key: &str, sub: Option<&str>) -> String {
    let Some(arr) = v.get(key).and_then(Value::as_array) else {
        return String::new();
    };
    arr.iter()
        .map(|e| match sub {
            Some(k) => e.get(k).and_then(Value::as_str).unwrap_or("").to_string(),
            None => e.as_str().unwrap_or("").to_string(),
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn join_strings_and_objects() {
        let v = json!({ "labels": ["bug", "ui"], "assignees": [{"login": "a"}, {"login": "b"}] });
        assert_eq!(join(&v, "labels", None), "bug, ui");
        assert_eq!(join(&v, "assignees", Some("login")), "a, b");
        assert_eq!(join(&v, "missing", None), "");
    }
}
