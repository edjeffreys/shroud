//! The end-to-end check, run as `cargo run -p shroud-smoke -- target/debug/shroud [section…]`.
//! Kept out of `cargo test`, which must never spawn a PTY or open a socket.

mod ctl;
mod harness;
mod sections;

use std::path::PathBuf;
use std::time::Instant;

use harness::{Env, Report};

fn main() {
    let mut args = std::env::args().skip(1);
    let bin = std::fs::canonicalize(args.next().unwrap_or_else(|| "target/debug/shroud".into()))
        .expect("the shroud binary to test");
    let only: Vec<String> = args.collect();
    let started = Instant::now();
    let chosen: Vec<_> =
        sections::ALL.iter().filter(|(name, _)| only.is_empty() || only.iter().any(|o| o == name)).copied().collect();
    let handles: Vec<_> = chosen
        .iter()
        .map(|&(name, run)| {
            let bin: PathBuf = bin.clone();
            std::thread::spawn(move || {
                let env = Env::new(&bin, name);
                let mut report = Report::default();
                let t = Instant::now();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&env, &mut report)));
                if let Err(p) = outcome {
                    let why = p
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    report.check(&format!("the section ran to the end (panicked: {why})"), false);
                }
                env.stop_server();
                if !report.failed() {
                    let _ = std::fs::remove_dir_all(&env.tmp);
                }
                (report, env.tmp.clone(), t.elapsed())
            })
        })
        .collect();
    let mut failed = false;
    let (mut passed, mut total) = (0, 0);
    for (&(name, _), h) in chosen.iter().zip(handles) {
        let (report, tmp, took) = h.join().expect("section thread");
        println!("== {name} ({:.1}s)", took.as_secs_f64());
        for (ok, line) in &report.lines {
            println!("{} {line}", if *ok { "PASS" } else { "FAIL" });
            total += 1;
            passed += usize::from(*ok);
        }
        if report.failed() {
            failed = true;
            println!("   kept for its logs: {}", tmp.display());
        }
    }
    println!("{passed}/{total} passed in {:.1}s", started.elapsed().as_secs_f64());
    std::process::exit(i32::from(failed));
}
