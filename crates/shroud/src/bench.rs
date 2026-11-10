//! `shroud bench`: the server's hot path run in-process, with no PTY or socket, so runs
//! repeat and `--check` can gate CI. Numbers from a debug build mean little.

use std::io;
use std::time::{Duration, Instant};

use shroud_core::layout::Rect;
use shroud_core::render::{Frame, Grid, Links, PaneView, compose, diff};
use shroud_core::theme::Theme;
use shroud_vt::{Snapshot, Terminal};

pub const USAGE: &str =
    "  shroud bench [--mb <n>] [--check]  time parsing and frames in-process; --check fails below the floors";

// Mixed ASCII, colour, wide characters and an emoji cluster: closer to real output than noise.
const LINE: &str =
    "plain text 0123456789 \x1b[31mred\x1b[0m \x1b[1;38;2;10;200;90mtruecolor\x1b[0m 世界 emoji ✨👍🏽 tabs\tend\r\n";
const COLS: u16 = 200;
const ROWS: u16 = 50;
const FRAMES: usize = 300;
const CHUNK: usize = 64 * 1024;

// Floors, loose enough for a busy CI machine; a frame must fit well inside 120 fps.
const MIN_PARSE_MB_S: f64 = 40.0;
const MAX_FRAME_P99: Duration = Duration::from_millis(4);
// An echoed key changes one cell, so its frame must cost far less than a full screen.
const MAX_ECHO_P99: Duration = Duration::from_millis(1);

pub fn run(args: &[&str]) -> io::Result<i32> {
    let mut mb = 64usize;
    let mut check = false;
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "--check" => check = true,
            "--mb" => {
                mb = it
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or_else(|| io::Error::other("--mb needs a positive number"))?
            }
            _ => {
                eprintln!("usage:\n{USAGE}");
                return Ok(2);
            }
        }
    }
    let release = !cfg!(debug_assertions);
    if check && !release {
        eprintln!("shroud: bench --check needs a release build (cargo build --release)");
        return Ok(2);
    }
    println!("shroud bench ({}, {mb} MB, {COLS}×{ROWS})", if release { "release" } else { "debug" });
    let parse = parse_rate(mb).map_err(vt_err)?;
    println!("  parse  {parse:7.1} MB/s");
    let f = frames(|term, i| {
        for row in 0..ROWS as usize {
            term.write(&LINE.as_bytes()[(i + row) % 20..]);
        }
    })
    .map_err(vt_err)?;
    println!(
        "  frame  p50 {}  p99 {}  max {}  ({:.1} KB written per frame)",
        ms(f.p50),
        ms(f.p99),
        ms(f.max),
        f.bytes as f64 / 1024.0
    );
    let screen = LINE.repeat(ROWS as usize);
    let e = frames(|term, i| term.write(if i == 0 { screen.as_bytes() } else { b"x" })).map_err(vt_err)?;
    println!(
        "  echo   p50 {}  p99 {}  max {}  ({:.0} B written per frame)",
        ms(e.p50),
        ms(e.p99),
        ms(e.max),
        e.bytes as f64
    );
    if !check {
        return Ok(0);
    }
    let mut ok = true;
    if parse < MIN_PARSE_MB_S {
        println!("FAIL parse is below {MIN_PARSE_MB_S} MB/s");
        ok = false;
    }
    if f.p99 > MAX_FRAME_P99 {
        println!("FAIL frame p99 is above {}", ms(MAX_FRAME_P99));
        ok = false;
    }
    if e.p99 > MAX_ECHO_P99 {
        println!("FAIL echo p99 is above {}", ms(MAX_ECHO_P99));
        ok = false;
    }
    if ok {
        println!("ok");
    }
    Ok(i32::from(!ok))
}

struct Frames {
    p50: Duration,
    p99: Duration,
    max: Duration,
    bytes: usize,
}

fn parse_rate(mb: usize) -> Result<f64, shroud_vt::Error> {
    let stream = LINE.repeat(mb * 1024 * 1024 / LINE.len() + 1);
    let stream = &stream.as_bytes()[..mb * 1024 * 1024];
    let mut term = Terminal::new(COLS, ROWS)?;
    let mut effects = Vec::new();
    let start = Instant::now();
    for chunk in stream.chunks(CHUNK) {
        term.write(chunk);
        term.drain_effects(&mut effects);
        effects.clear();
    }
    Ok(mb as f64 / start.elapsed().as_secs_f64())
}

fn frames(mut output: impl FnMut(&mut Terminal, usize)) -> Result<Frames, shroud_vt::Error> {
    let mut term = Terminal::new(COLS, ROWS)?;
    let mut snap = Snapshot::new()?;
    let mut links = Links::default();
    let mut grid = Grid::new(COLS, ROWS);
    let theme = Theme::default();
    let area = Rect { x: 0, y: 0, w: COLS, h: ROWS };
    let (mut prev, mut next) = (None::<Frame>, Frame::new(COLS, ROWS));
    let mut out = Vec::new();
    let (mut times, mut bytes) = (Vec::with_capacity(FRAMES), 0);
    for i in 0..FRAMES {
        output(&mut term, i);
        let start = Instant::now();
        snap.update(&term, &mut grid, &mut links)?;
        let cursor = snap.cursor()?;
        compose(
            &mut next,
            area,
            &[PaneView { rect: area, grid: &grid, cursor, float: None, highlight: None, title: None, matches: &[] }],
            Some(0),
            &theme,
        );
        out.clear();
        diff(&mut out, prev.as_ref(), &next, &links);
        times.push(start.elapsed());
        bytes += out.len();
        match &mut prev {
            Some(p) => std::mem::swap(p, &mut next),
            None => prev = Some(next.clone()),
        }
    }
    times.sort_unstable();
    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    Ok(Frames { p50: at(0.5), p99: at(0.99), max: at(1.0), bytes: bytes / FRAMES })
}

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

fn vt_err(e: shroud_vt::Error) -> io::Error {
    io::Error::other(format!("libghostty-vt error {}", e.0))
}
