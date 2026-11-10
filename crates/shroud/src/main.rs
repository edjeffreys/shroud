//! CLI entry: `shroud` attaches, starting a server if none is running.

mod bench;
mod client;
mod config_file;
mod layout_child;
mod plugin_cli;
mod process;
mod pty;
mod server;

use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use shroud_core::config::parse;
use shroud_core::protocol::{Attach, Control, socket_path};

/// CI sets SHROUD_VERSION for dev builds (`0.2.0-dev.12+g3f9c2ab`); other builds say Cargo.toml's version.
const VERSION: &str = match option_env!("SHROUD_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

const USAGE: &str = "usage:
  shroud                             attach to the most recent session, or start `default`
  shroud session new [<name>]        start a session in this directory and attach
  shroud session attach [<name>]     attach to a session
  shroud session ls                  list sessions
  shroud session kill <name>         close a session and its shells
  shroud session rename <old> <new>  rename a session
  shroud server restart              start a new server (after a rebuild); windows reattach, shells start afresh
  shroud server stop                 stop the server and close every shell
  shroud config check                report problems in config.toml and the layouts that apply here
";

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("shroud: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> io::Result<i32> {
    let dir = runtime_dir()?;
    let socket = socket_path::server_socket(&dir);
    let log = socket_path::server_log(&dir);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let inside = std::env::var_os("SHROUD").is_some();
    match args.as_slice() {
        [] if inside => {
            println!(
                "{USAGE}\nYou're inside shroud already: Ctrl+Space ; switches sessions and Ctrl+Space C starts one."
            );
            Ok(0)
        }
        ["session", "new" | "attach", ..] if inside => {
            eprintln!(
                "shroud: already inside shroud; use Ctrl+Space ; to switch sessions or Ctrl+Space C to create one"
            );
            Ok(1)
        }
        [] => client::attach(&socket, &log, Attach::Recent),
        ["session", rest @ ..] => session(&socket, &log, rest),
        ["server", "restart"] => client::control(&socket, Control::StopServer { restart: true }),
        ["server", "stop"] => client::control(&socket, Control::StopServer { restart: false }),
        ["config", "check"] => config_check(),
        ["bench", rest @ ..] => bench::run(rest),
        ["pane", rest @ ..] => client::api::pane(&socket_path::control_socket(&dir), rest),
        ["events"] => client::api::events(&socket_path::control_socket(&dir)),
        ["command", rest @ ..] => client::api::command(&socket_path::control_socket(&dir), rest),
        ["plugin", rest @ ..] => plugin_cli::run(rest, &socket_path::control_socket(&dir)),
        ["__plugins"] => plugin_host(),
        ["__plugins", "--specs", init] => {
            let reply = match shroud_plugins::Host::specs_of(Path::new(init)) {
                Ok(specs) => serde_json::json!(specs),
                Err(e) => serde_json::json!({"error": e}),
            };
            println!("{reply}");
            Ok(0)
        }
        ["__layout"] => layout_child::serve(),
        ["__server"] => {
            server::run(server::Options { socket, control: socket_path::control_socket(&dir) })?;
            Ok(0)
        }
        ["--version" | "-V"] => {
            println!("shroud {VERSION}");
            Ok(0)
        }
        ["help" | "--help" | "-h"] => {
            println!("{USAGE}{}{}\n{}", plugin_cli::USAGE, client::api::USAGE, bench::USAGE);
            Ok(0)
        }
        _ => {
            eprintln!("{USAGE}{}{}\n{}", plugin_cli::USAGE, client::api::USAGE, bench::USAGE);
            Ok(2)
        }
    }
}

/// Started by the server, which passes the socket and a token that proves it.
fn plugin_host() -> io::Result<i32> {
    let (Some(socket), Ok(token)) = (std::env::var_os(server::SOCKET_ENV), std::env::var(server::TOKEN_ENV)) else {
        eprintln!("shroud: __plugins is started by the server");
        return Ok(2);
    };
    shroud_plugins::run(Path::new(&socket), &token, config_file::data_dir().map(|d| d.join("data")))?;
    Ok(0)
}

fn session(socket: &Path, log: &Path, args: &[&str]) -> io::Result<i32> {
    match args {
        ["new"] | ["new", _] => {
            let cwd = std::env::current_dir()?;
            client::attach(
                socket,
                log,
                Attach::New { name: args.get(1).copied().unwrap_or(""), cwd: &cwd.to_string_lossy() },
            )
        }
        ["attach"] => client::attach(socket, log, Attach::Recent),
        ["attach", name] => client::attach(socket, log, Attach::Named(name)),
        ["ls"] => client::control(socket, Control::List),
        ["kill", name] => client::control(socket, Control::Kill(name)),
        ["rename", from, to] => client::control(socket, Control::Rename { from, to }),
        _ => {
            eprintln!("{USAGE}");
            Ok(2)
        }
    }
}

/// Reads the files itself, so it works without a server and before one would load them.
fn config_check() -> io::Result<i32> {
    let mut bad = false;
    let Some(path) = config_file::settings_path() else {
        eprintln!("shroud: no HOME or XDG_CONFIG_HOME, so there is no config to check");
        return Ok(1);
    };
    match std::fs::read_to_string(&path) {
        Err(_) => println!("{}: not found; using the defaults", path.display()),
        Ok(text) => match parse(&text) {
            Err(p) => {
                bad = true;
                println!("{}: {p}", path.display());
            }
            Ok((_, problems)) if problems.is_empty() => println!("{}: ok", path.display()),
            Ok((_, problems)) => {
                bad = true;
                problems.iter().for_each(|p| println!("{}: {p}", path.display()));
            }
        },
    }
    if let Some((path, text)) = config_file::layout_for(&std::env::current_dir()?) {
        match layout_child::parse(&text) {
            Ok(tabs) => println!(
                "{}: ok, {} tab{} (a new session here starts from it)",
                path.display(),
                tabs.len(),
                if tabs.len() == 1 { "" } else { "s" }
            ),
            Err(p) => {
                bad = true;
                println!("{}: {p}", path.display());
            }
        }
    }
    Ok(i32::from(bad))
}

fn runtime_dir() -> io::Result<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    let tmp = std::env::var("TMPDIR").ok();
    let dir = socket_path::runtime_dir(xdg.as_deref(), tmp.as_deref(), uid);
    // Created 0700 from the start, so there is no moment when others could get in.
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    secure(&dir, uid)?;
    Ok(dir)
}

// Anyone who can reach the socket can type into every pane.
fn secure(dir: &Path, uid: u32) -> io::Result<()> {
    // Not followed: in a shared /tmp, another user could plant a link to a directory of ours.
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(io::Error::other(format!("{} is not a directory", dir.display())));
    }
    if meta.uid() != uid {
        return Err(io::Error::other(format!("{} is owned by another user", dir.display())));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_refuses_a_symlink_and_tightens_a_loose_directory() {
        let base = std::env::temp_dir().join(format!("shroud-secure-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let uid = rustix::process::getuid().as_raw();

        assert!(secure(&link, uid).is_err());
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o755);
        secure(&real, uid).unwrap();
        assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
