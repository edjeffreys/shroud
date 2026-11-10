//! Lua plugins in the plugin host: commands, the watchdog, git installs, init.lua, keymaps, the
//! built-in hints and sessions.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::json;

use crate::ctl::Ctl;
use crate::harness::{Client, ESC, Env, LEADER, Report, WAIT, has_clock};

pub fn run(env: &Env, r: &mut Report) {
    let mut c = start(env);
    local(&mut c, env, r);
    builtin(&mut c, env, r);
}

/// Git sources and init.lua, apart from `run` so the two halves run at once.
pub fn run_sources(env: &Env, r: &mut Report) {
    let mut c = start(env);
    git(&mut c, env, r);
    init_lua(&mut c, env, r);
}

fn start(env: &Env) -> Client {
    let mut c = env.spawn(None, &[]);
    c.wait(WAIT, |c| has_clock(&c.status()));
    c.ready();
    c
}

/// The host restarts after a config change, so a plugin's key may need pressing until it's back.
fn press_until(c: &mut Client, keys: &[u8], cond: impl Fn(&Client) -> bool) -> bool {
    for _ in 0..8 {
        c.send(keys);
        if c.wait_quietly(Duration::from_millis(800), &cond) {
            return true;
        }
    }
    false
}

fn make_plugin(env: &Env, name: &str, manifest: &str, init: &str) -> PathBuf {
    let dir = env.tmp.join("plugins").join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("plugin.toml"), format!("name = \"{name}\"\nversion = \"0.1.0\"\napi = \"0.1\"\n{manifest}"))
        .unwrap();
    fs::write(dir.join("init.lua"), init).unwrap();
    dir
}

fn local(c: &mut Client, env: &Env, r: &mut Report) {
    let hello = make_plugin(
        env,
        "hello",
        "[commands]\ngreet = { key = \"g y\" }\nslow = { key = \"g u\" }\n",
        r#"
local open
shroud.command("slow", function()
  local r = shroud.system({"/bin/sh", "-c", "sleep 0.5; echo sys-$((40+2))"}):wait()
  shroud.overlay_open{lines = {r.stdout}, modal = true}
end)
shroud.command("greet", function(ev)
  open = shroud.overlay_open{title = "lua", lines = {opts_word .. " from lua x" .. (ev.count or 1)}, modal = true}
end)
shroud.on("overlay_key", function(ev) shroud.overlay_close{overlay = ev.overlay} end)
return { setup = function(opts) opts_word = opts.word end }
"#,
    );
    let stuck = make_plugin(
        env,
        "stuck",
        "[commands]\nspin = { key = \"g z\" }\n",
        "shroud.command(\"spin\", function() while true do end end)\nreturn {}\n",
    );
    env.write_config(&format!(
        "[[plugins]]\nsrc = \"path:{}\"\n[plugins.opts]\nword = \"hello\"\n[[plugins]]\nsrc = \"path:{}\"\n",
        hello.display(),
        stuck.display()
    ));
    let greeted = press_until(c, &[LEADER, b"gy"].concat(), |c| c.screen().contains("hello from lua x1"));
    r.check("a Lua plugin's command opens its overlay, with setup's opts", greeted);
    c.send(b"q");
    c.wait_gone("from lua");

    c.send(&[LEADER, b"gu"].concat());
    c.idle(Duration::from_millis(100));
    c.send(&[LEADER, b"gy"].concat());
    let early =
        c.wait(Duration::from_millis(400), |c| c.screen().contains("hello from lua")) && !c.screen().contains("sys-42");
    c.send(b"q");
    let late = c.wait_screen("sys-42");
    r.check("shroud.system waits without blocking other plugin commands", early && late);
    c.send(b"q");
    r.check(
        "its overlay_key handler closes the overlay",
        c.wait(WAIT, |c| !c.screen().contains("sys-42") && !c.screen().contains("from lua")),
    );
    c.send(&[LEADER, b"gz"].concat());
    r.check(
        "the watchdog stops a plugin stuck in a loop and the bar says plugin",
        c.wait(Duration::from_secs(6), |c| c.status().contains("plugin stuck stopped")),
    );
    let restarted = press_until(c, &[LEADER, b"02gy"].concat(), |c| c.screen().contains("hello from lua x2"));
    r.check("the restarted host still runs the other plugins", restarted);
    c.send(b"q");
    c.wait_gone("from lua");
    c.send(&[LEADER, b"gz"].concat());
    r.check(
        "the stuck plugin stays off until the config changes",
        c.wait(WAIT, |c| c.status().contains("no plugin provides")),
    );
    env.remove_config();
    c.wait(WAIT, |c| c.status().contains("reloaded"));
}

fn git_in(env: &Env, repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=smoke", "-c", "user.email=smoke@example.com"])
        .args(args)
        .current_dir(repo)
        .env("HOME", &env.tmp)
        .output()
        .expect("git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn greeter(env: &Env, repo: &Path, word: &str) -> String {
    fs::write(
        repo.join("plugin.toml"),
        "name = \"greeter\"\nversion = \"0.1.0\"\napi = \"0.1\"\n[commands]\nwave = { key = \"g u\" }\n",
    )
    .unwrap();
    fs::write(repo.join("init.lua"), format!("shroud.command(\"wave\", function() shroud.overlay_open{{lines = {{\"{word}\"}}, modal = true}} end)\nshroud.on(\"overlay_key\", function(ev) shroud.overlay_close{{overlay = ev.overlay}} end)\nreturn {{}}\n")).unwrap();
    git_in(env, repo, &["add", "-A"]);
    git_in(env, repo, &["commit", "-q", "-m", word]);
    git_in(env, repo, &["rev-parse", "HEAD"])
}

fn plugin_cli(env: &Env, args: &[&str]) -> (i32, String, String) {
    let out = env.command(&[&["plugin"], args].concat()).output().expect("shroud plugin");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn git(c: &mut Client, env: &Env, r: &mut Report) {
    let repo = env.tmp.join("repos/greeter");
    fs::create_dir_all(&repo).unwrap();
    git_in(env, &repo, &["init", "-q"]);
    let first = greeter(env, &repo, "wave-one");
    env.write_config(&format!("[[plugins]]\nsrc = \"git:{}\"\n", repo.display()));
    c.wait(WAIT, |c| c.status().contains("reloaded"));
    let (code, out, err) = plugin_cli(env, &["sync"]);
    let lock_path = env.config.with_file_name("plugins.lock");
    let lock = fs::read_to_string(&lock_path).unwrap_or_default();
    r.check_or(
        "plugin sync clones a git plugin, pins its commit and restarts the host",
        code == 0 && out.contains("greeter: installed") && lock.contains(&first) && out.contains("restarted"),
        (out, err),
    );
    r.check("the synced plugin runs", press_until(c, &[LEADER, b"gu"].concat(), |c| c.screen().contains("wave-one")));
    c.send(b"q");
    c.wait_gone("wave-one");
    let second = greeter(env, &repo, "wave-two");
    let (code, out, err) = plugin_cli(env, &["sync"]);
    r.check_or(
        "sync keeps the pin when upstream moves",
        code == 0 && out.contains("up to date") && fs::read_to_string(&lock_path).unwrap_or_default().contains(&first),
        (out, err),
    );
    let (code, out, err) = plugin_cli(env, &["update"]);
    r.check_or(
        "update moves to the new commit and relocks",
        code == 0
            && out.contains("greeter: updated")
            && fs::read_to_string(&lock_path).unwrap_or_default().contains(&second),
        (out, err),
    );
    r.check(
        "the restarted host runs the updated plugin",
        press_until(c, &[LEADER, b"gu"].concat(), |c| c.screen().contains("wave-two")),
    );
    c.send(b"q");
    c.wait_gone("wave-two");
    let (_, out, err) = plugin_cli(env, &["ls"]);
    r.check_or(
        "plugin ls shows the plugin at its locked commit",
        out.contains("greeter") && out.contains(&format!("at {}", &second[..7.min(second.len())])),
        (out, err),
    );
    let installs = env.tmp.join("data/shroud/plugins");
    fs::create_dir_all(installs.join("stray")).unwrap();
    let (_, out, _) = plugin_cli(env, &["clean"]);
    let kept: Vec<String> = fs::read_dir(&installs)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    r.check_or(
        "plugin clean removes what config no longer lists",
        out.contains("removed stray") && kept == ["greeter"],
        (out, kept),
    );
}

fn init_lua(c: &mut Client, env: &Env, r: &mut Report) {
    let luacfg = make_plugin(
        env,
        "luacfg",
        "[commands]\nshow = { key = \"g k\" }\n",
        "local o\nshroud.command(\"show\", function() shroud.overlay_open{lines = {o.text()}, modal = true} end)\nshroud.on(\"overlay_key\", function(ev) shroud.overlay_close{overlay = ev.overlay} end)\nreturn { setup = function(opts) o = opts end }\n",
    );
    let init = env.config.with_file_name("init.lua");
    fs::write(&init, format!("shroud.plugins {{\n  {{ \"path:{}\", opts = {{ text = function() return \"from-init-\" .. (1 + 1) end }} }},\n}}\n", luacfg.display())).unwrap();
    r.check(
        "init.lua declares a plugin and hands it opts with functions",
        press_until(c, &[LEADER, b"gk"].concat(), |c| c.screen().contains("from-init-2")),
    );
    c.send(b"q");
    c.wait_gone("from-init-2");
    let (_, out, err) = plugin_cli(env, &["ls"]);
    r.check_or(
        "plugin ls lists what init.lua declares",
        out.contains("luacfg") && out.contains(&format!("path:{}", luacfg.display())),
        (out, err),
    );
    fs::write(
        &init,
        format!(
            "shroud.plugins {{ {{ \"path:{}\", opts = {{ text = function() return \"x\" end }} }} }}\nshroud.on(\"overlay_key\", function(ev) shroud.overlay_close{{overlay = ev.overlay}} end)\nshroud.keymap(\"g m\", function() shroud.overlay_open{{lines = {{\"mapped-\" .. 6 * 7}}, modal = true}} end)\nshroud.keymap(\"g k\", function() shroud.overlay_open{{lines = {{\"over-\" .. 3 + 4}}, modal = true}} end)\n",
            luacfg.display()
        ),
    )
    .unwrap();
    r.check(
        "shroud.keymap binds a key to a Lua function",
        press_until(c, &[LEADER, b"gm"].concat(), |c| c.screen().contains("mapped-42")),
    );
    c.send(b"q");
    c.wait_gone("mapped-42");
    r.check(
        "shroud.keymap takes a key over from a plugin's default",
        press_until(c, &[LEADER, b"gk"].concat(), |c| c.screen().contains("over-7")),
    );
    c.send(b"q");
    c.wait_gone("over-7");
    fs::write(&init, "while true do end\n").unwrap();
    let (code, out, err) = plugin_cli(env, &["ls"]);
    r.check_or(
        "plugin ls gives up on an init.lua that never returns",
        code == 1 && err.contains("took over a second"),
        (out, err),
    );
    let _ = fs::remove_file(&init);
}

fn builtin(c: &mut Client, env: &Env, r: &mut Report) {
    let hinted = (0..6).any(|_| {
        c.send(LEADER);
        let shown = c.wait_quietly(Duration::from_millis(1500), |c| {
            let s = c.screen();
            s.contains("vsplit") && s.contains("1-9") && s.contains("goto-tab")
        });
        if !shown {
            c.send(ESC);
            c.settle();
        }
        shown
    });
    r.check_or("after a pause, the built-in hints show what each key does", hinted, c.screen());
    c.send(b"g");
    r.check("hints follow the next key at once", c.wait_screen("next-tab"));
    c.send(ESC);
    r.check("hints close when the sequence ends", c.wait_gone("next-tab"));

    // A long debounce, so killing the session below doesn't save over it before the restore.
    env.write_config("[[plugins]]\nsrc = \"builtin:sessions\"\n[plugins.opts]\ndebounce_ms = 60000\n[[plugins]]\nsrc = \"builtin:hints\"\nenabled = false\n");
    c.wait(WAIT, |c| c.status().contains("reloaded"));
    let mut ctl = Ctl::connect(env);
    let made = ctl.reply(json!({"id": 40, "cmd": "session_new", "name": "keepme", "cwd": "/tmp"}));
    let saved = env.tmp.join("data/shroud/data/sessions/sessions.json");
    let said = press_until(c, &[LEADER, b"gs"].concat(), |c| c.status().contains("saved"));
    r.check(
        "sessions.save writes every session's layout",
        made["ok"] == true && said && fs::read_to_string(&saved).is_ok_and(|s| s.contains("\"keepme\"")),
    );
    ctl.reply(json!({"id": 41, "cmd": "session_kill", "session": "keepme"}));
    c.wait(WAIT, |_| !env.cli(&["session", "ls"]).1.contains("keepme"));
    c.send(&[LEADER, b"gr"].concat());
    let restored = c.wait(WAIT, |c| c.status().contains("restored 1 session"));
    let listing = env.cli(&["session", "ls"]).1;
    r.check_or(
        "sessions.restore reopens a saved session that isn't running",
        restored && listing.contains("keepme"),
        (c.status(), listing),
    );
    c.send(LEADER);
    c.idle(Duration::from_millis(800));
    r.check("hints stay off when config turns them off", !c.screen().contains("vsplit"));
    c.send(ESC);
    ctl.reply(json!({"id": 42, "cmd": "session_kill", "session": "keepme"}));
    env.remove_config();
}
