//! The crate graph enforces core → vt → shroud; this enforces the layers inside core
//! and keeps colour values out of everything but the theme.

use std::fs;
use std::path::{Path, PathBuf};

// Module → modules it may import from `crate::`. Anything else is an upward or sideways edge.
const ALLOWED: &[(&str, &[&str])] = &[
    ("layout", &["layout"]),
    ("render", &["render", "layout", "theme"]),
    ("theme", &["theme", "render"]),
    ("input", &["input", "layout"]),
    ("protocol", &["protocol"]),
    ("pace", &["pace"]),
    ("switcher", &["switcher", "overlay", "render", "theme", "input", "layout"]),
    ("shell", &["shell"]),
    ("statusbar", &["statusbar", "render", "theme"]),
    ("config", &["config", "input", "theme", "layout", "render"]),
    ("overlay", &["overlay", "render", "theme", "layout"]),
    ("visual", &["visual", "input", "render"]),
    ("watchdog", &["watchdog"]),
];

// Colour literals are allowed only where colours are defined or encoded, plus pane-cell translation.
const COLOUR_ALLOWED: &[&str] =
    &["core/src/theme.rs", "core/src/render/color.rs", "core/src/render/sgr.rs", "vt/src/snapshot.rs"];

#[test]
fn core_modules_only_import_downward() {
    let src = crate_dir("core").join("src");
    let mut violations = Vec::new();
    for file in rust_files(&src) {
        let rel = file.strip_prefix(&src).unwrap();
        let module = rel.components().next().unwrap().as_os_str().to_string_lossy().trim_end_matches(".rs").to_string();
        if module == "lib" {
            continue;
        }
        let allowed = ALLOWED
            .iter()
            .find(|(m, _)| *m == module)
            .map(|(_, a)| *a)
            .unwrap_or_else(|| panic!("{module} missing from ALLOWED"));
        for (n, target) in crate_imports(&non_test_lines(&file)) {
            if !allowed.contains(&target.as_str()) {
                violations.push(format!("{}:{n}: {module} imports {target}", rel.display()));
            }
        }
    }
    assert!(violations.is_empty(), "layer violations:\n{}", violations.join("\n"));
}

#[test]
fn colour_values_live_only_in_the_theme() {
    let crates = crate_dir("core").parent().unwrap().to_path_buf();
    let mut violations = Vec::new();
    for krate in ["core", "vt", "plugins", "shroud"] {
        for file in rust_files(&crates.join(krate).join("src")) {
            let rel = file.strip_prefix(&crates).unwrap().to_string_lossy().to_string();
            if COLOUR_ALLOWED.contains(&rel.as_str()) {
                continue;
            }
            for (n, line) in non_test_lines(&file) {
                if line.contains("Color::Palette(")
                    || line.contains("Color::Rgb(")
                    || line.contains("\\x1b[38;")
                    || line.contains("\\x1b[48;")
                {
                    violations.push(format!("{rel}:{n}: {}", line.trim()));
                }
            }
        }
    }
    assert!(violations.is_empty(), "colour values outside theme:\n{}", violations.join("\n"));
}

fn crate_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(name)
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|n| n != "ffi.rs") {
            out.push(path);
        }
    }
    out
}

// Relies on tests living at the bottom of each file.
fn non_test_lines(file: &Path) -> Vec<(usize, String)> {
    let text = fs::read_to_string(file).unwrap();
    text.lines()
        .enumerate()
        .take_while(|(_, l)| l.trim() != "#[cfg(test)]")
        .map(|(i, l)| (i + 1, l.to_string()))
        .collect()
}

/// The first module of every `crate::` path, with its line. Reads across lines, so a
/// `use crate::{a::X, b::Y}` that rustfmt split over several lines still counts.
fn crate_imports(lines: &[(usize, String)]) -> Vec<(usize, String)> {
    let mut text = String::new();
    let mut starts = Vec::new();
    for (n, l) in lines {
        starts.push((text.len(), *n));
        text.push_str(l);
        text.push('\n');
    }
    let line_of = |at: usize| starts.iter().rev().find(|(s, _)| *s <= at).map_or(0, |(_, n)| *n);
    let ident = |s: &str| {
        let s = s.trim_start();
        let end = s.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(s.len());
        s[..end].to_string()
    };
    let mut out = Vec::new();
    for (i, _) in text.match_indices("crate::") {
        let rest = &text[i + "crate::".len()..];
        let Some(group) = rest.strip_prefix('{') else {
            out.push((line_of(i), ident(rest)));
            continue;
        };
        let (mut depth, mut item) = (0, 0);
        for (j, c) in group.char_indices() {
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => {
                    out.push((line_of(i), ident(&group[item..j])));
                    break;
                }
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    out.push((line_of(i), ident(&group[item..j])));
                    item = j + 1;
                }
                _ => {}
            }
        }
    }
    out.retain(|(_, m)| !m.is_empty());
    out
}

#[test]
fn imports_are_read_across_lines() {
    let lines: Vec<(usize, String)> = [
        "use crate::layout::Rect;",
        "use crate::{",
        "    render::{cell::Cell, grid::Grid},",
        "    theme::Theme,",
        "};",
    ]
    .iter()
    .enumerate()
    .map(|(i, l)| (i + 1, l.to_string()))
    .collect();
    let found: Vec<(usize, String)> = crate_imports(&lines);
    let want = [(1, "layout"), (2, "render"), (2, "theme")].map(|(n, m)| (n, m.to_string()));
    assert_eq!(found, want);
}
