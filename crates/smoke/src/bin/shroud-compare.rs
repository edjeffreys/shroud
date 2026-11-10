//! The outer PTY is drained as fast as it can be read, so this times the multiplexer rather than a
//! terminal's renderer. Each tool gets the same bash, an isolated socket and no user config.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use std::{env, fs};

use shroud_smoke::pty::Pty;

const COLS: u16 = 200;
const ROWS: u16 = 50;
const KEYS: usize = 200;
const FLOOD_KEYS: usize = 50;
const ECHO: &[u8] = b"stty -icanon -echo; while IFS= read -rn1 c; do printf '\\r%s' \"$c$c$c$c$c$c$c$c\"; done\r";

#[derive(Default)]
struct Result {
    startup: Option<f64>,
    idle_cpu: Option<f64>,
    lat: Latency,
    flood: Latency,
    cat: Option<f64>,
    cat_cpu: Option<f64>,
    rss_mb: Option<f64>,
}

#[derive(Default)]
struct Latency {
    p50: Option<f64>,
    p99: Option<f64>,
    missed: usize,
}

struct Tool {
    name: &'static str,
    argv: Vec<String>,
    flood: Option<String>,
    /// The process name to count CPU and memory for; none for plain bash, which has no server.
    process: Option<&'static str>,
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut opt = |name: &str, default: &str| match args.iter().position(|a| a == name) {
        Some(i) if i + 1 < args.len() => {
            let v = args.remove(i + 1);
            args.remove(i);
            v
        }
        _ => default.to_string(),
    };
    let mb: usize = opt("--mb", "100").parse().expect("--mb takes a number");
    let only = opt("--only", "plain,shroud,tmux,zellij");
    let shroud = fs::canonicalize(args.first().map(String::as_str).unwrap_or("target/release/shroud"))
        .expect("the shroud binary");
    // zellij's socket path has to fit in sun_path.
    let tmp = PathBuf::from(format!("/tmp/mc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("zellij")).unwrap();
    let data = tmp.join(format!("{mb}mb.txt"));
    let line =
        "plain text 0123456789 \x1b[31mred\x1b[0m \x1b[1;38;2;10;200;90mtruecolor\x1b[0m 世界 emoji ✨ tabs\tend\n"
            .as_bytes();
    let size = mb * 1024 * 1024;
    fs::write(&data, line.iter().copied().cycle().take(size).collect::<Vec<u8>>()).unwrap();
    let zconf = tmp.join("zellij/config.kdl");
    fs::write(&zconf, "show_startup_tips false\nshow_release_notes false\nmouse_mode false\npane_frames true\n")
        .unwrap();

    let s = shroud.display().to_string();
    let z = zconf.display().to_string();
    let zdir = tmp.join("zellij").display().to_string();
    let tools = [
        Tool {
            name: "plain",
            argv: vec!["/bin/bash".into(), "--norc".into(), "--noprofile".into()],
            flood: None,
            process: None,
        },
        Tool {
            name: "shroud",
            argv: vec![s.clone()],
            flood: Some(format!("{s} pane run yes 0123456789")),
            process: Some("shroud"),
        },
        Tool {
            name: "tmux",
            argv: [
                "tmux",
                "-L",
                "compare",
                "-f",
                "/dev/null",
                "new-session",
                "-s",
                "bench",
                "/bin/bash --norc --noprofile",
            ]
            .map(String::from)
            .to_vec(),
            flood: Some("tmux -L compare split-window -d 'yes 0123456789'".into()),
            process: Some("tmux"),
        },
        Tool {
            name: "zellij",
            argv: vec!["zellij".into(), "--config-dir".into(), zdir, "--config".into(), z, "-s".into(), "bench".into()],
            flood: Some("zellij action new-pane -d right -- yes 0123456789 && zellij action move-focus left".into()),
            process: Some("zellij"),
        },
    ];

    let mut results = Vec::new();
    for tool in tools.iter().filter(|t| only.split(',').any(|o| o == t.name)) {
        if tool.name != "plain" && !installed(&tool.argv[0]) {
            println!("skip {}: not installed", tool.name);
            continue;
        }
        println!("running {}…", tool.name);
        match run(tool, &tmp, &data) {
            Ok(r) => results.push((tool.name, r)),
            Err(e) => println!("  {} failed: {e}", tool.name),
        }
    }

    println!(
        "\n{mb} MB cat, {COLS}x{ROWS}, outer PTY drained without rendering; {KEYS} keystrokes, {FLOOD_KEYS} more while `yes` floods another pane\n"
    );
    println!(
        "{:8} {:>8} {:>7} {:>8} {:>8} {:>8} {:>9} {:>9} {:>9} {:>8} {:>8}",
        "", "cat", "MB/s", "cat CPU", "key p50", "key p99", "flood p50", "flood p99", "idle CPU", "RSS", "startup"
    );
    for (name, r) in &results {
        let rate = r.cat.map(|c| mb as f64 / c);
        let mut row = format!(
            "{name:8} {:>8} {:>7} {:>8} {:>8} {:>8} {:>9} {:>9} {:>9} {:>8} {:>8}",
            fmt(r.cat, " s", 2),
            fmt(rate, "", 0),
            fmt(r.cat_cpu, " s", 2),
            fmt(r.lat.p50, " ms", 2),
            fmt(r.lat.p99, " ms", 2),
            fmt(r.flood.p50, " ms", 2),
            fmt(r.flood.p99, " ms", 2),
            fmt(r.idle_cpu, "%", 2),
            fmt(r.rss_mb, " MB", 0),
            fmt(r.startup, " s", 2)
        );
        if r.lat.missed > 0 {
            row += &format!("  ({} keys missed)", r.lat.missed);
        }
        if r.flood.missed > 0 {
            row += &format!("  ({} flood keys missed)", r.flood.missed);
        }
        println!("{row}");
    }
    let _ = fs::remove_dir_all(&tmp);
}

fn installed(program: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {program}")).output().is_ok_and(|o| o.status.success())
}

fn fmt(v: Option<f64>, unit: &str, digits: usize) -> String {
    v.map_or("—".into(), |v| format!("{v:.digits$}{unit}"))
}

/// pid → (CPU seconds, RSS in KB, command name).
fn procs() -> HashMap<u32, (f64, u64, String)> {
    let out = Command::new("ps")
        .args(["-A", "-o", "pid=,time=,rss=,comm="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let time = f.next()?;
            let rss = f.next()?.parse().ok()?;
            let comm = f.collect::<Vec<_>>().join(" ");
            let secs = time.split(':').fold(0.0, |acc, part| acc * 60.0 + part.parse::<f64>().unwrap_or(0.0));
            Some((pid, (secs, rss, Path::new(&comm).file_name()?.to_string_lossy().into_owned())))
        })
        .collect()
}

struct Session {
    pty: Pty,
    buf: Vec<u8>,
}

impl Session {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.pty.write(bytes)
    }

    fn drain(&mut self, secs: f64, done: impl Fn(&[u8]) -> bool) -> Option<f64> {
        let start = Instant::now();
        let deadline = start + Duration::from_secs_f64(secs);
        self.buf.clear();
        while Instant::now() < deadline {
            if done(&self.buf) {
                return Some(start.elapsed().as_secs_f64());
            }
            if self.pty.read(Duration::from_millis(50), &mut self.buf) == 0 && self.pty.closed {
                break;
            }
            // Only the tail matters for markers; keeping it bounded keeps a 100 MB cat cheap.
            if self.buf.len() > 1 << 20 {
                self.buf.drain(..self.buf.len() - 4096);
            }
        }
        None
    }

    fn idle(&mut self, secs: f64) {
        self.drain(secs, |_| false);
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn run(tool: &Tool, tmp: &Path, data: &Path) -> std::result::Result<Result, String> {
    let before: HashSet<u32> = procs().into_keys().collect();
    let t0 = Instant::now();
    let mut cmd = Command::new(&tool.argv[0]);
    cmd.args(&tool.argv[1..]).current_dir(tmp).env_clear().envs(environment(tmp));
    let mut s = Session { pty: Pty::spawn(cmd, COLS, ROWS).map_err(|e| e.to_string())?, buf: Vec::new() };
    let mut r = Result {
        startup: s.drain(15.0, |b| contains(b, b"$ ")).map(|_| t0.elapsed().as_secs_f64()),
        ..Result::default()
    };

    // zellij drops keys typed before its pane exists, so the probe repeats.
    let mut ready = false;
    for _ in 0..50 {
        s.write(b"printf 'RE%sDY\\n' A\r").map_err(|_| format!("{} exited", tool.name))?;
        if s.drain(0.3, |b| contains(b, b"READY")).is_some() {
            ready = true;
            break;
        }
    }
    if !ready {
        return Err("the shell never answered".into());
    }
    s.idle(1.0);

    let mine = || -> HashMap<u32, (f64, u64, String)> {
        procs()
            .into_iter()
            .filter(|(p, v)| !before.contains(p) && tool.process.is_some_and(|n| v.2.contains(n)))
            .collect()
    };
    let cpu_since = |a: &HashMap<u32, (f64, u64, String)>, b: &HashMap<u32, (f64, u64, String)>| {
        b.iter().map(|(p, v)| v.0 - a.get(p).map_or(0.0, |x| x.0)).sum::<f64>()
    };

    let a = mine();
    s.idle(5.0);
    r.idle_cpu = tool.process.map(|_| cpu_since(&a, &mine()) / 5.0 * 100.0);

    // Each key comes back as eight copies of itself over the same spot: a status bar never
    // holds that, and every cell changes, so no renderer can skip part of it.
    s.write(ECHO).map_err(|e| e.to_string())?;
    s.idle(0.8);
    r.lat = echo_latency(&mut s, KEYS);
    s.write(b"\x03").map_err(|e| e.to_string())?;
    s.idle(0.3);
    s.write(b"stty sane\r").map_err(|e| e.to_string())?;
    s.idle(0.8);

    // A file, not text on screen: a multiplexer skips cells already showing the same character,
    // so a marker on screen can arrive in pieces.
    let a = mine();
    let done = tmp.join(format!("done-{}", tool.name));
    s.write(format!("clear; cat {}; touch {}\r", data.display(), done.display()).as_bytes())
        .map_err(|e| e.to_string())?;
    r.cat = s.drain(300.0, |_| done.exists());
    r.cat_cpu = tool.process.map(|_| cpu_since(&a, &mine()));
    s.idle(1.0);
    r.rss_mb = tool.process.map(|_| mine().values().map(|v| v.1 as f64).sum::<f64>() / 1024.0);

    // Without a second pane a frame can scroll the echo away before it's drawn, so plain bash
    // has no number here.
    if let Some(flood) = &tool.flood {
        s.write(format!("{flood}\r").as_bytes()).map_err(|e| e.to_string())?;
        s.idle(1.5);
        s.write(ECHO).map_err(|e| e.to_string())?;
        s.idle(1.0);
        r.flood = echo_latency(&mut s, FLOOD_KEYS);
        let _ = Command::new("pkill").args(["-f", "yes 0123456789"]).status();
    }
    s.idle(1.0);

    let left = mine();
    drop(s);
    for pid in left.keys() {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    Ok(r)
}

fn environment(tmp: &Path) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = env::vars()
        .filter(|(k, _)| {
            !["ZELLIJ", "TMUX", "SHROUD", "GHOSTTY"].iter().any(|p| k.starts_with(p)) && k != "TERM_PROGRAM"
        })
        .collect();
    let t = tmp.display().to_string();
    for (k, v) in [
        ("TERM", "xterm-256color".to_string()),
        ("SHELL", "/bin/bash".into()),
        ("PS1", "$ ".into()),
        ("BASH_SILENCE_DEPRECATION_WARNING", "1".into()),
        ("TMPDIR", format!("{t}/")),
        ("XDG_RUNTIME_DIR", t.clone()),
        ("XDG_CONFIG_HOME", format!("{t}/config")),
        ("XDG_DATA_HOME", format!("{t}/data")),
        ("XDG_CACHE_HOME", format!("{t}/cache")),
    ] {
        vars.retain(|(x, _)| x != k);
        vars.push((k.into(), v));
    }
    vars
}

fn echo_latency(s: &mut Session, keys: usize) -> Latency {
    let mut lat = Vec::new();
    for i in 0..keys {
        let ch = b"abcdefghijklmnopqrstuvwxyz"[i % 26];
        let want = [ch; 8];
        if s.write(&[ch]).is_err() {
            break;
        }
        if let Some(took) = s.drain(1.0, |b| contains(b, &want)) {
            lat.push(took * 1000.0);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    lat.sort_by(f64::total_cmp);
    let pick =
        |q: f64| (!lat.is_empty()).then(|| lat[((lat.len() as f64 * q) as usize).saturating_sub(1).min(lat.len() - 1)]);
    Latency { p50: (!lat.is_empty()).then(|| lat[lat.len() / 2]), p99: pick(0.99), missed: keys - lat.len() }
}
