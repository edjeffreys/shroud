//! Detach, reattach, restart and the last pane closing, which ends the server.

use std::time::Duration;

use crate::harness::{Env, LEADER, Report, WAIT, contains};

pub fn run(env: &Env, r: &mut Report) {
    let mut c = env.spawn(None, &[]);
    c.ready();
    c.send(b"echo shroud-$((40+2))\r");
    c.wait_line("shroud-42");
    c.send(&[LEADER, b"v"].concat());
    c.wait_screen("│");

    let at = c.mark();
    c.send(&[LEADER, b"d"].concat());
    let restored = c.wait(WAIT, |c| contains(c.since(at), b"\x1b[?1049l") && contains(c.since(at), b"[detached]"));
    r.check("detach restores the terminal", restored && c.wait_exit(WAIT) == Some(0));

    let mut c = env.spawn(None, &[]);
    let ok = c.wait(WAIT, |c| c.screen().contains("shroud-42") && c.screen().contains('│'));
    r.check_or("reattach redraws previous output", ok, c.screen());

    let old = env.server_pid();
    let at = c.mark();
    let (code, reply) = env.cli(&["server", "restart"]);
    // Reading the client while waiting: unread, it blocks, and the old server can't finish with it.
    let renewed = c.wait(WAIT, |_| env.server_pid().is_some_and(|p| Some(p) != old));
    let redrawn = c.wait(WAIT, |c| contains(c.since(at), b"\x1b[?1049h") && contains(c.since(at), b"\x1b[?2026h"));
    r.check_or(
        "server restart starts a new server and the window reattaches to it",
        code == 0 && renewed && redrawn && c.exit_code().is_none(),
        (reply, old, env.server_pid(), renewed, redrawn, c.screen()),
    );
    c.send(b"echo restarted-$((1+1))\r");
    r.check("the restarted window has a working shell", c.wait_line("restarted-2"));

    let mut code = None;
    for _ in 0..4 {
        c.send(b"exit\r");
        code = c.wait_exit(Duration::from_secs(2));
        if code.is_some() {
            break;
        }
    }
    let gone = c.wait(WAIT, |_| env.server_pid().is_none());
    r.check_or("closing the last pane ends client and server cleanly", code == Some(0) && gone, code);
}
