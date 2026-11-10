//! NDJSON on `control.sock`: plain lines, so a shell script can't get the framing wrong.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

// Bounds a runaway client's buffer.
pub const MAX_LINE: usize = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    #[default]
    Right,
    Down,
}

/// Percentages of the tab; omitted fields take the centred default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FloatSpec {
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
}

impl Default for FloatSpec {
    fn default() -> FloatSpec {
        FloatSpec { x: 10, y: 10, w: 80, h: 80 }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Input {
    Text(String),
    Key(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSpec {
    #[default]
    Centre,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Cursor,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpanSpec {
    pub text: String,
    /// A theme token or a colour value, for the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
}

/// A plain string is one unstyled span.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum LineSpec {
    Text(String),
    Spans(Vec<SpanSpec>),
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub lines: Vec<LineSpec>,
    #[serde(default)]
    pub anchor: AnchorSpec,
    #[serde(default)]
    pub modal: bool,
}

/// One tab's panes, in the same model as layout files. `size` is a percentage of the
/// parent split; `pane` and `title` are only ever reported, never read.
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutSpec {
    pub root: LayoutNode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floats: Vec<FloatNode>,
}

#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum LayoutNode {
    Split(SplitNode),
    Pane(PaneNode),
}

#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SplitNode {
    pub split: Dir,
    pub parts: Vec<LayoutNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
}

#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PaneNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Reported only: the foreground program, when it isn't the shell. Restoring it is the
    /// caller's choice; `command` would run it unasked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tags: super::tags::Tags,
}

#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FloatNode {
    #[serde(flatten)]
    pub at: FloatSpec,
    #[serde(flatten)]
    pub pane: PaneNode,
}

/// What `run` and `bind` take: a command name, or a shortcut table such as `{"run": "nvim"}`,
/// checked by the server, since shortcuts are an input concern.
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Target {
    Name(String),
    Shortcut(Map<String, Value>),
}

#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    List,
    Split {
        pane: Option<u32>,
        #[serde(default)]
        dir: Dir,
        cwd: Option<String>,
        command: Option<String>,
        #[serde(default)]
        focus: bool,
        #[serde(default)]
        wait: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        float: Option<FloatSpec>,
        /// The pane closes when `command` exits, as a popup should.
        #[serde(default, skip_serializing_if = "is_false")]
        close_on_exit: bool,
        /// A command pane: when `command` ends, Enter runs it again and `q` closes the pane.
        #[serde(default, skip_serializing_if = "is_false")]
        keep: bool,
    },
    Send {
        pane: Option<u32>,
        input: Vec<Input>,
    },
    Output {
        pane: Option<u32>,
    },
    /// The pane's text: the screen, or with `history` everything kept.
    Capture {
        pane: Option<u32>,
        #[serde(default, skip_serializing_if = "is_false")]
        history: bool,
    },
    /// Any command a key can be bound to, such as `zoom` or `goto-tab 3`, as if `client`
    /// pressed it; without `client`, a client on `session` (default the most recent).
    Run {
        command: Target,
        client: Option<u64>,
        session: Option<String>,
    },
    Close {
        pane: Option<u32>,
    },
    Focus {
        pane: Option<u32>,
    },
    Subscribe,
    /// The session defaults to the most recently used one. With `client`, only that client
    /// sees it.
    OverlayOpen {
        session: Option<String>,
        client: Option<u64>,
        #[serde(flatten)]
        spec: OverlaySpec,
    },
    OverlayUpdate {
        overlay: u32,
        #[serde(flatten)]
        spec: OverlaySpec,
    },
    OverlayClose {
        overlay: u32,
    },
    /// `key` is a leader sequence such as "g s", used only if it's free.
    RegisterCommand {
        name: String,
        key: Option<String>,
    },
    /// Binds `keys` (a leader sequence, or with `direct` one key that skips the leader) to a
    /// command, over config and plugin defaults, until this connection closes. No `command`
    /// unbinds.
    Bind {
        keys: String,
        command: Option<Target>,
        #[serde(default)]
        direct: bool,
    },
    Bindings,
    /// `tab` is 1-based and defaults to the active one.
    Layout {
        session: Option<String>,
        tab: Option<usize>,
    },
    /// Opens a new tab built from the layout.
    ApplyLayout {
        session: Option<String>,
        layout: LayoutSpec,
    },
    /// Shown above the pane when `pane_frames` is on; no `title` goes back to the program's.
    PaneTitle {
        pane: Option<u32>,
        title: Option<String>,
    },
    /// No `value` removes the tag.
    Tag {
        pane: Option<u32>,
        key: String,
        value: Option<String>,
    },
    /// Rearranges an existing tab. A pane node with `pane` moves that pane; one without starts
    /// a new one. Every tile must be placed; floats left out stay.
    SetLayout {
        session: Option<String>,
        /// 1-based, default the active one.
        tab: Option<usize>,
        layout: LayoutSpec,
    },
    SessionNew {
        name: Option<String>,
        cwd: Option<String>,
    },
    SessionKill {
        session: String,
    },
    SessionRename {
        session: Option<String>,
        to: String,
    },
    TabNew {
        session: Option<String>,
    },
    /// Shown where the status bar puts messages, for a few seconds.
    Message {
        session: Option<String>,
        text: String,
    },
    /// The plugin host's first line. The token proves the server spawned it; the reply lists
    /// the plugins to load.
    PluginHello {
        token: String,
    },
    /// Sent before and after each Lua handler, so the server knows whom to blame for a hang.
    /// Gets no reply: the host must not wait on the server before running Lua.
    PluginRunning {
        plugin: Option<String>,
    },
    PluginError {
        plugin: String,
        message: String,
    },
    /// This connection draws the status bar from now on. It gets a `status` event whenever a
    /// client's bar would change, and answers with `status_set`.
    StatusTake,
    StatusRelease,
    StatusSet {
        client: u64,
        #[serde(default)]
        left: Vec<SpanSpec>,
        #[serde(default)]
        right: Vec<SpanSpec>,
        /// The background between and behind the spans: a theme token or colour.
        #[serde(default)]
        fill: Option<String>,
    },
    /// After `plugin sync`: restart the host and forgive every plugin, as a config change does.
    PluginsReload,
    PluginStatus,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    CommandFinished {
        pane: u32,
        exit: Option<i32>,
    },
    PaneOpened {
        pane: u32,
        session: String,
    },
    PaneClosed {
        pane: u32,
    },
    /// Only to the overlay's owner, whether or not it subscribed.
    OverlayKey {
        overlay: u32,
        key: String,
    },
    OverlayClosed {
        overlay: u32,
    },
    FocusChanged {
        session: String,
        pane: u32,
    },
    SessionOpened {
        session: String,
    },
    SessionClosed {
        session: String,
    },
    SessionRenamed {
        from: String,
        to: String,
    },
    /// 1-based, as the status bar numbers tabs.
    TabChanged {
        session: String,
        tab: usize,
    },
    LayoutChanged {
        session: String,
        tab: usize,
    },
    ClientAttached {
        client: u64,
        session: String,
    },
    ClientDetached {
        client: u64,
    },
    /// Keys are chords as config writes them, the leader first.
    KeyPending {
        client: u64,
        session: String,
        keys: Vec<String>,
    },
    KeyDone {
        client: u64,
    },
    /// Only to the plugin that registered the command.
    Command {
        name: String,
        client: u64,
        session: String,
        count: Option<u16>,
    },
    /// `value` is absent when the tag was removed.
    TagChanged {
        pane: u32,
        key: String,
        value: Option<String>,
    },
    Bell {
        pane: u32,
    },
    TitleChanged {
        pane: u32,
        title: String,
    },
    CwdChanged {
        pane: u32,
        cwd: String,
    },
    /// A pane printed after `QUIET` with no output, other than echoing a key.
    OutputStarted {
        pane: u32,
    },
    /// `normal`, `leader`, `jump` or `visual`, as the status bar's state names them.
    ModeChanged {
        client: u64,
        mode: String,
    },
    /// Only to the connection drawing the status bar: what that client's bar should show.
    Status {
        client: u64,
        state: Value,
    },
}

/// The id survives a bad request, so its error reaches the right caller.
pub fn parse_request(line: &str) -> (Option<Value>, Result<Request, String>) {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return (None, Err(format!("not JSON: {e}"))),
    };
    let Value::Object(mut obj) = value else { return (None, Err("a request is a JSON object".into())) };
    let id = obj.remove("id");
    let request = serde_json::from_value(Value::Object(obj)).map_err(|e| e.to_string());
    (id, request)
}

pub fn encode_request(id: Option<u64>, request: &Request) -> String {
    let mut v = serde_json::to_value(request).expect("requests serialize");
    if let (Some(id), Value::Object(obj)) = (id, &mut v) {
        obj.insert("id".into(), id.into());
    }
    line(v)
}

pub fn ok(id: Option<&Value>, fields: Value) -> String {
    let mut obj = Map::new();
    if let Some(id) = id {
        obj.insert("id".into(), id.clone());
    }
    obj.insert("ok".into(), true.into());
    if let Value::Object(f) = fields {
        obj.extend(f);
    }
    line(Value::Object(obj))
}

pub fn error(id: Option<&Value>, message: &str) -> String {
    let mut obj = Map::new();
    if let Some(id) = id {
        obj.insert("id".into(), id.clone());
    }
    obj.insert("ok".into(), false.into());
    obj.insert("error".into(), message.into());
    line(Value::Object(obj))
}

pub fn event(e: &Event) -> String {
    line(match e {
        Event::CommandFinished { pane, exit } => json!({"event": "command_finished", "pane": pane, "exit": exit}),
        Event::PaneOpened { pane, session } => json!({"event": "pane_opened", "pane": pane, "session": session}),
        Event::PaneClosed { pane } => json!({"event": "pane_closed", "pane": pane}),
        Event::OverlayKey { overlay, key } => json!({"event": "overlay_key", "overlay": overlay, "key": key}),
        Event::OverlayClosed { overlay } => json!({"event": "overlay_closed", "overlay": overlay}),
        Event::FocusChanged { session, pane } => json!({"event": "focus_changed", "session": session, "pane": pane}),
        Event::SessionOpened { session } => json!({"event": "session_opened", "session": session}),
        Event::SessionClosed { session } => json!({"event": "session_closed", "session": session}),
        Event::SessionRenamed { from, to } => json!({"event": "session_renamed", "from": from, "to": to}),
        Event::TabChanged { session, tab } => json!({"event": "tab_changed", "session": session, "tab": tab}),
        Event::LayoutChanged { session, tab } => json!({"event": "layout_changed", "session": session, "tab": tab}),
        Event::ClientAttached { client, session } => {
            json!({"event": "client_attached", "client": client, "session": session})
        }
        Event::ClientDetached { client } => json!({"event": "client_detached", "client": client}),
        Event::KeyPending { client, session, keys } => {
            json!({"event": "key_pending", "client": client, "session": session, "keys": keys})
        }
        Event::KeyDone { client } => json!({"event": "key_done", "client": client}),
        Event::Command { name, client, session, count } => {
            json!({"event": "command", "name": name, "client": client, "session": session, "count": count})
        }
        Event::TagChanged { pane, key, value } => {
            json!({"event": "tag_changed", "pane": pane, "key": key, "value": value})
        }
        Event::Bell { pane } => json!({"event": "bell", "pane": pane}),
        Event::TitleChanged { pane, title } => json!({"event": "title_changed", "pane": pane, "title": title}),
        Event::CwdChanged { pane, cwd } => json!({"event": "cwd_changed", "pane": pane, "cwd": cwd}),
        Event::OutputStarted { pane } => json!({"event": "output_started", "pane": pane}),
        Event::ModeChanged { client, mode } => json!({"event": "mode_changed", "client": client, "mode": mode}),
        Event::Status { client, state } => {
            let mut v = json!({"event": "status", "client": client});
            if let (Value::Object(o), Value::Object(st)) = (&mut v, state) {
                o.extend(st.clone());
            }
            v
        }
    })
}

fn line(v: Value) -> String {
    let mut s = v.to_string();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_with_defaults_and_keep_their_id() {
        let (id, r) = parse_request(r#"{"id":3,"cmd":"split","command":"cargo test","wait":true}"#);
        assert_eq!(id, Some(json!(3)));
        assert_eq!(
            r,
            Ok(Request::Split {
                pane: None,
                dir: Dir::Right,
                cwd: None,
                command: Some("cargo test".into()),
                focus: false,
                wait: true,
                float: None,
                close_on_exit: false,
                keep: false
            })
        );
        let (_, r) = parse_request(r#"{"cmd":"send","pane":7,"input":[{"text":"ls"},{"key":"enter"}]}"#);
        assert_eq!(
            r,
            Ok(Request::Send { pane: Some(7), input: vec![Input::Text("ls".into()), Input::Key("enter".into())] })
        );
    }

    #[test]
    fn overlays_take_plain_or_styled_lines() {
        let (_, r) = parse_request(
            r#"{"cmd":"overlay_open","title":"keys","lines":["plain",[{"text":"v","style":"badge.done","bold":true},{"text":" vsplit"}]],"anchor":"bottom_right"}"#,
        );
        let Ok(Request::OverlayOpen { session: None, client: None, spec }) = r else { panic!("{r:?}") };
        assert_eq!(spec.anchor, AnchorSpec::BottomRight);
        assert_eq!(spec.lines[0], LineSpec::Text("plain".into()));
        assert!(matches!(&spec.lines[1], LineSpec::Spans(s) if s[0].bold && s[1].style.is_none()));
        assert!(parse_request(r#"{"cmd":"overlay_open","lines":[],"anchor":"middle"}"#).1.is_err());
        assert!(parse_request(r#"{"cmd":"overlay_open","lines":[],"colour":1}"#).1.is_err(), "typos are refused");
    }

    #[test]
    fn layouts_nest_splits_and_panes() {
        let (_, r) = parse_request(
            r#"{"cmd":"apply_layout","layout":{"root":{"split":"right","parts":[{"cwd":"~/src","size":30},{"split":"down","parts":[{},{"command":"htop"}]}]},"floats":[{"w":50,"cwd":"/tmp"}]}}"#,
        );
        let Ok(Request::ApplyLayout { layout, .. }) = r else { panic!("{r:?}") };
        let LayoutNode::Split(root) = &layout.root else { panic!() };
        assert_eq!(root.split, Dir::Right);
        assert!(
            matches!(&root.parts[0], LayoutNode::Pane(p) if p.size == Some(30.0) && p.cwd.as_deref() == Some("~/src"))
        );
        assert!(matches!(&root.parts[1], LayoutNode::Split(s) if s.split == Dir::Down));
        assert_eq!(layout.floats[0].at.w, 50);
        assert!(parse_request(r#"{"cmd":"apply_layout","layout":{"root":{"cwd":"/","colour":1}}}"#).1.is_err());
    }

    #[test]
    fn an_empty_float_is_centred() {
        let (_, r) = parse_request(r#"{"cmd":"split","float":{}}"#);
        assert!(matches!(r, Ok(Request::Split { float: Some(f), .. }) if f == FloatSpec::default()));
        assert!(parse_request(r#"{"cmd":"split","float":{"z":1}}"#).1.is_err());
    }

    #[test]
    fn a_plugin_bar_is_spans_with_colours() {
        let (_, r) = parse_request(
            r##"{"cmd":"status_set","client":3,"left":[{"text":" s ","style":"0","bg":"#82aaff","bold":true,"italic":true}]}"##,
        );
        let Ok(Request::StatusSet { client: 3, left, right, fill: None }) = r else { panic!("{r:?}") };
        assert!(right.is_empty() && left[0].italic && left[0].bg.as_deref() == Some("#82aaff"));
        let line = event(&Event::Status { client: 3, state: json!({"mode": "leader"}) });
        assert_eq!(line, "{\"event\":\"status\",\"client\":3,\"mode\":\"leader\"}\n");
    }

    #[test]
    fn host_requests_parse() {
        assert_eq!(
            parse_request(r#"{"cmd":"plugin_running","plugin":null}"#).1,
            Ok(Request::PluginRunning { plugin: None })
        );
        assert_eq!(
            parse_request(r#"{"cmd":"plugin_running","plugin":"hints"}"#).1,
            Ok(Request::PluginRunning { plugin: Some("hints".into()) })
        );
        assert!(matches!(
            parse_request(r#"{"id":0,"cmd":"plugin_hello","token":"t"}"#).1,
            Ok(Request::PluginHello { .. })
        ));
    }

    #[test]
    fn bad_requests_say_why_and_keep_the_id() {
        let (id, r) = parse_request(r#"{"id":"a","cmd":"fly"}"#);
        assert_eq!(id, Some(json!("a")));
        assert!(r.unwrap_err().contains("fly"));
        assert!(parse_request(r#"{"cmd":"split","colour":1}"#).1.unwrap_err().contains("colour"));
        assert!(parse_request("nope").1.is_err());
        assert!(parse_request("[1]").1.is_err());
    }

    #[test]
    fn round_trips_through_the_cli_encoder() {
        let r = Request::Split {
            pane: Some(2),
            dir: Dir::Down,
            cwd: Some("/tmp".into()),
            command: None,
            focus: true,
            wait: false,
            float: Some(FloatSpec { w: 50, ..FloatSpec::default() }),
            close_on_exit: true,
            keep: false,
        };
        let (id, back) = parse_request(encode_request(Some(9), &r).trim_end());
        assert_eq!((id, back), (Some(json!(9)), Ok(r)));
    }

    #[test]
    fn replies_and_events_are_single_lines() {
        assert_eq!(ok(Some(&json!(1)), json!({"pane": 7})), "{\"id\":1,\"ok\":true,\"pane\":7}\n");
        assert_eq!(error(None, "no pane 9"), "{\"ok\":false,\"error\":\"no pane 9\"}\n");
        assert_eq!(
            event(&Event::CommandFinished { pane: 7, exit: Some(1) }),
            "{\"event\":\"command_finished\",\"pane\":7,\"exit\":1}\n"
        );
        assert!(!ok(None, json!({"text": "a\nb"})).trim_end().contains('\n'));
    }

    #[test]
    fn run_and_capture_requests() {
        assert_eq!(
            parse_request(r#"{"cmd":"run","command":"goto-tab 3"}"#).1,
            Ok(Request::Run { command: Target::Name("goto-tab 3".into()), client: None, session: None })
        );
        assert_eq!(
            parse_request(r#"{"cmd":"capture","history":true}"#).1,
            Ok(Request::Capture { pane: None, history: true })
        );
        assert!(matches!(
            parse_request(r#"{"cmd":"split","command":"fzf","close_on_exit":true}"#).1,
            Ok(Request::Split { close_on_exit: true, .. })
        ));
        assert!(matches!(
            parse_request(r#"{"cmd":"split","command":"make","keep":true}"#).1,
            Ok(Request::Split { keep: true, .. })
        ));
        let (_, r) = parse_request(r#"{"cmd":"run","command":{"exec":["git","pull"]}}"#);
        assert!(
            matches!(&r, Ok(Request::Run { command: Target::Shortcut(t), .. }) if t["exec"] == json!(["git", "pull"])),
            "{r:?}"
        );
        let (_, r) = parse_request(r#"{"cmd":"bind","keys":"g e","command":{"run":"nvim"}}"#);
        assert!(matches!(r, Ok(Request::Bind { command: Some(Target::Shortcut(_)), .. })), "{r:?}");
        assert!(parse_request(r#"{"cmd":"run","command":3}"#).1.is_err());
    }

    #[test]
    fn pane_and_mode_events_encode_flat() {
        assert_eq!(
            event(&Event::CwdChanged { pane: 3, cwd: "/tmp".into() }),
            "{\"event\":\"cwd_changed\",\"pane\":3,\"cwd\":\"/tmp\"}\n"
        );
        assert_eq!(
            event(&Event::ModeChanged { client: 1, mode: "visual".into() }),
            "{\"event\":\"mode_changed\",\"client\":1,\"mode\":\"visual\"}\n"
        );
    }

    #[test]
    fn set_layout_names_panes() {
        let (_, r) = parse_request(
            r#"{"cmd":"set_layout","tab":2,"layout":{"root":{"split":"right","parts":[{"pane":4},{}]},"floats":[{"pane":7}]}}"#,
        );
        let Ok(Request::SetLayout { tab: Some(2), layout, .. }) = r else { panic!("{r:?}") };
        let LayoutNode::Split(root) = &layout.root else { panic!() };
        assert!(matches!(&root.parts[0], LayoutNode::Pane(p) if p.pane == Some(4)));
        assert_eq!(layout.floats[0].pane.pane, Some(7));
    }

    #[test]
    fn pane_title_requests() {
        assert_eq!(
            parse_request(r#"{"cmd":"pane_title","pane":2,"title":"build"}"#).1,
            Ok(Request::PaneTitle { pane: Some(2), title: Some("build".into()) })
        );
        assert_eq!(parse_request(r#"{"cmd":"pane_title"}"#).1, Ok(Request::PaneTitle { pane: None, title: None }));
    }

    #[test]
    fn tag_requests_and_tagged_panes() {
        assert_eq!(
            parse_request(r#"{"cmd":"tag","pane":3,"key":"git.branch","value":"main"}"#).1,
            Ok(Request::Tag { pane: Some(3), key: "git.branch".into(), value: Some("main".into()) })
        );
        let (_, r) = parse_request(r#"{"cmd":"apply_layout","layout":{"root":{"tags":{"k":"v"}}}}"#);
        let Ok(Request::ApplyLayout { layout, .. }) = r else { panic!("{r:?}") };
        assert!(matches!(&layout.root, LayoutNode::Pane(p) if p.tags.get("k").map(String::as_str) == Some("v")));
    }

    #[test]
    fn bind_requests() {
        assert_eq!(
            parse_request(r#"{"cmd":"bind","keys":"g y","command":"zoom"}"#).1,
            Ok(Request::Bind { keys: "g y".into(), command: Some(Target::Name("zoom".into())), direct: false })
        );
        assert_eq!(
            parse_request(r#"{"cmd":"bind","keys":"ctrl+h","direct":true}"#).1,
            Ok(Request::Bind { keys: "ctrl+h".into(), command: None, direct: true })
        );
    }
}
