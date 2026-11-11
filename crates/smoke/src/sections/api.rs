//! Live config, the event stream, the pane CLI and `control.sock`: overlays, plugin commands,
//! layouts, tags and the plugin-drawn bar.

use std::time::Duration;

use serde_json::{Value, json};

use crate::ctl::{Ctl, Events, is};
use crate::harness::{Client, ESC, Env, LEADER, Report, WAIT, contains, eventually, has_clock};

pub fn run(env: &Env, r: &mut Report) {
    let mut c = env.spawn(None, &[]);
    c.wait(WAIT, |c| has_clock(&c.status()));
    c.ready();
    config(&mut c, env, r);
    let mut ev = Events::start(env);
    // The server drops events until the subscription lands, so wait for one to come through.
    eventually(WAIT, || {
        c.send(b"printf '\\a'\r");
        ev.wait(Duration::from_secs(1), |e| is(e, "bell"))
    });
    ev.clear();
    events(&mut c, &mut ev, r);
    panes(&mut c, env, &mut ev, r);
    control(&mut c, env, &mut ev, r);
    frames(&mut c, env, r);
}

fn sessions(env: &Env) -> Value {
    serde_json::from_str(&env.cli(&["pane", "ls"]).1).unwrap_or(Value::Null)
}

fn id(text: &str) -> Value {
    text.trim().parse::<u64>().map_or(Value::Null, Value::from)
}

fn panes_of(env: &Env) -> Vec<Value> {
    sessions(env)[0]["tabs"][0]["panes"].as_array().cloned().unwrap_or_default()
}

fn config(c: &mut Client, env: &Env, r: &mut Report) {
    env.write_config("[keys.direct]\n\"alt+n\" = \"new-tab\"\n");
    r.check("config.toml reloads live", c.wait(WAIT, |c| c.status().contains("config reloaded")));
    c.send(b"\x1b[110;3u");
    r.check(
        "a direct key from config acts without the leader",
        c.wait(WAIT, |_| env.cli(&["session", "ls"]).1.contains("default: 2 tabs")),
    );
    c.ready();
    c.send(b"exit\r");
    c.wait(WAIT, |_| env.cli(&["session", "ls"]).1.contains("default: 1 tab"));
    c.send(&[LEADER, b"gp"].concat());
    c.settle();
    c.send(b"echo tick\r");
    r.check("leader g p shows the last frame's cost in the bar", c.wait(WAIT, |c| frame_time(&c.status())));
    c.send(&[LEADER, b"gp"].concat());
    c.settle();
    env.write_config("[theme]\nborder.nope = 1\n");
    r.check(
        "a config problem shows its line, with the config alert",
        c.wait(WAIT, |c| c.status().contains("line 2: unknown theme token")),
    );
    let (code, report) = env.cli(&["config", "check"]);
    r.check_or("shroud config check reports the problem", code == 1 && report.contains("line 2"), report);
    env.remove_config();
    r.check(
        "removing the config goes back to the defaults",
        c.wait(WAIT, |c| c.status().contains("reloaded") && !c.status().contains("line 2")),
    );
}

fn frame_time(bar: &str) -> bool {
    bar.split_whitespace().any(|w| w.strip_suffix("ms").is_some_and(|n| n.contains('.') && n.parse::<f64>().is_ok()))
}

fn events(c: &mut Client, ev: &mut Events, r: &mut Report) {
    c.send(LEADER);
    let pending = ev.wait(WAIT, |e| is(e, "key_pending") && e["keys"] == json!(["ctrl+space"]));
    c.send(ESC);
    let done = ev.wait(WAIT, |e| is(e, "key_done"));
    r.check("a paused leader sends key_pending with the leader, then key_done", pending && done);
    ev.wait(WAIT, |e| is(e, "mode_changed") && e["mode"] == "normal");
    let modes: Vec<&str> =
        ev.seen.iter().filter(|e| is(e, "mode_changed")).filter_map(|e| e["mode"].as_str()).collect();
    r.check_or("mode_changed reports the leader and the way back", modes.starts_with(&["leader", "normal"]), &modes);
    ev.clear();
    c.send(b"printf '\\a\\033]2;smoke-title\\033\\\\\\033]7;file://h/tmp\\007'\r");
    let ok = ev.wait(WAIT, |e| is(e, "bell"))
        && ev.wait(WAIT, |e| is(e, "title_changed") && e["title"] == "smoke-title")
        && ev.wait(WAIT, |e| is(e, "cwd_changed") && e["cwd"].as_str().is_some_and(|d| d.ends_with("/tmp")));
    r.check_or("bell, title_changed and cwd_changed reach subscribers", ok, (ev.names(), c.screen()));
    ev.clear();
    c.send(b"sleep 2.5; echo late\r");
    r.check_or(
        "output_started marks output after a quiet spell",
        ev.wait(Duration::from_secs(5), |e| is(e, "output_started")),
        ev.names(),
    );
}

fn panes(c: &mut Client, env: &Env, ev: &mut Events, r: &mut Report) {
    let listed = sessions(env);
    r.check(
        "shroud pane ls lists sessions as JSON",
        listed[0]["tabs"][0]["panes"][0]["id"].as_u64().is_some_and(|id| id > 0),
    );
    ev.clear();
    let out = env
        .command(&["pane", "run", "--wait", "printf '\\033]133;C\\007'; echo api-$((6*7)); (exit 3)"])
        .output()
        .expect("pane run");
    r.check(
        "pane run --wait exits with the command's code and prints its output",
        out.status.code() == Some(3) && String::from_utf8_lossy(&out.stdout).contains("api-42"),
    );
    ev.wait(WAIT, |e| is(e, "command_finished"));
    let opened = ev.seen.iter().find(|e| is(e, "pane_opened")).map(|e| e["pane"].clone()).unwrap_or(Value::Null);
    r.check(
        "events report the new pane and its finished command",
        !opened.is_null() && ev.seen.iter().any(|e| is(e, "command_finished") && e["exit"] == 3 && e["pane"] == opened),
    );

    let (code, fl) = env.cli(&["pane", "split", "--float"]);
    let fl = fl.trim().to_string();
    let floats: Vec<String> =
        panes_of(env).iter().filter(|p| p["float"] == true).map(|p| p["id"].to_string()).collect();
    r.check_or(
        "pane split --float opens a float and pane ls marks it",
        code == 0 && floats == [fl.clone()],
        (&floats, &fl),
    );
    let (code, scr) = env.cli(&["pane", "capture"]);
    let (code2, hist) = env.cli(&["pane", "capture", "--history"]);
    r.check(
        "pane capture prints the screen, and --history more of it",
        code == 0 && code2 == 0 && hist.len() >= scr.len() && !scr.is_empty(),
    );
    let long = env.cli(&["pane", "split"]).1.trim().to_string();
    env.cli(&["pane", "send", "-p", &long, "seq 1 3000", "-k", "enter"]);
    let kept = eventually(WAIT, || env.cli(&["pane", "capture", "-p", &long, "--history"]).1.contains("\n3000\n"));
    let hist = env.cli(&["pane", "capture", "-p", &long, "--history"]).1;
    r.check("a pane keeps thousands of lines of history by default", kept && hist.contains("\n1\n2\n3\n"));
    env.cli(&["pane", "close", "-p", &long]);
    env.cli(&["pane", "focus", "-p", &fl]);
    c.settle();
    c.send(&[LEADER, b"h"].concat());
    let tiled = c.wait(WAIT, |_| panes_of(env).iter().any(|p| p["focused"] == true && p["float"] == false));
    r.check("leader h from a lone float hides it and focuses a tile", tiled);
    let sl = env.cli(&["pane", "run", "sleep 30"]).1.trim().to_string();
    let seen = c.wait(WAIT, |_| panes_of(env).iter().any(|p| p["id"] == id(&sl) && p["process"] == "sleep 30"));
    r.check("pane ls reports a pane's foreground program", seen);
    env.cli(&["pane", "close", "-p", &sl]);
    let (code, popup) = env.cli(&["pane", "run", "--float", "--close", "true"]);
    let popup = popup.trim().to_string();
    let closed = c.wait(WAIT, |_| !panes_of(env).iter().any(|p| p["id"] == id(&popup)));
    r.check("pane run --close closes the pane when its command exits", code == 0 && !popup.is_empty() && closed);
    let (code, _) = env.cli(&["command", "zoom"]);
    let zoomed = c.wait(WAIT, |c| c.status().contains("ZOOM"));
    let (code2, _) = env.cli(&["command", "zoom"]);
    c.wait(WAIT, |c| !c.status().contains("ZOOM"));
    let (bad, _) = env.cli(&["command", "no-such-thing"]);
    r.check(
        "shroud command runs a key's command, and refuses unknown ones",
        code == 0 && code2 == 0 && zoomed && bad != 0,
    );
    env.cli(&["pane", "close", "-p", &fl]);

    ev.clear();
    let (code, new) = env.cli(&["pane", "split", "--down"]);
    let new = new.trim().to_string();
    c.send(&[LEADER, b"="].concat());
    env.cli(&["pane", "send", "-p", &new, "echo sent-$((1+1))", "-k", "enter"]);
    r.check("pane split opens a pane and pane send types into it", code == 0 && c.wait_screen("sent-2"));
    env.cli(&["pane", "focus", "-p", &new]);
    let ok = ev.wait(WAIT, |e| is(e, "layout_changed"))
        && ev.wait(WAIT, |e| is(e, "focus_changed") && e["pane"] == id(&new));
    r.check_or("a split and a focus change are reported", ok, ev.names());
    ev.clear();
    let (code, _) = env.cli(&["pane", "close", "-p", &new]);
    env.cli(&["pane", "close", "-p", &opened.to_string()]);
    let ok = ev.wait(WAIT, |e| is(e, "pane_closed") && e["pane"] == id(&new))
        && ev.wait(WAIT, |e| is(e, "pane_closed") && e["pane"] == opened);
    r.check("pane close closes it and the event stream says so", code == 0 && ok);
}

fn control(c: &mut Client, env: &Env, ev: &mut Events, r: &mut Report) {
    let mut ctl = Ctl::connect(env);
    ctl.send(json!({"id": 1, "cmd": "overlay_open", "title": "smoke-hints", "lines": ["press a key", [{"text": "x", "style": "badge.done"}]], "modal": true, "anchor": "bottom_right"}));
    let drawn = c.wait(WAIT, |c| c.screen().contains("smoke-hints") && c.screen().contains("press a key"));
    let ov = ctl.wait(WAIT, |l| l["id"] == 1).map(|l| l["overlay"].clone()).unwrap_or(Value::Null);
    r.check("overlay_open draws a titled box over the panes", !ov.is_null() && drawn);
    c.send(b"q");
    r.check(
        "a modal overlay sends keys to its owner, not the pane",
        ctl.event(WAIT, |e| is(e, "overlay_key") && e["key"] == "q").is_some(),
    );
    c.send(ESC);
    r.check(
        "escape always closes a modal overlay and tells the owner",
        ctl.event(WAIT, |e| is(e, "overlay_closed") && e["overlay"] == ov).is_some(),
    );

    let ok = ctl.reply(json!({"id": 10, "cmd": "register_command", "name": "smoke.hello", "key": "g h"}));
    let clash = ctl.reply(json!({"id": 11, "cmd": "register_command", "name": "smoke.clash", "key": "v"}));
    r.check_or(
        "a plugin command takes a free default key; a taken one is reported, not stolen",
        ok["ok"] == true
            && ok.get("key_conflict").is_none()
            && clash["key_conflict"].as_str().is_some_and(|s| s.contains("already in use")),
        (&ok, &clash),
    );
    let binds = ctl.reply(json!({"id": 12, "cmd": "bindings"}));
    let after = binds["after_leader"].as_array().cloned().unwrap_or_default();
    r.check(
        "bindings lists built-in and plugin keys",
        after.contains(&json!({"keys": "g h", "command": "smoke.hello"}))
            && after.contains(&json!({"keys": "v", "command": "vsplit"})),
    );
    c.send(&[LEADER, b"03gh"].concat());
    r.check(
        "its key sends the plugin a command event with the count",
        ctl.event(WAIT, |e| is(e, "command") && e["name"] == "smoke.hello" && e["count"] == 3).is_some(),
    );
    ctl.send(json!({"id": 2, "cmd": "overlay_open", "title": "orphan", "lines": ["bye"]}));
    c.wait_screen("orphan");
    drop(ctl);
    r.check("an overlay vanishes when its owner disconnects", c.wait_gone("orphan"));
    c.send(&[LEADER, b"gh"].concat());
    r.check(
        "a plugin key whose plugin has gone says so",
        c.wait(WAIT, |c| c.status().contains("no plugin provides smoke.hello")),
    );

    let mut ctl = Ctl::connect(env);
    let here = ctl.reply(json!({"id": 20, "cmd": "layout"}));
    r.check("layout reports the active tab in the layout-file model", here["layout"].get("root").is_some());
    let new = ctl.reply(json!({"id": 23, "cmd": "session_new", "name": "scripted", "cwd": "/tmp"}));
    let spec = json!({"root": {"split": "right", "parts": [{"cwd": "/tmp", "size": 30}, {"split": "down", "parts": [{}, {}]}]}, "floats": [{"w": 50, "h": 40}]});
    let made = ctl.reply(json!({"id": 21, "cmd": "apply_layout", "session": "scripted", "layout": spec}));
    let tab = made["tab"].clone();
    let back = ctl.reply(json!({"id": 22, "cmd": "layout", "session": "scripted", "tab": tab}))["layout"].clone();
    let root = &back["root"];
    let shaped = made["ok"] == true
        && root["split"] == "right"
        && root["parts"].as_array().map(Vec::len) == Some(2)
        && root["parts"][0]["size"] == 30.0
        && root["parts"][1]["split"] == "down"
        && back["floats"][0]["w"] == 50
        && back.get("floats_hidden").is_none();
    r.check_or("apply_layout builds a tab that reads back the same shape", shaped, (&made, &back));
    let (pa, pb, pc) = (
        root["parts"][0]["pane"].clone(),
        root["parts"][1]["parts"][0]["pane"].clone(),
        root["parts"][1]["parts"][1]["pane"].clone(),
    );
    let pf = back["floats"][0]["pane"].clone();
    let re_spec = json!({"root": {"split": "down", "parts": [{"pane": pc}, {"pane": pa}]}, "floats": [{"pane": pb, "w": 30, "h": 30}]});
    let moved = ctl.reply(json!({"id": 28, "cmd": "set_layout", "session": "scripted", "tab": tab, "layout": re_spec}));
    let after = ctl.reply(json!({"id": 29, "cmd": "layout", "session": "scripted", "tab": tab}))["layout"].clone();
    let tiles: Vec<Value> =
        after["root"]["parts"].as_array().map(|p| p.iter().map(|x| x["pane"].clone()).collect()).unwrap_or_default();
    let floated: Vec<(Value, Value)> = after["floats"]
        .as_array()
        .map(|f| f.iter().map(|x| (x["pane"].clone(), x["w"].clone())).collect())
        .unwrap_or_default();
    let want_floats = [(pf.clone(), json!(50)), (pb.clone(), json!(30))];
    r.check_or(
        "set_layout moves running panes between tiles and floats",
        moved["ok"] == true
            && after["root"]["split"] == "down"
            && tiles == [pc.clone(), pa.clone()]
            && want_floats.iter().all(|w| floated.contains(w))
            && floated.len() == 2,
        (&moved, &after),
    );
    let bad = ctl.reply(
        json!({"id": 31, "cmd": "set_layout", "session": "scripted", "tab": tab, "layout": {"root": {"pane": pc}}}),
    );
    r.check_or(
        "set_layout refuses to leave a tile out",
        bad["error"].as_str().is_some_and(|e| e.contains("leaves out pane")),
        &bad,
    );
    ev.clear();
    let tagged = ctl.reply(json!({"id": 32, "cmd": "tag", "pane": pa, "key": "smoke.role", "value": "editor"}));
    let listed = sessions(env)
        .as_array()
        .and_then(|s| s.iter().find(|x| x["name"] == "scripted").cloned())
        .unwrap_or(Value::Null);
    let tags_seen: Vec<Value> = listed["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| t["panes"].as_array().cloned().unwrap_or_default())
        .filter(|p| p["id"] == pa)
        .map(|p| p["tags"].clone())
        .collect();
    let laid = ctl.reply(json!({"id": 33, "cmd": "layout", "session": "scripted", "tab": tab}))["layout"].clone();
    let copy = ctl.reply(json!({"id": 34, "cmd": "apply_layout", "session": "scripted", "layout": laid}));
    let copied =
        ctl.reply(json!({"id": 35, "cmd": "layout", "session": "scripted", "tab": copy["tab"]}))["layout"].clone();
    let copied_tags: Vec<Value> =
        copied["root"]["parts"].as_array().map(|p| p.iter().map(|x| x["tags"].clone()).collect()).unwrap_or_default();
    let role = json!({"smoke.role": "editor"});
    let tag_event = ev.wait(WAIT, |e| is(e, "tag_changed") && e["value"] == "editor");
    r.check_or(
        "tags show in list and layout, survive apply_layout, and send tag_changed",
        tagged["ok"] == true && tags_seen == [role.clone()] && copied_tags.contains(&role) && tag_event,
        (&tagged, &tags_seen, &copied_tags),
    );
    let bad_tag = ctl.reply(json!({"id": 36, "cmd": "tag", "pane": pa, "key": "Bad Key", "value": "x"}));
    r.check_or(
        "a bad tag name is refused",
        bad_tag["error"].as_str().is_some_and(|e| e.contains("isn't a tag name")),
        &bad_tag,
    );

    let lone = ctl.reply(json!({"id": 26, "cmd": "session_new", "name": "lone", "cwd": "/tmp"}));
    let lone_panes = || {
        sessions(env)
            .as_array()
            .and_then(|s| s.iter().find(|x| x["name"] == "lone").cloned())
            .map(|s| s["tabs"][0]["panes"].as_array().cloned().unwrap_or_default())
            .unwrap_or_default()
    };
    let tile = lone_panes().first().map(|p| p["id"].to_string()).unwrap_or_default();
    env.cli(&["pane", "split", "--float", "-p", &tile]);
    env.cli(&["pane", "close", "-p", &tile]);
    let promoted =
        eventually(WAIT, || matches!(lone_panes().as_slice(), [p] if p["float"] == false && p["focused"] == true));
    r.check_or("closing a tab's last tile turns its float into the tile", lone["ok"] == true && promoted, lone_panes());
    ctl.reply(json!({"id": 27, "cmd": "session_kill", "session": "lone"}));
    let hidden_spec = json!({"root": {}, "floats": [{}], "floats_hidden": true});
    let hidden = ctl.reply(json!({"id": 37, "cmd": "apply_layout", "session": "scripted", "layout": hidden_spec}));
    let hidden_back =
        ctl.reply(json!({"id": 38, "cmd": "layout", "session": "scripted", "tab": hidden["tab"]}))["layout"].clone();
    r.check_or(
        "a layout keeps whether its floats are shown",
        hidden_back["floats_hidden"] == true,
        (&hidden, &hidden_back),
    );
    let ren = ctl.reply(json!({"id": 24, "cmd": "session_rename", "session": "scripted", "to": "scripted2"}));
    let listing = env.cli(&["session", "ls"]).1;
    let kill = ctl.reply(json!({"id": 25, "cmd": "session_kill", "session": "scripted2"}));
    let after = env.cli(&["session", "ls"]).1;
    r.check(
        "sessions can be created, renamed and killed over the socket",
        new["ok"] == true
            && ren["ok"] == true
            && listing.contains("scripted2")
            && kill["ok"] == true
            && !after.contains("scripted2"),
    );

    let took = ctl.reply(json!({"id": 30, "cmd": "status_take"}));
    c.send(LEADER);
    let leader = ctl.event(WAIT, |e| is(e, "status") && e["mode"] == "leader");
    c.send(ESC);
    let state = leader.clone().unwrap_or(Value::Null);
    r.check_or(
        "a plugin that takes the bar hears each client's state as it changes",
        took["ok"] == true
            && leader.is_some()
            && state["session"].is_string()
            && state["width"].as_u64().unwrap_or(0) > 0,
        &state,
    );
    ctl.reply(json!({"id": 31, "cmd": "status_set", "client": state["client"], "left": [{"text": " smokebar ", "bg": "#82aaff", "style": "0", "bold": true}], "right": [{"text": "far-right", "italic": true}]}));
    let at = c.mark();
    let drawn = c.wait(WAIT, |c| c.status().contains("smokebar") && c.status().contains("far-right"));
    r.check(
        "status_set draws the plugin's bar, colours included",
        drawn && contains(&c.raw[at.saturating_sub(4096)..], b"48;2;130;170;255"),
    );
    drop(ctl);
    r.check(
        "the built-in bar returns when the bar's plugin goes away",
        c.wait(WAIT, |c| !c.status().contains("smokebar") && c.status().contains("default")),
    );

    let mut ctl = Ctl::connect(env);
    ctl.reply(json!({"id": 32, "cmd": "overlay_open", "client": 999_999, "lines": ["not-for-you"]}));
    c.settle();
    r.check("an overlay for another client isn't drawn here", !c.screen().contains("not-for-you"));
    drop(ctl);
    let (code, msg) = env.cli(&["pane", "send", "-p", "9999", "x"]);
    r.check_or("a request for a missing pane fails with a reason", code == 1 && msg.contains("no pane 9999"), msg);
    c.send(ESC);
}

fn frames(c: &mut Client, env: &Env, r: &mut Report) {
    env.write_config("pane_frames = true\n");
    r.check_or(
        "pane_frames puts the tiles inside a frame",
        c.wait(WAIT, |c| c.rows()[0].starts_with('┌') && c.rows()[22].starts_with('└')),
        c.screen(),
    );
    let mut ctl = Ctl::connect(env);
    let set = ctl.reply(json!({"id": 50, "cmd": "pane_title", "title": "smoke-title"}));
    r.check_or(
        "pane_title names the pane on its border",
        set["ok"] == true && c.wait(WAIT, |c| c.rows()[0].contains(" smoke-title ")),
        c.rows()[0].clone(),
    );
    env.remove_config();
    r.check("turning pane_frames off gives the room back", c.wait(WAIT, |c| !c.rows()[0].starts_with('┌')));
}
