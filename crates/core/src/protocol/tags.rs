//! Limits keep a buggy plugin from bloating `list` and every saved session.

use std::collections::BTreeMap;

pub const MAX_TAGS: usize = 32;
pub const MAX_VALUE: usize = 1024;

pub type Tags = BTreeMap<String, String>;

/// Keys follow command names, so `myplugin.state` reads as whose it is.
pub fn check_key(key: &str) -> Result<(), String> {
    let ok = !key.is_empty()
        && key.len() <= 64
        && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c));
    if ok { Ok(()) } else { Err(format!("`{key}` isn't a tag name: up to 64 lowercase letters, digits, -, _ and .")) }
}

pub fn set(tags: &mut Tags, key: &str, value: Option<String>) -> Result<(), String> {
    check_key(key)?;
    match value {
        None => {
            tags.remove(key);
        }
        Some(v) if v.len() > MAX_VALUE => return Err(format!("a tag's value is at most {MAX_VALUE} bytes")),
        Some(_) if !tags.contains_key(key) && tags.len() >= MAX_TAGS => {
            return Err(format!("a pane holds at most {MAX_TAGS} tags"));
        }
        Some(v) => {
            tags.insert(key.to_string(), v);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_replace_remove_and_limits() {
        let mut t = Tags::new();
        set(&mut t, "git.branch", Some("main".into())).unwrap();
        set(&mut t, "git.branch", Some("dev".into())).unwrap();
        assert_eq!(t.get("git.branch").map(String::as_str), Some("dev"));
        set(&mut t, "git.branch", None).unwrap();
        assert!(t.is_empty());
        assert!(set(&mut t, "Bad Key", Some("x".into())).is_err());
        assert!(set(&mut t, "k", Some("x".repeat(MAX_VALUE + 1))).is_err());
        for i in 0..MAX_TAGS {
            set(&mut t, &format!("k{i}"), Some(String::new())).unwrap();
        }
        assert!(set(&mut t, "one-more", Some(String::new())).is_err());
        assert!(set(&mut t, "k0", Some("replacing is fine".into())).is_ok());
    }
}
