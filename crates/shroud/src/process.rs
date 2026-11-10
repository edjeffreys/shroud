//! Best effort: the process can exit or exec between the calls, and then there's no answer.

use std::os::fd::AsFd;

/// The foreground program's command line, quoted for a shell, unless it's `shell` itself.
pub fn foreground(pty: impl AsFd, shell: u32) -> Option<String> {
    let pgid = rustix::termios::tcgetpgrp(pty).ok()?.as_raw_nonzero().get() as u32;
    if pgid == shell {
        return None;
    }
    let argv = argv(pgid)?;
    (!argv.is_empty()).then(|| argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "))
}

#[cfg(target_os = "macos")]
fn argv(pid: u32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut len: libc::size_t = 0;
    // SAFETY: a null buffer asks only for the size, written to `len`.
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` holds `len` bytes, and sysctl writes at most that and updates `len`.
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } != 0 {
        return None;
    }
    buf.truncate(len);
    parse_procargs(&buf)
}

#[cfg(target_os = "linux")]
fn argv(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(raw.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect())
}

/// The layout is undocumented outside xnu's source: argc, exec path, NUL padding, argv.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_procargs(buf: &[u8]) -> Option<Vec<String>> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let rest = &buf[4..];
    let after_path = rest.iter().position(|&b| b == 0)?;
    let start = after_path + rest[after_path..].iter().position(|&b| b != 0)?;
    let args = rest[start..]
        .split(|&b| b == 0)
        .take(argc.max(0) as usize)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    Some(args)
}

fn quote(arg: &str) -> String {
    let plain = !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+~".contains(c));
    if plain { arg.to_string() } else { format!("'{}'", arg.replace('\'', r"'\''")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procargs_skip_the_path_and_padding_and_stop_at_argc() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/nvim\0\0\0\0nvim\0src/main.rs\0HOME=/x\0");
        assert_eq!(parse_procargs(&buf), Some(vec!["nvim".into(), "src/main.rs".into()]));
        assert_eq!(parse_procargs(b"\x01"), None);
    }

    #[test]
    fn arguments_are_quoted_only_when_needed() {
        assert_eq!(quote("src/main.rs"), "src/main.rs");
        assert_eq!(quote("two words"), "'two words'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote(""), "''");
    }
}
