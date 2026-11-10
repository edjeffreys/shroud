//! A layout tree plus the panes its leaves name, and floats over them. Owns pane lifetimes.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use shroud_core::config::PaneSpec;
use shroud_core::input::Action;
use shroud_core::layout::{Axis, FloatRect, PaneId, Placement, Rect, Shape, Tree, neighbour};
use shroud_core::protocol::Size;

use super::launch::Launch;
use super::pane::{self, Pane};

mod floats;
mod rearrange;

#[derive(Debug, Clone, Copy)]
pub struct Float {
    pub pane: PaneId,
    pub at: FloatRect,
}

/// A pane as drawn: tiled, or a float with the frame around it.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub pane: PaneId,
    pub rect: Rect,
    pub float: Option<Rect>,
}

#[derive(Debug)]
pub struct Tab {
    pub tree: Tree,
    pub panes: BTreeMap<PaneId, Pane>,
    pub focused: PaneId,
    /// Bottom to top.
    pub floats: Vec<Float>,
    pub floats_shown: bool,
    pub name: Option<String>,
    /// When the user last left this tab, for `last-tab`.
    pub left_at: Option<Instant>,
    /// Drawn alone over the whole area; the rest keep their sizes underneath.
    zoomed: Option<PaneId>,
    previous: Option<PaneId>,
    area: Rect,
    cell_px: (u32, u32),
    launch: Launch,
}

impl Tab {
    pub fn new(size: Size, launch: Launch, cwd: Option<&Path>) -> io::Result<Tab> {
        let area = pane_area(size);
        let id = pane::next_id();
        let pane = Pane::spawn(id, area.w, area.h, cwd, &launch)?;
        Ok(Tab::adopt(size, launch, id, pane))
    }

    /// Any pane that fails to start fails the whole tab, so a layout never half-applies.
    pub fn from_shape(size: Size, launch: Launch, shape: &Shape<PaneSpec>, base: &Path) -> io::Result<Tab> {
        let mut specs = Vec::new();
        let ids = shape.clone().map(&mut |spec| {
            let id = pane::next_id();
            specs.push((id, spec));
            id
        });
        let tree = Tree::from_shape(&ids).ok_or_else(|| io::Error::other("empty layout"))?;
        let area = pane_area(size);
        let placements = tree.layout(area);
        let mut panes = BTreeMap::new();
        for (id, spec) in specs {
            let cwd = resolve(base, spec.cwd.as_deref())?;
            let rect = placements.iter().find(|p| p.pane == id).map(|p| p.rect).unwrap_or_default();
            let mut pane = Pane::spawn(id, rect.w.max(1), rect.h.max(1), Some(&cwd), &launch)?;
            if let Some(cmd) = &spec.command {
                pane.type_line(cmd);
            }
            pane.tags = spec.tags;
            panes.insert(id, pane);
        }
        let focused = placements.first().map_or(0, |p| p.pane);
        let mut tab = Tab {
            tree,
            panes,
            focused,
            floats: Vec::new(),
            floats_shown: true,
            name: None,
            left_at: None,
            zoomed: None,
            previous: None,
            area,
            cell_px: cell_px(size),
            launch,
        };
        tab.relayout();
        Ok(tab)
    }

    pub fn set_launch(&mut self, launch: Launch) {
        let reframe = launch.frames != self.launch.frames;
        self.launch = launch;
        if reframe {
            self.relayout();
        }
    }

    /// Where tiles go: inside the frame when there is one. Floats use the whole area.
    fn tiles(&self) -> Rect {
        if self.launch.frames { self.area.inset(1) } else { self.area }
    }

    pub fn frames(&self) -> bool {
        self.launch.frames
    }

    /// A tab around a pane that already exists, for moving a pane into its own tab.
    pub fn adopt(size: Size, launch: Launch, id: PaneId, pane: Pane) -> Tab {
        let mut tab = Tab {
            tree: Tree::new(id),
            panes: BTreeMap::from([(id, pane)]),
            focused: id,
            floats: Vec::new(),
            floats_shown: true,
            name: None,
            left_at: None,
            zoomed: None,
            previous: None,
            area: pane_area(size),
            cell_px: cell_px(size),
            launch,
        };
        tab.relayout();
        tab
    }

    /// None for a float or the last tile: a tab can't be left without one.
    pub fn take(&mut self, id: PaneId) -> Option<Pane> {
        if self.tree.len() < 2 || !self.tree.contains(id) {
            return None;
        }
        let next = self.tree.remove(id).ok()??;
        if self.focused == id {
            self.focused = next;
        }
        if self.previous == Some(id) {
            self.previous = None;
        }
        self.relayout();
        self.panes.remove(&id)
    }

    pub fn focused_cwd(&self) -> Option<PathBuf> {
        self.cwd_of(self.focused)
    }

    pub fn cwd_of(&self, id: PaneId) -> Option<PathBuf> {
        self.panes.get(&id).and_then(|p| p.shell.cwd.clone()).map(PathBuf::from)
    }

    pub fn cell_px(&self) -> (u32, u32) {
        self.cell_px
    }

    pub fn area(&self) -> Rect {
        self.area
    }

    pub fn placements(&self) -> Vec<Placement> {
        self.tree.layout(self.tiles())
    }

    /// What is drawn: the zoomed pane alone, or every view.
    pub fn views(&self) -> Vec<View> {
        match self.zoomed.filter(|z| self.panes.contains_key(z)) {
            Some(pane) => vec![View { pane, rect: self.tiles(), float: None }],
            None => self.all_views(),
        }
    }

    pub fn is_zoomed(&self) -> bool {
        self.zoomed.is_some()
    }

    /// Only worth it with something to hide.
    pub fn toggle_zoom(&mut self) {
        self.zoomed = match self.zoomed {
            Some(_) => None,
            None => (self.all_views().len() > 1).then_some(self.focused),
        };
        self.relayout();
    }

    /// Tiles, then visible floats bottom to top: drawing order, and focus order.
    fn all_views(&self) -> Vec<View> {
        let mut v: Vec<View> =
            self.placements().iter().map(|p| View { pane: p.pane, rect: p.rect, float: None }).collect();
        if self.floats_shown {
            v.extend(self.floats.iter().map(|f| View {
                pane: f.pane,
                rect: f.at.inner(self.area),
                float: Some(f.at.outer(self.area)),
            }));
        }
        v
    }

    pub fn focused_pane(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.focused)
    }

    pub fn resize(&mut self, size: Size) {
        self.area = pane_area(size);
        self.cell_px = cell_px(size);
        self.relayout();
    }

    pub fn split(&mut self, axis: Axis) -> io::Result<()> {
        let cwd = self.focused_cwd();
        self.split_pane(self.focused, axis, cwd.as_deref(), true).map(|_| ())
    }

    pub fn split_pane(&mut self, target: PaneId, axis: Axis, cwd: Option<&Path>, focus: bool) -> io::Result<PaneId> {
        let id = pane::next_id();
        self.zoomed = None;
        self.tree.split(target, axis, id).map_err(|e| io::Error::other(format!("{e:?}")))?;
        let rect = self.placements().iter().find(|p| p.pane == id).map(|p| p.rect).unwrap_or_default();
        match Pane::spawn(id, rect.w.max(1), rect.h.max(1), cwd, &self.launch) {
            Ok(pane) => {
                self.panes.insert(id, pane);
                if focus {
                    self.set_focus(id);
                }
                self.relayout();
                Ok(id)
            }
            Err(e) => {
                let _ = self.tree.remove(id);
                Err(e)
            }
        }
    }

    pub fn focus_pane(&mut self, id: PaneId) {
        if self.panes.contains_key(&id) {
            if self.is_float(id) {
                self.floats_shown = true;
            }
            self.set_focus(id);
        }
    }

    pub fn apply(&mut self, action: Action) -> bool {
        let order: Vec<PaneId> = self.all_views().iter().map(|v| v.pane).collect();
        let at = order.iter().position(|&p| p == self.focused).unwrap_or(0);
        let focused = self.focused;
        match action {
            Action::Split(axis) => {
                if let Err(e) = self.split(axis) {
                    eprintln!("shroud: split: {e}");
                }
            }
            Action::Focus(dir) if self.is_float(focused) => self.focus_from_float(dir),
            Action::Focus(dir) => {
                if let Some(id) = neighbour(&self.placements(), focused, dir) {
                    self.set_focus(id);
                }
            }
            Action::FocusNext => self.set_focus(order[(at + 1) % order.len()]),
            Action::FocusPrev => self.set_focus(order[(at + order.len() - 1) % order.len()]),
            Action::FocusNth(n) => self.set_focus(order[(n as usize).clamp(1, order.len()) - 1]),
            Action::FocusPrevious => {
                if let Some(p) = self.previous.filter(|p| self.panes.contains_key(p)) {
                    self.set_focus(p);
                }
            }
            Action::FocusFirst => self.set_focus(order[0]),
            Action::FocusLastPane => self.set_focus(order[order.len() - 1]),
            Action::ToggleFloats => {
                if let Err(e) = self.toggle_floats() {
                    eprintln!("shroud: float: {e}");
                }
            }
            Action::NewFloat => {
                let cwd = self.focused_cwd();
                if let Err(e) = self.new_float(FloatRect::CENTRED, cwd.as_deref(), true) {
                    eprintln!("shroud: float: {e}");
                }
            }
            Action::Close => return self.remove(focused),
            Action::Only => {
                // From a float, the other floats close; the tiles a tab needs stay.
                let from_float = self.is_float(focused);
                let others: Vec<PaneId> =
                    order.into_iter().filter(|&id| id != focused && self.is_float(id) == from_float).collect();
                for id in others {
                    self.remove(id);
                }
            }
            Action::Grow(axis, n) => {
                self.tree.grow(self.tiles(), focused, axis, n);
            }
            Action::SetSize(axis, cells) => {
                if let Some(r) = self.placements().iter().find(|p| p.pane == focused).map(|p| p.rect) {
                    let now = if axis == Axis::Horizontal { r.w } else { r.h };
                    self.tree.grow(self.tiles(), focused, axis, cells as i32 - now as i32);
                }
            }
            Action::Equalize => self.tree.equalize(),
            Action::Maximize(axis) => self.tree.maximize(self.tiles(), focused, axis),
            Action::MoveToEdge(dir) => {
                self.tree.move_to_edge(focused, dir);
            }
            Action::Exchange => {
                if let Some(other) = self.tree.exchange(focused) {
                    self.set_focus(other);
                }
            }
            Action::Rotate => {
                self.tree.rotate(focused);
            }
            _ => {}
        }
        self.relayout();
        true
    }

    fn set_focus(&mut self, id: PaneId) {
        if id != self.focused {
            self.zoomed = None;
            self.previous = Some(self.focused);
            self.focused = id;
        }
        if let Some(p) = self.panes.get_mut(&id) {
            p.badge = None;
        }
    }

    pub fn remove(&mut self, id: PaneId) -> bool {
        if self.zoomed == Some(id) {
            self.zoomed = None;
        }
        if let Some(i) = self.floats.iter().position(|f| f.pane == id) {
            self.floats.remove(i);
            self.panes.remove(&id);
            if self.focused == id {
                match self.floats.last().filter(|_| self.floats_shown) {
                    Some(f) => self.focused = f.pane,
                    None => self.focus_tiled(),
                }
            }
            return true;
        }
        self.panes.remove(&id);
        match self.tree.remove(id) {
            Ok(Some(next)) => {
                if self.focused == id || !self.panes.contains_key(&self.focused) {
                    self.focused = next;
                    if let Some(p) = self.panes.get_mut(&next) {
                        p.badge = None;
                    }
                }
                if self.previous == Some(id) {
                    self.previous = None;
                }
                self.relayout();
                true
            }
            // A tab must keep a tile, so the top float takes the last one's place.
            Ok(None) => match self.floats.pop() {
                Some(f) => {
                    self.tree = Tree::new(f.pane);
                    self.focused = f.pane;
                    self.previous = None;
                    self.relayout();
                    true
                }
                None => false,
            },
            Err(_) => !self.panes.is_empty(),
        }
    }

    fn relayout(&mut self) {
        let (cw, ch) = self.cell_px;
        let floats = self.floats.iter().map(|f| (f.pane, f.at.inner(self.area)));
        let tiles = self.tree.layout(self.tiles()).into_iter().map(|p| (p.pane, p.rect));
        for (id, rect) in tiles.collect::<Vec<_>>().into_iter().chain(floats.collect::<Vec<_>>()) {
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.resize(rect.w.max(1), rect.h.max(1), cw, ch);
            }
        }
        let whole = self.tiles();
        if let Some(pane) = self.zoomed.and_then(|z| self.panes.get_mut(&z)) {
            pane.resize(whole.w.max(1), whole.h.max(1), cw, ch);
        }
    }
}

fn resolve(base: &Path, cwd: Option<&str>) -> io::Result<PathBuf> {
    let path = match cwd {
        None => return Ok(base.to_path_buf()),
        Some("~") => home(),
        Some(c) => match c.strip_prefix("~/") {
            Some(rest) => home().join(rest),
            None => base.join(c),
        },
    };
    if path.is_dir() { Ok(path) } else { Err(io::Error::other(format!("cwd {} is not a directory", path.display()))) }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// The bottom row belongs to the status bar, unless that would leave no room for panes.
pub fn pane_area(size: Size) -> Rect {
    let rows = if size.rows >= 2 { size.rows - 1 } else { size.rows };
    Rect::new(0, 0, size.cols, rows)
}

fn cell_px(size: Size) -> (u32, u32) {
    let w = if size.cols > 0 { size.px_w as u32 / size.cols as u32 } else { 0 };
    let h = if size.rows > 0 { size.px_h as u32 / size.rows as u32 } else { 0 };
    (w, h)
}
