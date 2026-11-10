//! Builds libghostty-vt from its verified release tarball.
//! SHROUD_LIBVT_DIR=<prefix> skips that, for offline or packaged builds.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION: &str = "1.0.0";
// From the release's minisign-signed SHA256SUMS.
const SHA256: &str = "4a6ef758be721923e1969e7b81b443dcc92d8dd3fb5a39600c03a8817a753676";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SHROUD_LIBVT_DIR");

    let prefix = match env::var_os("SHROUD_LIBVT_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => build_from_source(),
    };

    // ld64 prefers a dylib over an archive in the same directory, whatever `static=` says.
    let only_static = PathBuf::from(env::var("OUT_DIR").unwrap()).join("libvt-static");
    std::fs::create_dir_all(&only_static).expect("create static lib dir");
    std::fs::copy(prefix.join("lib/libghostty-vt.a"), only_static.join("libghostty-vt.a"))
        .expect("copy libghostty-vt.a");
    println!("cargo:rustc-link-search=native={}", only_static.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    // The fat archive bundles simdutf and highway, which are C++.
    if env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        println!("cargo:rustc-link-lib=c++");
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }
}

fn build_from_source() -> PathBuf {
    // Shared across profiles so debug, release and test don't each spend ~40s on it.
    let cache = target_dir().join(format!("libvt-{VERSION}"));
    let prefix = cache.join("prefix");
    if prefix.join("lib/libghostty-vt.a").exists() {
        return prefix;
    }

    std::fs::create_dir_all(&cache).expect("create libvt cache dir");
    let tarball = cache.join(format!("libghostty-vt-{VERSION}.tar.gz"));
    if !tarball.exists() {
        let url = format!("https://release.files.ghostty.org/libghostty-vt/{VERSION}/libghostty-vt-{VERSION}.tar.gz");
        run(Command::new("curl").args(["-sSfL", "-o"]).arg(&tarball).arg(&url), "download libghostty-vt");
    }
    verify(&tarball);

    run(Command::new("tar").arg("xzf").arg(&tarball).arg("-C").arg(&cache), "unpack libghostty-vt");
    let src = cache.join(format!("libghostty-vt-{VERSION}"));

    // Built optimised regardless of cargo profile: a debug libvt makes every debug run crawl.
    let staging = cache.join("prefix.tmp");
    let _ = std::fs::remove_dir_all(&staging);
    let status = Command::new("zig")
        .args(["build", "-Doptimize=ReleaseFast", "--prefix"])
        .arg(&staging)
        .current_dir(&src)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => panic!("zig build of libghostty-vt failed ({s})"),
        Err(e) => panic!("could not run zig ({e}). libghostty-vt {VERSION} needs Zig 0.17.x: brew install zig"),
    }
    // Atomic, so an interrupted build is never reused.
    std::fs::rename(&staging, &prefix).expect("install libvt prefix");
    prefix
}

fn verify(tarball: &Path) {
    let out = Command::new("shasum").args(["-a", "256"]).arg(tarball).output().expect("run shasum");
    let got = String::from_utf8_lossy(&out.stdout);
    let got = got.split_whitespace().next().unwrap_or("");
    if got != SHA256 {
        let _ = std::fs::remove_file(tarball);
        panic!("libghostty-vt tarball checksum mismatch: got {got}, want {SHA256}");
    }
}

fn target_dir() -> PathBuf {
    // Cargo doesn't expose the target dir to build scripts.
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    out.ancestors().nth(4).map(Path::to_path_buf).unwrap_or(out)
}

fn run(cmd: &mut Command, what: &str) {
    let status = cmd.status().unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(status.success(), "{what} failed ({status})");
}
