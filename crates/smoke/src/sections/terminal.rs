//! One client through everything a terminal sees: setup, colours, panes, tabs, visual mode,
//! copying and the mouse.

use std::time::Duration;

use crate::harness::{Client, ESC, Env, LEADER, OUTPUT_START, Report, WAIT, contains, cpu_seconds, has_clock};

pub fn run(env: &Env, r: &mut Report) {
    let mut c = env.spawn(None, &[]);
    c.wait(WAIT, |c| has_clock(&c.status()) && contains(&c.raw, b"\x1b[?2027$p"));
    c.ready();
    r.check(
        "setup sequences sent (alt screen, kitty flags)",
        contains(&c.raw, b"\x1b[?1049h") && contains(&c.raw, b"\x1b[>31u"),
    );
    r.check("first frame uses synchronized output", contains(&c.raw, b"\x1b[?2026h"));
    r.check("status bar shows a clock", has_clock(&c.status()));
    r.check("attach asks the terminal for its colours", contains(&c.raw, b"\x1b]11;?") && contains(&c.raw, b";255;?"));
    r.check("attach asks whether the terminal clusters graphemes", contains(&c.raw, b"\x1b[?2027$p"));

    colours(&mut c, env, r);
    output(&mut c, r);
    panes(&mut c, r);
    tabs(&mut c, r);
    visual(&mut c, r);
    copying(&mut c, r);
    jumping(&mut c, r);
}

fn colours(c: &mut Client, env: &Env, r: &mut Report) {
    c.send(b"\x1b]11;rgb:1010/2020/3030\x1b\\");
    c.settle();
    c.send(b"printf '\\033]11;?\\033\\\\'; read -rs -d '\\' -t 2 r; echo \"bg=${r#*rgb:}\"\r");
    r.check("a pane program's OSC 11 query gets the terminal's real background", c.wait_line("bg=1010/2020/3030"));
    c.send(b"printf '\\033[?2031h'; echo ar$((0))med; read -rs -d n -t 3 r; echo \"scheme=${r#*997;}\"\r");
    c.wait_line("ar0med");
    let at = c.mark();
    c.send(b"\x1b[?997;2n");
    let ok = c.wait(WAIT, |c| contains(c.since(at), b"\x1b]11;?") && c.has_line("scheme=2"));
    r.check("a light/dark switch re-queries colours and reaches panes using mode 2031", ok);

    c.send(b"echo shroud-$((40+2))\r");
    r.check("typed command echoes through pane", c.wait_line("shroud-42"));
    c.settle();
    let idle = env.server_pid().map(|p| {
        let from = cpu_seconds(p);
        c.idle(Duration::from_secs(5));
        cpu_seconds(p) - from
    });
    r.check_or(
        &format!("an idle server stays under 1% CPU ({:.0} ms in 5 s)", idle.unwrap_or(-1.0) * 1000.0),
        idle.is_some_and(|s| (0.0..0.05).contains(&s)),
        idle,
    );
}

fn output(c: &mut Client, r: &mut Report) {
    let at = c.mark();
    c.send("printf '\\360\\237\\221\\215\\360\\237\\217\\275|\\n'\r".as_bytes());
    // Screen rows keep one codepoint per cell, so the skin tone isn't in them.
    c.wait_line("👍|");
    let out = c.since(at);
    r.check(
        "an emoji with a skin tone is one cluster and what follows is placed explicitly",
        contains(out, "👍🏽\x1b[".as_bytes()) && !contains(out, "👍🏽|".as_bytes()),
    );

    let at = c.mark();
    c.send(b"printf '\\033]8;;https://shroud.test/x\\033\\\\link-%s\\033]8;;\\033\\\\\\n' me\r");
    let link: &[u8] = b"\x1b]8;;https://shroud.test/x\x1b\\link-me";
    c.wait_raw(at, link);
    let out = c.since(at);
    let after = out.windows(link.len()).position(|w| w == link).map(|i| &out[i + link.len()..]);
    r.check(
        "OSC 8 links reach the terminal around their text",
        after.is_some_and(|rest| contains(rest, b"\x1b]8;;\x1b\\")),
    );

    let at = c.mark();
    c.send(b"printf '\\033_Ga=T,f=24,s=2,v=1,i=5,q=2;/wAAAP8A\\033\\\\'; echo\r");
    let ok = c.wait(WAIT, |c| {
        contains(c.since(at), b"\x1b_Ga=t,q=2,f=24,s=2,v=1,") && contains(c.since(at), b"\x1b_Ga=p,q=2,C=1,")
    });
    r.check("a Kitty image in a pane is sent on and placed", ok);
    let at = c.mark();
    c.send(b"clear\r");
    let ok =
        c.wait(WAIT, |c| contains(c.since(at), b"\x1b_Ga=d,q=2,d=i,") || contains(c.since(at), b"\x1b_Ga=d,q=2,d=I,"));
    r.check("clearing the screen takes the image away", ok);
}

fn panes(c: &mut Client, r: &mut Report) {
    c.send(&[LEADER, b"f"].concat());
    r.check(
        "leader f opens a float with its own frame",
        c.wait(WAIT, |c| c.screen().contains('┌') && c.screen().contains('┘')),
    );
    c.send(b"echo in-$((2*3))-float\r");
    r.check("the float takes focus and input", c.wait_screen("in-6-float"));
    c.send(&[LEADER, b"f"].concat());
    c.wait_gone("┌");
    c.send(b"echo back-$((3*3))\r");
    r.check(
        "leader f again hides it and focus returns to the tile",
        c.wait_line("back-9") && !c.screen().contains('┌'),
    );
    c.send(&[LEADER, b"f"].concat());
    c.wait_screen("┌");
    c.send(b"exit\r");
    c.wait_gone("┌");

    c.send(&[LEADER, b"v"].concat());
    r.check("leader+v draws a vertical divider", c.wait_screen("│"));
    c.send(b"printf 'right-pane\\n'\r");
    r.check("new pane is focused and receives input", c.wait_screen("│right-pane"));

    c.send(&[OUTPUT_START, b"echo copy-me\r"].concat());
    c.wait_screen("│copy-me");
    let at = c.mark();
    c.send(&[LEADER, b"y"].concat());
    let ok = c.wait(WAIT, |c| contains(c.since(at), b"\x1b]52;c;Y29weS1tZQ==") && c.status().contains("copied 1 line"));
    r.check("leader+y copies the last output via OSC 52", ok);

    c.send(&[LEADER, b"["].concat());
    r.check("leader+[ enters jump mode", c.wait(WAIT, |c| c.status().contains("JUMP")));
    c.send(ESC);
    c.wait(WAIT, |c| !c.status().contains("JUMP"));

    c.send(&[OUTPUT_START, b"sleep 1.2; false\r"].concat());
    c.idle(Duration::from_millis(300));
    c.send(&[LEADER, b"h"].concat());
    r.check("a failed command in an unfocused pane gets a badge", c.wait(WAIT, |c| c.status().contains('✗')));
    c.send(&[LEADER, b"l"].concat());
    c.settle();
}

fn tabs(c: &mut Client, r: &mut Report) {
    c.send(&[LEADER, b"N"].concat());
    c.wait_gone("│");
    c.send(b"echo tab-two\r");
    r.check("leader+N opens a new tab", c.wait_line("tab-two"));
    c.send(&[LEADER, b"gT"].concat());
    r.check(
        "leader+gT shows the previous tab",
        c.wait(WAIT, |c| c.screen().contains("│right-pane") && !c.has_line("tab-two")),
    );
    c.send(&[LEADER, b"gt"].concat());
    r.check("leader+gt returns to the next tab", c.wait_line("tab-two"));
    c.send(&[LEADER, b"T"].concat());
    r.check(
        "leader+T refuses to move a pane that is alone",
        c.wait(WAIT, |c| c.status().contains("only one pane in this tab")),
    );
    c.send(b"exit\r");
    r.check("exiting a tab's last shell closes the tab", c.wait_screen("│right-pane"));
    c.send(&[LEADER, b"T"].concat());
    c.wait_gone("│");
    c.send(b"echo moved-pane\r");
    c.wait_line("moved-pane");
    c.send(&[LEADER, b"gT"].concat());
    let moved_away = c.wait(WAIT, |c| !c.has_line("moved-pane"));
    c.send(&[LEADER, b"gt"].concat());
    let back = c.wait(WAIT, |c| c.has_line("moved-pane") && !c.screen().contains('│'));
    r.check("leader+T moves the focused pane into its own tab", moved_away && back);
    c.send(b"exit\r");
    c.settle();
    c.send(&[LEADER, b"v"].concat());
    c.settle();

    c.send(&[LEADER, b"e"].concat());
    c.wait_screen("┌");
    c.send(b"echo floated-$((2*4))\r");
    r.check(
        "leader e floats the focused tile, which keeps focus",
        c.wait(WAIT, |c| c.screen().contains('┌') && c.screen().contains("floated-8")),
    );
    c.send(&[LEADER, b"e"].concat());
    r.check(
        "leader e again tiles it beside the others",
        c.wait(WAIT, |c| c.screen().contains('│') && !c.screen().contains('┌')),
    );
    c.send(&[LEADER, b"z"].concat());
    r.check("leader z zooms the pane and the bar says so", c.wait(WAIT, |c| c.status().contains("ZOOM")));
    c.send(&[LEADER, b"z"].concat());
    r.check(
        "leader z again restores the split",
        c.wait(WAIT, |c| c.screen().contains('│') && !c.status().contains("ZOOM")),
    );
}

fn visual(c: &mut Client, r: &mut Report) {
    c.send(b"echo find-the-needle-here\r");
    c.wait(WAIT, |c| c.screen().matches("find-the-needle-here").count() >= 2);
    c.send(&[LEADER, b"V"].concat());
    r.check("leader V enters visual mode", c.wait(WAIT, |c| c.status().contains("VISUAL")));
    let at = c.mark();
    c.send(b"/needle");
    let typed = c.wait(WAIT, |c| c.status().contains("/needle"));
    let lit = c.wait(WAIT, |c| contains(c.since(at), b"[4m") || contains(c.since(at), b";4m"));
    r.check("visual search underlines matches as you type", lit);
    c.send(b"\r");
    c.wait(WAIT, |c| !c.status().contains("/needle"));
    let at = c.mark();
    c.send(b"vey");
    let ok = c.wait(WAIT, |c| contains(c.since(at), b"\x1b]52;c;bmVlZGxl") && c.status().contains("copied 1 line"));
    r.check("visual mode searches, selects a word and copies it", typed && ok);

    c.send(&[LEADER, b"V"].concat());
    c.wait(WAIT, |c| c.status().contains("VISUAL"));
    c.send(b"/needle\r");
    c.settle();
    let at = c.mark();
    c.send(b"yiw");
    r.check("visual yiw copies the word under the cursor without selecting it", c.wait_raw(at, b"\x1b]52;c;bmVlZGxl"));
    c.send(&[LEADER, b"gV"].concat());
    r.check(
        "leader g V starts a line selection and the bar says V-LINE",
        c.wait(WAIT, |c| c.status().contains("V-LINE")),
    );
    c.send(&[ESC, ESC].concat());
    c.wait(WAIT, |c| !c.status().contains("V-LINE") && !c.status().contains("VISUAL"));
}

fn copying(c: &mut Client, r: &mut Report) {
    c.send(&[LEADER, b"N"].concat());
    c.wait_gone("│");
    c.send(b"echo tab-$((1+1))-here\r");
    c.wait_line("tab-2-here");
    c.send(&[LEADER, b","].concat());
    c.settle();
    c.send(b"logs\r");
    r.check("leader , names the tab and the bar shows it", c.wait(WAIT, |c| c.status().contains("2:logs")));
    c.send(&[LEADER, b"e"].concat());
    let refused = c.wait(WAIT, |c| c.status().contains("one tiled pane"));
    r.check_or(
        "leader e on a tab's only tile refuses and leaves the layout alone",
        refused && !c.screen().contains('┼'),
        c.status(),
    );

    c.send(b"clear; echo drag-me\r");
    c.wait(WAIT, |c| c.rows()[0] == "drag-me");
    let at = c.mark();
    c.send(b"\x1b[<0;1;1M\x1b[<32;4;1M\x1b[<32;7;1M\x1b[<0;7;1m");
    r.check("a mouse drag in a plain pane copies what it covered", c.wait_raw(at, b"\x1b]52;c;ZHJhZy1tZQ=="));
    c.send(b"printf '\\033]133;A\\007$ \\033]133;B\\007echo hi\\r\\n\\033]133;C\\007hi\\r\\n\\033]133;D;0\\007'\r");
    c.wait_line("hi");
    let at = c.mark();
    c.send(&[LEADER, b"Y"].concat());
    r.check("leader Y copies the command line and its output", c.wait_raw(at, b"\x1b]52;c;JCBlY2hvIGhpCmhp"));
    let at = c.mark();
    c.send(&[LEADER, b"y"].concat());
    r.check("leader y copies only the output", c.wait_raw(at, b"\x1b]52;c;aGk="));

    c.send(b"clear; echo tab-$((1+1))-here\r");
    c.wait(WAIT, |c| c.rows()[0] == "tab-2-here");
    c.send(&[LEADER, b"1"].concat());
    let gone = c.wait(WAIT, |c| !c.has_line("tab-2-here"));
    c.send(&[LEADER, b"2"].concat());
    r.check("leader 1 and leader 2 go straight to those tabs", gone && c.wait_line("tab-2-here"));
    c.send(&[LEADER, b"g\t"].concat());
    let gone = c.wait(WAIT, |c| !c.has_line("tab-2-here"));
    c.send(&[LEADER, b"g\t"].concat());
    r.check("leader g tab toggles to the last tab and back", gone && c.wait_line("tab-2-here"));
}

fn jumping(c: &mut Client, r: &mut Report) {
    // `(exit 3)` because the smoke shell reports `$?` again at its next prompt.
    c.send(b"clear; printf '\\033]133;A\\007$ \\033]133;B\\007echo alpha\\r\\n\\033]133;C\\007alpha-out\\r\\n\\033]133;D;3\\007'; (exit 3)\r");
    c.wait_line("alpha-out");
    c.send(&[OUTPUT_START, b"echo beta-out\r"].concat());
    c.wait_line("beta-out");
    c.send(&[LEADER, b"["].concat());
    let first = c.wait(WAIT, |c| c.status().contains("JUMP") && c.status().contains('/'));
    c.send(b"[");
    let second = c.wait(WAIT, |c| c.status().contains("echo alpha") && c.status().contains("✗3"));
    r.check_or("jump mode names the selected command and its exit code in the bar", first && second, c.status());
    let at = c.mark();
    c.send(b"Y");
    let copied = c.wait_raw(at, b"\x1b]52;c;JCBlY2hvIGFscGhhCmFscGhhLW91dA==");
    r.check(
        "Y in jump mode copies that command and its output, and leaves",
        copied && c.wait(WAIT, |c| !c.status().contains("JUMP")),
    );
    c.send(&[LEADER, b"["].concat());
    c.wait(WAIT, |c| c.status().contains("JUMP"));
    c.send(b"v");
    let visual = c.wait(WAIT, |c| c.status().contains("VISUAL"));
    let at = c.mark();
    c.send(b"y");
    r.check(
        "v in jump mode selects the command's output in visual mode",
        visual && c.wait_raw(at, b"\x1b]52;c;YmV0YS1vdXQ="),
    );
}
