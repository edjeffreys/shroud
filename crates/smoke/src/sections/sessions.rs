//! Several clients, the switcher, the CLI and layout files.

use std::time::Duration;

use crate::harness::{ESC, Env, LEADER, Report, WAIT, eventually};

// Found by fuzzing: kdl spends seconds on it, inside the size and depth limits.
const SLOW_LAYOUT: &[u8] = b"\x0e \x00\x00\x08/(e\x00\x00~'\" 000e77388,$\x0b/- \x00( /- {08,$\x0b/- \x00( /- {0Ke\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x088P]/, 8\x10]\x10]8PM/-]8000\x00( /- 00e76388,$\x0b/- \x00( /- {0Ke\x00\x00\x00#\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00~'\" 000e\x00\x00\x00\x00~'\" 000e77388,$\x0b/-\x19\x00]( /- {5Ke\x00\x00\x00\x00\x00~'\" 000e\x00\x00\x00\x00~'\" 000e77388,$\x0b/-\x19\x00]( /- {5Ke\x00\x00~'\" 000Ke\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x00\x088P]/, 8\x10]\x10]8PM/-]8000\x00( /- 00e76388,$\x0b/- \x00( /- {0Ke\x00\x00\x00#\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00~'\" 000e\x00\x00\x00\x00~'\" 000e77388,$\x0b/-\x19\x00]( /- {5Ke\x00\x00\x00\x00\x00~'\" 000e\x00\x00\x00\x00~'\" 000e77388,$\x0b/-\x19\x00]( /- {5Ke\x00\x00~'\" 000e7738( /;8888888";

pub fn run(env: &Env, r: &mut Report) {
    let mut main = env.spawn(None, &[]);
    main.ready();
    main.send(b"echo shroud-$((40+2))\r");
    main.wait_line("shroud-42");

    let other = env.tmp.join("other-dir");
    std::fs::create_dir_all(&other).unwrap();
    let mut c2 = env.spawn(Some(&other), &["session", "new", "second"]);
    c2.ready();
    let listing = env.cli(&["session", "ls"]).1;
    r.check_or(
        "shroud session new starts a named session; session ls lists both",
        listing.contains("default") && listing.contains("second: 1 tab (attached)"),
        &listing,
    );
    c2.send(&[LEADER, b";"].concat());
    r.check(
        "the switcher lists sessions with the current one tagged",
        c2.wait(WAIT, |c| c.screen().contains("default") && c.screen().contains("current")),
    );
    c2.send(b"\r");
    r.check("enter in the switcher moves the client to the selected session", c2.wait_line("shroud-42"));
    c2.send(&[LEADER, b")"].concat());
    c2.wait(WAIT, |c| !c.has_line("shroud-42"));
    c2.send(&[LEADER, b"'"].concat());
    let back = c2.wait_line("shroud-42");
    c2.send(&[LEADER, b"'"].concat());
    c2.wait(WAIT, |c| !c.has_line("shroud-42"));
    let listing = env.cli(&["session", "ls"]).1;
    r.check_or(
        "leader ' toggles to the last session and back",
        back && listing.contains("second: 1 tab (attached)"),
        &listing,
    );
    let (code, _) = env.cli(&["session", "rename", "second", "third"]);
    r.check(
        "leader+) cycles sessions and session rename shows in the bar",
        code == 0 && c2.wait(WAIT, |c| c.status().contains("third")),
    );
    let (code, _) = env.cli(&["session", "kill", "third"]);
    r.check("session kill moves its clients to the most recent session left", code == 0 && c2.wait_line("shroud-42"));
    c2.send(&[LEADER, b"C"].concat());
    c2.settle();
    c2.send(b"fourth\r");
    let made = c2.wait(WAIT, |_| env.cli(&["session", "ls"]).1.contains("fourth: 1 tab (attached)"));
    r.check("leader+C creates a session from inside shroud", made);
    c2.send(&[LEADER, b";"].concat());
    c2.wait_screen("current");
    c2.send(b"\x1b[120;5u");
    r.check("ctrl+x in the switcher asks before closing", c2.wait_screen("y/n"));
    c2.send(b"n");
    c2.settle();
    c2.send(ESC);
    c2.wait_gone("current");
    env.cli(&["session", "kill", "fourth"]);
    c2.wait_line("shroud-42");

    let project = env.tmp.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(".shroud.kdl"),
        "tab {\n  vsplit {\n    pane\n    pane command=\"echo from-$((6*7))-layout\"\n  }\n}\ntab\n",
    )
    .unwrap();
    let mut c4 = env.spawn(Some(&project), &["session", "new", "proj"]);
    let laid = c4.wait(WAIT, |c| c.screen().contains("from-42-layout") && c.screen().contains('│'));
    let listing = env.cli(&["session", "ls"]).1;
    r.check_or(
        "a new session starts from the directory's .shroud.kdl",
        laid && listing.contains("proj: 2 tabs"),
        &listing,
    );
    env.cli(&["session", "kill", "proj"]);
    c4.wait_line("shroud-42");
    c4.send(&[LEADER, b"d"].concat());
    c4.wait_exit(WAIT);

    let slow = env.tmp.join("slow");
    std::fs::create_dir_all(&slow).unwrap();
    std::fs::write(slow.join(".shroud.kdl"), SLOW_LAYOUT).unwrap();
    let mut c5 = env.spawn(Some(&slow), &["session", "new", "slow"]);
    let said = c5.wait(Duration::from_secs(6), |c| c.status().contains("probably malformed"));
    let listing = env.cli(&["session", "ls"]).1;
    r.check_or(
        "a layout kdl can't parse in time starts one pane and says why",
        said && listing.contains("slow: 1 tab"),
        (&listing, c5.status()),
    );
    let parser_gone = eventually(WAIT, || layout_children(env).is_empty());
    r.check_or("the timed-out layout parser is gone", parser_gone, layout_children(env));
    env.cli(&["session", "kill", "slow"]);
    c5.wait_line("shroud-42");
    c5.send(&[LEADER, b"d"].concat());
    c5.wait_exit(WAIT);

    c2.send(&[LEADER, b"d"].concat());
    r.check("a second client detaches cleanly", c2.wait_exit(WAIT) == Some(0));
    let mut c3 = env.spawn(None, &["session", "attach", "nope"]);
    let code = c3.wait_exit(WAIT);
    r.check_or(
        "attaching to a missing session is refused",
        code == Some(1) && c3.screen().contains("no session named nope"),
        (code, c3.screen()),
    );
}

/// Children of this section's server only, since other sections' servers run alongside.
fn layout_children(env: &Env) -> Vec<String> {
    let Some(server) = env.server_pid() else { return Vec::new() };
    let out = std::process::Command::new("pgrep").args(["-P", &server.to_string(), "-f", "__layout"]).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().map(str::to_string).collect()).unwrap_or_default()
}
