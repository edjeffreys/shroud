#!/usr/bin/env bash
# Writes a formula for the prebuilt binaries in dist/ into edjeffreys/homebrew-tap and pushes it,
# along with the tap's README. Run by .github/workflows/release.yml:
#
#   update-tap.sh shroud     <version> <release tag> <asset prefix>   after a release
#   update-tap.sh shroud-dev <version> <release tag> <asset prefix>   after a dev build
#
# Needs TOKEN (write access to the tap) and GITHUB_REPOSITORY. With TAP_DIR set, it writes
# into that directory and skips git, to try it locally.
set -euo pipefail

name=$1 version=$2 tag=$3 prefix=$4
repo=${GITHUB_REPOSITORY:?}

case $name in
  shroud)
    class=Shroud other=shroud-dev
    desc="Terminal multiplexer for modern terminals, built on libghostty-vt"
    ;;
  shroud-dev)
    class=ShroudDev other=shroud
    desc="Terminal multiplexer for modern terminals (dev build of main)"
    ;;
  *) echo "unknown formula $name" >&2; exit 2 ;;
esac

sum() { cut -d' ' -f1 "dist/$prefix-$1.tar.gz.sha256"; }
url() { echo "https://github.com/$repo/releases/download/$tag/$prefix-$1.tar.gz"; }

tap=${TAP_DIR:-tap}
if [[ -z ${TAP_DIR:-} ]]; then
  git clone -q "https://x-access-token:${TOKEN:?}@github.com/edjeffreys/homebrew-tap.git" "$tap"
fi
mkdir -p "$tap/Formula"

{
  cat <<RUBY
# Written by shroud's release workflow; changes here are overwritten at the next publish.
class $class < Formula
  desc "$desc"
  homepage "https://github.com/$repo"
  version "$version"
  license "MIT"

  on_macos do
    on_arm do
      url "$(url aarch64-apple-darwin)"
      sha256 "$(sum aarch64-apple-darwin)"
    end
    on_intel do
      url "$(url x86_64-apple-darwin)"
      sha256 "$(sum x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "$(url aarch64-unknown-linux-gnu)"
      sha256 "$(sum aarch64-unknown-linux-gnu)"
    end
    on_intel do
      url "$(url x86_64-unknown-linux-gnu)"
      sha256 "$(sum x86_64-unknown-linux-gnu)"
    end
  end

  conflicts_with "$other", because: "both install a shroud binary"
RUBY
  if [[ $name == shroud ]]; then
    cat <<RUBY

  head do
    url "https://github.com/$repo.git", branch: "main"
    depends_on "rust" => :build
    depends_on "zig" => :build
  end

  def install
    if build.head?
      # Homebrew's sandbox allows writes only inside the build directory.
      ENV["ZIG_GLOBAL_CACHE_DIR"] = buildpath/".zig-cache"
      system "cargo", "install", *std_cargo_args(path: "crates/shroud")
    else
      bin.install "shroud"
    end
  end
RUBY
  else
    cat <<RUBY

  def install
    bin.install "shroud"
  end
RUBY
  fi
  cat <<'RUBY'

  test do
    assert_match "shroud ", shell_output("#{bin}/shroud --version")
  end
end
RUBY
} > "$tap/Formula/$name.rb"

cat > "$tap/README.md" <<'MD'
# edjeffreys/tap

Homebrew formulae for [shroud](https://github.com/edjeffreys/shroud), written by its release workflow.

Homebrew asks you to trust a third-party formula before it loads it:

```sh
brew trust --formula edjeffreys/tap/shroud
brew install edjeffreys/tap/shroud
```

| Formula | What you get |
|---|---|
| `shroud` | the latest release |
| `shroud-dev` | the newest `main`, prebuilt whenever its CI passes |
| `shroud --HEAD` | the newest `main`, built from source (needs Rust and Zig, which brew installs) |

`shroud` and `shroud-dev` both install `shroud`, so have one at a time. In a Brewfile:

```ruby
tap "edjeffreys/tap"
brew "edjeffreys/tap/shroud", trusted: true   # or "edjeffreys/tap/shroud-dev"
```
MD

cat "$tap/Formula/$name.rb"
[[ -n ${TAP_DIR:-} ]] && exit 0

cd "$tap"
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git add "Formula/$name.rb" README.md
git diff --cached --quiet && exit 0
git commit -q -m "$name $version"
# A release and a dev build can publish at once; each touches its own formula, so rebasing is safe.
for attempt in 1 2 3; do
  git push -q origin HEAD:main && exit 0
  git pull -q --rebase origin main
done
echo "couldn't push to the tap" >&2
exit 1
