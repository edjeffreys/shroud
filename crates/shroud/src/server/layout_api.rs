//! Layouts and sessions over `control.sock`, in the layout files' model, so saving a
//! session and restoring it needs nothing else.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use shroud_core::config::PaneSpec;
use shroud_core::layout::{Axis, FloatRect, PaneId, Shape, Slot};
use shroud_core::protocol::api::{Dir, FloatNode, FloatSpec, LayoutNode, LayoutSpec, PaneNode, SplitNode};
use shroud_core::protocol::tags::{self, Tags};

use super::DEFAULT_SIZE;
use super::Server;
use super::tab::Tab;

impl Server {
    pub(super) fn session_index(&self, name: Option<&str>) -> Result<usize, String> {
        match name {
            Some(n) => self.sessions.iter().position(|s| s.name == n).ok_or_else(|| format!("no session named {n}")),
            None => self
                .sessions
                .iter()
                .enumerate()
                .max_by_key(|(_, s)| s.last_used)
                .map(|(i, _)| i)
                .ok_or_else(|| "no sessions".into()),
        }
    }

    pub(super) fn layout_of(&self, session: Option<&str>, tab: Option<usize>) -> Result<Value, String> {
        let s = &self.sessions[self.session_index(session)?];
        let t = match tab {
            Some(n) => s.tabs.get(n.wrapping_sub(1)).ok_or_else(|| format!("no tab {n}"))?,
            None => s.tab(),
        };
        let root = node(&t.tree.to_shape(), t, None);
        let floats = t
            .floats
            .iter()
            .map(|f| FloatNode {
                at: FloatSpec { x: f.at.x, y: f.at.y, w: f.at.w, h: f.at.h },
                pane: pane_node(t, f.pane, None),
            })
            .collect();
        Ok(json!({"layout": LayoutSpec { root, floats, floats_hidden: !t.floats.is_empty() && !t.floats_shown }}))
    }

    pub(super) fn apply_layout(&mut self, session: Option<&str>, layout: LayoutSpec) -> Result<Value, String> {
        check_tags(&layout)?;
        let si = self.session_index(session)?;
        let base = home();
        let shape = shape(&layout.root);
        let s = &mut self.sessions[si];
        let at = s.add_tab_from_shape(&shape, &base).map_err(|e| format!("layout: {e}"))?;
        let tab = &mut s.tabs[at];
        for f in &layout.floats {
            let rect = FloatRect::new(f.at.x, f.at.y, f.at.w, f.at.h).ok_or("a float must fit inside the tab")?;
            let cwd = f.pane.cwd.as_deref().map(|c| expand(&base, c));
            let id = tab.new_float(rect, cwd.as_deref(), false).map_err(|e| format!("float: {e}"))?;
            if let (Some(cmd), Some(p)) = (&f.pane.command, tab.panes.get_mut(&id)) {
                p.type_line(cmd);
            }
        }
        // new_float shows them; a saved session puts back whatever was showing when it was saved.
        tab.floats_shown = !layout.floats_hidden;
        self.invalidate();
        Ok(json!({"tab": at + 1}))
    }

    pub(super) fn set_layout(
        &mut self,
        session: Option<&str>,
        tab: Option<usize>,
        layout: LayoutSpec,
    ) -> Result<Value, String> {
        check_tags(&layout)?;
        let si = self.session_index(session)?;
        let s = &mut self.sessions[si];
        let ti = match tab {
            Some(n) if (1..=s.tabs.len()).contains(&n) => n - 1,
            Some(n) => return Err(format!("no tab {n}")),
            None => s.active,
        };
        let mut floats = Vec::new();
        for f in &layout.floats {
            let at = FloatRect::new(f.at.x, f.at.y, f.at.w, f.at.h).ok_or("a float must fit inside the tab")?;
            floats.push((at, slot(&f.pane)));
        }
        s.tabs[ti].rearrange(slots(&layout.root), floats, &home())?;
        self.invalidate();
        Ok(json!({}))
    }

    pub(super) fn session_new(&mut self, name: Option<String>, cwd: Option<String>) -> Result<Value, String> {
        let name = name.unwrap_or_else(|| self.free_name());
        let cwd = cwd.map_or_else(home, |c| expand(&home(), &c));
        let size = self.clients.iter().find_map(|c| c.size).unwrap_or(DEFAULT_SIZE);
        self.create(&name, &cwd, size)?;
        Ok(json!({"session": name}))
    }

    pub(super) fn tab_new(&mut self, session: Option<&str>) -> Result<Value, String> {
        let si = self.session_index(session)?;
        let s = &mut self.sessions[si];
        let cwd = s.tab().focused_cwd().unwrap_or_else(home);
        let shape = Shape::Leaf(PaneSpec { cwd: Some(cwd.to_string_lossy().into_owned()), ..PaneSpec::default() });
        let at = s.add_tab_from_shape(&shape, &cwd).map_err(|e| format!("tab: {e}"))?;
        self.invalidate();
        Ok(json!({"tab": at + 1}))
    }
}

fn node(shape: &Shape<PaneId>, tab: &Tab, size: Option<f32>) -> LayoutNode {
    match shape {
        Shape::Leaf(id) => LayoutNode::Pane(pane_node(tab, *id, size)),
        Shape::Split { axis, parts } => LayoutNode::Split(SplitNode {
            split: if *axis == Axis::Horizontal { Dir::Right } else { Dir::Down },
            parts: parts.iter().map(|(s, p)| node(p, tab, s.map(percent))).collect(),
            size,
        }),
    }
}

fn pane_node(tab: &Tab, id: PaneId, size: Option<f32>) -> PaneNode {
    let p = tab.panes.get(&id);
    PaneNode {
        pane: Some(id),
        cwd: p.and_then(|p| p.shell.cwd.clone()),
        title: p.map(|p| p.title.clone()).filter(|t| !t.is_empty()),
        process: p.and_then(|p| p.process()),
        tags: p.map(|p| p.tags.clone()).unwrap_or_default(),
        command: None,
        size,
    }
}

// One decimal place: enough to rebuild the same cells, short enough to read.
fn percent(fraction: f32) -> f32 {
    (fraction * 1000.0).round() / 10.0
}

fn shape(n: &LayoutNode) -> Shape<PaneSpec> {
    match n {
        LayoutNode::Pane(p) => {
            Shape::Leaf(PaneSpec { cwd: p.cwd.clone(), command: p.command.clone(), tags: p.tags.clone() })
        }
        LayoutNode::Split(s) => Shape::Split {
            axis: if s.split == Dir::Right { Axis::Horizontal } else { Axis::Vertical },
            parts: s.parts.iter().map(|p| (size_of(p).map(|v| v / 100.0), shape(p))).collect(),
        },
    }
}

fn check_tags(layout: &LayoutSpec) -> Result<(), String> {
    fn pane(p: &PaneNode) -> Result<(), String> {
        let mut t = Tags::new();
        p.tags.iter().try_for_each(|(k, v)| tags::set(&mut t, k, Some(v.clone())))
    }
    fn walk(n: &LayoutNode) -> Result<(), String> {
        match n {
            LayoutNode::Pane(p) => pane(p),
            LayoutNode::Split(s) => s.parts.iter().try_for_each(walk),
        }
    }
    walk(&layout.root)?;
    layout.floats.iter().try_for_each(|f| pane(&f.pane))
}

fn slots(n: &LayoutNode) -> Shape<Slot<PaneSpec>> {
    match n {
        LayoutNode::Pane(p) => Shape::Leaf(slot(p)),
        LayoutNode::Split(s) => Shape::Split {
            axis: if s.split == Dir::Right { Axis::Horizontal } else { Axis::Vertical },
            parts: s.parts.iter().map(|p| (size_of(p).map(|v| v / 100.0), slots(p))).collect(),
        },
    }
}

fn slot(p: &PaneNode) -> Slot<PaneSpec> {
    match p.pane {
        Some(id) => Slot::Pane(id),
        None => Slot::New(PaneSpec { cwd: p.cwd.clone(), command: p.command.clone(), tags: p.tags.clone() }),
    }
}

fn size_of(n: &LayoutNode) -> Option<f32> {
    match n {
        LayoutNode::Pane(p) => p.size,
        LayoutNode::Split(s) => s.size,
    }
}

fn expand(base: &Path, c: &str) -> PathBuf {
    match c.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if c == "~" => home(),
        None => base.join(c),
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}
