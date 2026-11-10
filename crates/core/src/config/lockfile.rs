//! `plugins.lock`: the exact commit of each git plugin, so a config checked into dotfiles
//! installs the same code everywhere. Written by `plugin sync`, meant to be committed.

use toml::de::{DeTable, DeValue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locked {
    pub src: String,
    pub version: Option<String>,
    pub commit: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lock {
    pub plugins: Vec<Locked>,
}

impl Lock {
    pub fn get(&self, src: &str) -> Option<&Locked> {
        self.plugins.iter().find(|l| l.src == src)
    }

    /// A damaged lockfile only costs the pins in it; the next sync rewrites it.
    pub fn parse(text: &str) -> Result<Lock, String> {
        let doc = DeTable::parse(text).map_err(|e| e.message().to_string())?;
        let mut lock = Lock::default();
        let Some(list) = doc.get_ref().get("plugin") else { return Ok(lock) };
        let list = list.get_ref().as_array().ok_or("`plugin` must be a list of [[plugin]] tables")?;
        for entry in list {
            let t = entry.get_ref().as_table().ok_or("each [[plugin]] must be a table")?;
            let get = |k: &str| {
                t.get(k).and_then(|v| match v.get_ref() {
                    DeValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
            };
            let (Some(src), Some(commit)) = (get("src"), get("commit")) else {
                return Err("each [[plugin]] needs `src` and `commit` strings".into());
            };
            if !commit.bytes().all(|b| b.is_ascii_hexdigit()) || !(7..=64).contains(&commit.len()) {
                return Err(format!("`{commit}` isn't a commit hash"));
            }
            lock.plugins.push(Locked { src, version: get("version"), commit });
        }
        Ok(lock)
    }

    pub fn write(&self) -> String {
        let mut out = String::from("# Written by `shroud plugin sync`. Commit it beside config.toml.\n");
        for l in &self.plugins {
            out.push_str(&format!("\n[[plugin]]\nsrc = {}\n", quote(&l.src)));
            if let Some(v) = &l.version {
                out.push_str(&format!("version = {}\n", quote(v)));
            }
            out.push_str(&format!("commit = {}\n", quote(&l.commit)));
        }
        out
    }
}

// JSON's string escapes are a subset of TOML's basic-string escapes.
fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialize")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_including_awkward_strings() {
        let lock = Lock {
            plugins: vec![
                Locked {
                    src: "github:a/b".into(),
                    version: Some("v1.2.0".into()),
                    commit: "0123456789abcdef0123456789abcdef01234567".into(),
                },
                Locked { src: "git:/tmp/we\"ird\\path\u{1}".into(), version: None, commit: "abcdef0".into() },
            ],
        };
        assert_eq!(Lock::parse(&lock.write()), Ok(lock.clone()));
        assert_eq!(lock.get("github:a/b").map(|l| l.commit.len()), Some(40));
    }

    #[test]
    fn a_bad_lockfile_says_why() {
        assert_eq!(Lock::parse(""), Ok(Lock::default()));
        assert!(Lock::parse("[[plugin]]\nsrc = \"x\"\n").unwrap_err().contains("commit"));
        assert!(
            Lock::parse("[[plugin]]\nsrc = \"x\"\ncommit = \"--upload-pack=evil\"\n")
                .unwrap_err()
                .contains("commit hash")
        );
        assert!(Lock::parse("plugin = 3\n").is_err());
        assert!(Lock::parse("[[plugin").is_err());
    }
}
