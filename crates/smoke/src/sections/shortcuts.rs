//! Keys that run programs: command panes that close or hold, and background commands that
//! report to the status bar. Each command prints the output mark itself, as the smoke shell
//! only sends prompt marks.

use std::time::Duration;

use serde_json::{Value, json};

use crate::ctl::{Ctl, Events, is};
use crate::harness::{Client, Env, LEADER, Report, WAIT, eventually, has_clock};

const CONFIG: &str = r#"[keys.leader]
"g e" = { run = "printf '\\033]133;C\\007'; echo run-ok" }
"g f" = { run = "printf '\\033]133;C\\007'; echo run-bad; (exit 4)" }
"g k" = { run = "printf '\\033]133;C\\007'; echo kept", keep = true, open = "right" }
"g x" = { exec = ["sh", "-c", "echo pulled-ok"] }
"g z" = { exec = "echo nope >&2; exit 2" }
"g s" = { exec = ["sleep", "30"], timeout = 1 }
"#;

pub fn run(env: &Env, r: &mut Report) {
    env.write_config(CONFIG);
    let mut c = env.spawn(None, &[]);
    c.wait(WAIT, |c| has_clock(&c.status()));
    c.ready();
    let mut ev = Events::start(env);
    // The server drops events until the subscription lands, so wait for one to come through.
    eventually(WAIT, || {
        c.send(b"printf '\\a'\r");
        ev.wait(Duration::from_secs(1), |e| is(e, "bell"))
    });
    ev.clear();
    panes(&mut c, env, &mut ev, r);
    jobs(&mut c, r);
    api(&mut c, env, r);
}

fn pane_count(env: &Env) -> usize {
    let ls: Value = serde_json::from_str(&env.cli(&["pane", "ls"]).1).unwrap_or(Value::Null);
    ls[0]["tabs"].as_array().into_iter().flatten().map(|t| t["panes"].as_array().map_or(0, Vec::len)).sum()
}

fn panes(c: &mut Client, env: &Env, ev: &mut Events, r: &mut Report) {
    c.send(&[LEADER, b"ge"].concat());
    let opened = ev.wait(WAIT, |e| is(e, "pane_opened"));
    let closed = ev.wait(WAIT, |e| is(e, "pane_closed"));
    r.check_or("a run shortcut opens a pane and closes it when the command succeeds", opened && closed, ev.names());
    eventually(WAIT, || pane_count(env) == 1);

    c.send(&[LEADER, b"gf"].concat());
    let held = c.wait(WAIT, |c| c.status().contains("exited 4 · Enter reruns · q closes"));
    r.check_or("a failed command holds its pane and says how to rerun", held && pane_count(env) == 2, c.status());
    c.send(b"\r");
    r.check_or(
        "Enter in a held pane runs the command again",
        c.wait(WAIT, |c| c.rows().iter().filter(|row| row.contains("│run-bad ")).count() >= 2),
        c.screen(),
    );
    c.wait(WAIT, |c| c.status().contains("exited 4 · Enter"));
    c.send(b"q");
    r.check("q closes a held pane", eventually(WAIT, || pane_count(env) == 1));

    c.send(&[LEADER, b"gk"].concat());
    let kept = c.wait(WAIT, |c| c.status().contains("finished · Enter reruns"));
    r.check_or("keep holds the pane even when the command succeeds", kept && pane_count(env) == 2, c.status());
    c.send(b"echo released-$((2+3))\r");
    r.check_or(
        "any other key gives the held pane back to its shell",
        c.wait(WAIT, |c| c.screen().contains("released-5")),
        c.screen(),
    );
    c.send(b"exit\r");
    eventually(WAIT, || pane_count(env) == 1);
}

fn jobs(c: &mut Client, r: &mut Report) {
    c.send(&[LEADER, b"gx"].concat());
    r.check_or(
        "an exec shortcut reports success and its last line in the bar",
        c.wait(WAIT, |c| c.status().contains("echo pulled-ok ✓ pulled-ok")),
        c.status(),
    );
    c.send(&[LEADER, b"gz"].concat());
    r.check_or(
        "a failing exec shows its exit code and stderr",
        c.wait(WAIT, |c| c.status().contains("✗2 nope")),
        c.status(),
    );
    c.send(&[LEADER, b"gs"].concat());
    r.check_or(
        "an exec past its timeout is stopped and says so",
        c.wait(Duration::from_secs(8), |c| c.status().contains("sleep 30 ✗ stopped after 1s")),
        c.status(),
    );
}

fn api(c: &mut Client, env: &Env, r: &mut Report) {
    let mut ctl = Ctl::connect(env);
    let b = ctl.reply(json!({"cmd": "bindings"}));
    let shown = b["after_leader"]
        .as_array()
        .is_some_and(|l| l.iter().any(|e| e["keys"] == "g x" && e["command"] == "sh -c echo pulled-ok"));
    r.check_or("bindings name a shortcut by what it runs", shown, &b["after_leader"]);
    let reply = ctl.reply(json!({"cmd": "run", "command": {"exec": ["sh", "-c", "echo via-api"]}}));
    r.check_or(
        "run takes a shortcut table",
        reply["ok"] == true && c.wait(WAIT, |c| c.status().contains("✓ via-api")),
        (&reply, c.status()),
    );
    let (code, out) = env.cli(&["pane", "run", "--keep", "printf '\\033]133;C\\007'; (exit 5)"]);
    let held = c.wait(WAIT, |c| c.status().contains("exited 5 · Enter reruns"));
    r.check_or("pane run --keep makes a command pane", code == 0 && held, (out, c.status()));
    let bad = ctl.reply(json!({"cmd": "bind", "keys": "g q", "command": {"run": "x", "open": "left"}}));
    r.check_or(
        "a bad shortcut table is refused with the reason",
        bad["ok"] == false && bad["error"].as_str().is_some_and(|e| e.contains("\"left\"")),
        &bad,
    );
}
