use std::path::{Path, PathBuf};

// sun_path is 104 bytes on macOS; leave room for the longest socket name we create.
const MAX_DIR: usize = 104 - "/control.sock".len() - 32;

/// `$XDG_RUNTIME_DIR/shroud` where the system provides one (Linux, usually): it is private
/// to the user and cleared at logout. Otherwise `$TMPDIR/shroud-$UID` (macOS gives each user
/// their own TMPDIR), and `/tmp/shroud-$UID` when neither is set or a path is too long.
pub fn runtime_dir(xdg_runtime: Option<&str>, tmpdir: Option<&str>, uid: u32) -> PathBuf {
    let fits = |d: &PathBuf| d.as_os_str().len() <= MAX_DIR;
    // The spec says to ignore a relative path.
    if let Some(dir) = xdg_runtime.map(Path::new).filter(|x| x.is_absolute()).map(|x| x.join("shroud")).filter(fits) {
        return dir;
    }
    let dir = Path::new(tmpdir.filter(|t| !t.is_empty()).unwrap_or("/tmp")).join(format!("shroud-{uid}"));
    if fits(&dir) { dir } else { Path::new("/tmp").join(format!("shroud-{uid}")) }
}

/// One server holds every session, so there is one socket.
pub fn server_socket(dir: &Path) -> PathBuf {
    dir.join("server.sock")
}

pub fn control_socket(dir: &Path) -> PathBuf {
    dir.join("control.sock")
}

pub fn server_log(dir: &Path) -> PathBuf {
    dir.join("server.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_tmp() {
        assert_eq!(runtime_dir(None, None, 501), PathBuf::from("/tmp/shroud-501"));
        assert_eq!(runtime_dir(None, Some("/var/folders/x/T/"), 501), PathBuf::from("/var/folders/x/T/shroud-501"));
        assert_eq!(server_socket(Path::new("/d")), PathBuf::from("/d/server.sock"));
        let long = format!("/private/tmp/{}", "x".repeat(80));
        assert_eq!(runtime_dir(None, Some(&long), 501), PathBuf::from("/tmp/shroud-501"));
    }

    #[test]
    fn prefers_xdg_runtime_dir() {
        assert_eq!(runtime_dir(Some("/run/user/1000"), Some("/var/tmp"), 1000), PathBuf::from("/run/user/1000/shroud"));
        assert_eq!(runtime_dir(Some(""), None, 1000), PathBuf::from("/tmp/shroud-1000"));
        assert_eq!(runtime_dir(Some("relative"), None, 1000), PathBuf::from("/tmp/shroud-1000"));
        let long = format!("/run/{}", "x".repeat(80));
        assert_eq!(runtime_dir(Some(&long), Some("/var/tmp"), 1000), PathBuf::from("/var/tmp/shroud-1000"));
    }
}
