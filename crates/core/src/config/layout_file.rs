//! All or nothing: half a layout would surprise more than one plain pane.

use kdl::{KdlDocument, KdlNode, KdlValue};

use super::Problem;
use crate::layout::{Axis, MAX_PANES, Shape};

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PaneSpec {
    pub cwd: Option<String>,
    pub command: Option<String>,
    /// Only from the API: layout files don't set tags.
    #[serde(default)]
    pub tags: std::collections::BTreeMap<String, String>,
}

/// One shape per tab. An empty file is no tabs. Bad input can take seconds in `kdl`.
pub fn parse_layout(text: &str) -> Result<Vec<Shape<PaneSpec>>, Problem> {
    layout_bounds(text)?;
    let doc = KdlDocument::parse(text).map_err(|e| match e.diagnostics.first() {
        Some(d) => Problem::at(
            text,
            d.span.offset(),
            d.message.clone().or(d.label.clone()).unwrap_or_else(|| "invalid KDL".into()),
        ),
        None => Problem { line: None, message: "invalid KDL".into() },
    })?;
    let p = Parser { text };
    let nodes = doc.nodes();
    let tabs = if nodes.iter().all(|n| n.name().value() == "tab") {
        nodes.iter().map(|n| p.tab(n)).collect::<Result<Vec<_>, _>>()?
    } else if let [only] = nodes {
        vec![p.shape(only)?]
    } else {
        return Err(p.at(&nodes[0], "use `tab` nodes, or a single pane or split for one tab"));
    };
    if tabs.iter().any(|t| t.leaves().len() > MAX_PANES) {
        return Err(Problem { line: None, message: format!("a tab holds at most {MAX_PANES} panes") });
    }
    Ok(tabs)
}

struct Parser<'t> {
    text: &'t str,
}

impl Parser<'_> {
    fn at(&self, node: &KdlNode, message: impl Into<String>) -> Problem {
        Problem::at(self.text, node.span().offset(), message)
    }

    fn tab(&self, node: &KdlNode) -> Result<Shape<PaneSpec>, Problem> {
        self.props(node, &[])?;
        match children(node) {
            [] => Ok(Shape::Leaf(PaneSpec::default())),
            [only] => self.shape(only),
            _ => Err(self.at(node, "a tab holds one pane or split")),
        }
    }

    fn shape(&self, node: &KdlNode) -> Result<Shape<PaneSpec>, Problem> {
        Ok(self.part(node)?.1)
    }

    fn part(&self, node: &KdlNode) -> Result<(Option<f32>, Shape<PaneSpec>), Problem> {
        let name = node.name().value();
        let size = self.size(node)?;
        let shape = match name {
            "pane" => {
                self.props(node, &["cwd", "command", "size"])?;
                if !children(node).is_empty() {
                    return Err(self.at(node, "a pane has no children; wrap them in a split"));
                }
                Shape::Leaf(PaneSpec {
                    cwd: self.string(node, "cwd")?,
                    command: self.string(node, "command")?,
                    ..PaneSpec::default()
                })
            }
            "split" | "vsplit" => {
                self.props(node, &["size"])?;
                let axis = if name == "vsplit" { Axis::Horizontal } else { Axis::Vertical };
                let parts = children(node).iter().map(|c| self.part(c)).collect::<Result<Vec<_>, _>>()?;
                if parts.is_empty() {
                    return Err(self.at(node, format!("an empty `{name}`")));
                }
                Shape::Split { axis, parts }
            }
            "tab" => return Err(self.at(node, "tabs belong at the top level")),
            other => return Err(self.at(node, format!("unknown node `{other}`; use pane, split or vsplit"))),
        };
        Ok((size, shape))
    }

    fn props(&self, node: &KdlNode, allowed: &[&str]) -> Result<(), Problem> {
        for entry in node.entries() {
            match entry.name() {
                Some(n) if allowed.contains(&n.value()) => {}
                Some(n) => {
                    return Err(self.at(node, format!("`{}` has no property `{}`", node.name().value(), n.value())));
                }
                None => {
                    return Err(self.at(
                        node,
                        format!("`{}` takes properties like `cwd=\"src\"`, not bare values", node.name().value()),
                    ));
                }
            }
        }
        Ok(())
    }

    fn string(&self, node: &KdlNode, key: &str) -> Result<Option<String>, Problem> {
        match node.get(key) {
            None => Ok(None),
            Some(KdlValue::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(self.at(node, format!("`{key}` must be a string"))),
        }
    }

    fn size(&self, node: &KdlNode) -> Result<Option<f32>, Problem> {
        let Some(raw) = self.string(node, "size")? else { return Ok(None) };
        match raw.strip_suffix('%').and_then(|n| n.trim().parse::<f32>().ok()) {
            Some(pct) if pct > 0.0 && pct < 100.0 => Ok(Some(pct / 100.0)),
            _ => Err(self.at(node, format!("size `{raw}` should be a percentage such as \"30%\""))),
        }
    }
}

fn children(node: &KdlNode) -> &[KdlNode] {
    node.children().map_or(&[], |d| d.nodes())
}

/// Size and depth, both well past any real layout. Cheap enough to run before spawning a parser.
pub fn layout_bounds(text: &str) -> Result<(), Problem> {
    if text.len() > MAX_BYTES {
        return Err(Problem { line: None, message: format!("a layout file holds at most {} KiB", MAX_BYTES / 1024) });
    }
    match too_deep(text) {
        Some(offset) => Err(Problem::at(text, offset, format!("blocks nest more than {MAX_DEPTH} deep"))),
        None => Ok(()),
    }
}

/// `kdl` 6.7's error recovery costs about 2.75^depth × length on input like nested `( /-{`.
const MAX_DEPTH: usize = 8;
const MAX_BYTES: usize = 16 * 1024;

/// Braces in strings count, since `kdl`'s recovery can end a string early;
/// there a `}` only closes a `{` from the same string, so `"}}"` hides nothing.
fn too_deep(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let (mut i, mut depth) = (0, 0usize);
    while i < b.len() {
        let end = match (b[i], b.get(i + 1)) {
            (b'"', _) => Some(quoted_end(b, i)),
            (b'#', _) => raw_end(b, i),
            (b'/', Some(b'/')) => Some(b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |n| i + n)),
            (b'/', Some(b'*')) => Some(comment_end(b, i)),
            _ => None,
        };
        let (end, floor) = end.map_or((i + 1, None), |end| (end, Some(depth)));
        for (n, &c) in b[i..end].iter().enumerate() {
            match c {
                b'{' => depth += 1,
                b'}' if floor.is_none_or(|f| depth > f) => depth = depth.saturating_sub(1),
                _ => {}
            }
            if depth > MAX_DEPTH {
                return Some(i + n);
            }
        }
        i = end;
    }
    None
}

/// Byte-wise is safe: UTF-8 continuation bytes are never ASCII.
fn quoted_end(b: &[u8], start: usize) -> usize {
    let close: &[u8] = if b[start..].starts_with(b"\"\"\"") { b"\"\"\"" } else { b"\"" };
    let mut i = start + close.len();
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
        } else if b[i..].starts_with(close) {
            return i + close.len();
        } else {
            i += 1;
        }
    }
    b.len()
}

fn raw_end(b: &[u8], start: usize) -> Option<usize> {
    let hashes = b[start..].iter().take_while(|&&c| c == b'#').count();
    let from = start + hashes + 1;
    if b.get(from - 1) != Some(&b'"') {
        return None;
    }
    let close: Vec<u8> = std::iter::once(b'"').chain(std::iter::repeat_n(b'#', hashes)).collect();
    Some(b[from..].windows(close.len()).position(|w| w == close).map_or(b.len(), |n| from + n + close.len()))
}

fn comment_end(b: &[u8], start: usize) -> usize {
    let (mut i, mut open) = (start + 2, 1);
    while i < b.len() {
        if b[i..].starts_with(b"/*") {
            open += 1;
            i += 2;
        } else if b[i..].starts_with(b"*/") {
            open -= 1;
            i += 2;
            if open == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    b.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> Shape<PaneSpec> {
        Shape::Leaf(PaneSpec::default())
    }

    #[test]
    fn tabs_splits_and_panes() {
        let text = "tab {\n  vsplit {\n    pane\n    split size=\"30%\" {\n      pane cwd=\"src\"\n      pane command=\"cargo test\"\n    }\n  }\n}\ntab\n";
        let tabs = parse_layout(text).unwrap();
        let inner = Shape::Split {
            axis: Axis::Vertical,
            parts: vec![
                (None, Shape::Leaf(PaneSpec { cwd: Some("src".into()), ..PaneSpec::default() })),
                (None, Shape::Leaf(PaneSpec { command: Some("cargo test".into()), ..PaneSpec::default() })),
            ],
        };
        assert_eq!(
            tabs,
            vec![Shape::Split { axis: Axis::Horizontal, parts: vec![(None, pane()), (Some(0.3), inner)] }, pane()]
        );
    }

    #[test]
    fn a_lone_split_is_one_tab_and_empty_is_none() {
        assert_eq!(parse_layout("vsplit { pane; pane }").unwrap().len(), 1);
        assert!(parse_layout("").unwrap().is_empty());
    }

    #[test]
    fn problems_name_the_line() {
        let cases = [
            ("tab {\n  pane colour=\"red\"\n}", 2, "colour"),
            ("tab {\n  vsplit {\n    window\n  }\n}", 3, "window"),
            ("tab {\n  split size=\"30\" { pane; }\n}", 2, "percentage"),
            ("tab {\n  pane\n  pane\n}", 1, "one pane"),
            ("tab {\n  split\n}", 2, "empty"),
            ("pane\npane", 1, "tab"),
            ("tab {\n  pane {\n", 2, "closing"),
        ];
        for (text, line, needle) in cases {
            let e = parse_layout(text).unwrap_err();
            assert_eq!(e.line, Some(line), "{text}: {e}");
            assert!(e.message.contains(needle), "{text}: {e}");
        }
    }

    #[test]
    fn results_survive_json_for_the_parser_child() {
        for text in [
            "tab {\n  vsplit {\n    pane\n    split size=\"30%\" { pane cwd=\"src\"; pane command=\"cargo test\"; }\n  }\n}\ntab\n",
            "tab {\n  pane colour=\"red\"\n}",
        ] {
            let result = parse_layout(text);
            let json = serde_json::to_string(&result).unwrap();
            assert_eq!(serde_json::from_str::<Result<Vec<Shape<PaneSpec>>, Problem>>(&json).unwrap(), result);
        }
    }

    #[test]
    fn pathological_files_are_refused_before_kdl_sees_them() {
        // fuzz/artifacts/config_parse/oom-23ae387b…: ran past 10 s inside `kdl`.
        let artifact = "=\x00-\x02(\x02( /-{={( /- {\x02 /-{={{.\",$\x0b/- \x00( /- {\x02(\x02( /-{=\x00- {\x02 /-{={{.\",$\x0b/- \x00(  /- {\x02 /-{={{.\",$\x0b/- \x00( /- {\x02(\x02( /-{=\x00- {\x02 /-{={{.\",$\x0b/- \x00( /- {\x02(\x02( /-{=\x00\x00\x00E- {\x02 /-{=z{.\x00\x00E- {\x02 /-{=z{..{.{\"\",,/- {\x02(\x02( /-{=\x00\x00\x00E- {\x02 /-{=z{.\x00\x00E- {\x02 /-{=z{..{.{\"\",,";
        assert_eq!(artifact.len(), 238);
        let deep_in_strings = format!("\"}}}}\" {}", "( /-\"{\" ".repeat(40));
        let long = format!("tab{}", " ".repeat(MAX_BYTES));
        for text in [artifact, &"( /-{".repeat(40), &deep_in_strings, &long] {
            let start = std::time::Instant::now();
            let e = parse_layout(text).unwrap_err();
            assert!(e.message.contains("nest") || e.message.contains("KiB"), "{e}");
            assert!(start.elapsed().as_secs() < 1);
        }
    }

    #[test]
    fn depth_counts_only_real_nesting() {
        let nested = |n: usize| format!("{}pane{}", "split {\n".repeat(n), "\n}".repeat(n));
        assert!(parse_layout(&nested(MAX_DEPTH)).is_ok());
        assert_eq!(parse_layout(&nested(MAX_DEPTH + 1)).unwrap_err().line, Some(MAX_DEPTH + 1));
        let quiet = [
            "\"{}}}\"",
            "#\"{}}\"\"#",
            "##\"\"#{}\"##",
            "\"\"\"\n{}\n\"\"\"",
            "\"\\\"{}\"",
            "// {}}\n",
            "/* /* {} */ */",
        ];
        for q in quiet {
            assert_eq!(too_deep(&format!("{q}\n{}", nested(MAX_DEPTH))), None, "{q}");
            assert!(too_deep(&format!("{q}\n{}", nested(MAX_DEPTH + 1))).is_some(), "{q}");
        }
        assert!(too_deep(&format!("\"{{\"\n{}", nested(MAX_DEPTH))).is_some());
        let tabs = "tab { pane command=\"awk '{print}'\" }\n".repeat(40);
        assert_eq!(parse_layout(&tabs).unwrap().len(), 40);
    }
}
