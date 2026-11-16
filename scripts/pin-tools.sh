#!/usr/bin/env bash
# Pins the ffmpeg PolishCast downloads on first use to its latest release,
# and writes crates/export/src/tools.pins:
#
#   scripts/pin-tools.sh
#
# A copy of TrackCut's script: keep the two in step.
#
# Each line is `tool os arch sha256 url` for one platform the app runs on. The
# app installs a download only if it hashes to the line's sha256, so nothing
# the servers send later can change what runs.
#
# Every hash is read from what the project publishes beside the file. For files
# on GitHub it must also equal GitHub's own digest of the uploaded asset, which
# the publisher's sum file can't fake; any disagreement stops the script and
# leaves tools.pins as it was. ffmpeg for macOS and Linux comes from
# ffmpeg.martin-riedl.de, which has no second source: its sum is pinned as read.
#
# Needs gh (signed in) and curl. Review the diff before committing it.
set -euo pipefail

cd "$(dirname "$0")/.."
# The only two lines that differ between TrackCut's copy and PolishCast's.
ALL=(ffmpeg)
OUT=crates/export/src/tools.pins

TOOLS=("$@")
[ ${#TOOLS[@]} -eq 0 ] && TOOLS=("${ALL[@]}")

die() { echo "pin-tools: $*" >&2; exit 1; }
wants() { local t; for t in "${TOOLS[@]}"; do [ "$t" = "$1" ] && return 0; done; return 1; }
hash_in() { tr -d '\r' | grep -oiE '\b[0-9a-f]{64}\b' | head -1 | tr 'A-F' 'a-f'; }

# GitHub's digest of one release asset, without the "sha256:" prefix.
gh_digest() {
  local d
  d=$(gh api "repos/$1/releases/tags/$2" --jq ".assets[] | select(.name == \"$3\") | .digest")
  [ -n "$d" ] || die "$1 $2 has no asset $3"
  echo "${d#sha256:}"
}

# Prints one pin line after checking the published sum against GitHub's digest.
github_pin() { # tool os arch repo tag asset published_sum
  local digest
  digest=$(gh_digest "$4" "$5" "$6")
  [ "$digest" = "$7" ] || die "$6: published sum $7 is not GitHub's digest $digest"
  echo "$1 $2 $3 $7 https://github.com/$4/releases/download/$5/$6"
}

keep_old() { # tool: the lines already pinned for it
  [ -f "$OUT" ] && grep -E "^$1 " "$OUT" || true
}

pin_yt_dlp() {
  local repo=yt-dlp/yt-dlp tag sums
  tag=$(gh api "repos/$repo/releases/latest" --jq .tag_name)
  sums=$(curl -fsSL "https://github.com/$repo/releases/download/$tag/SHA2-256SUMS")
  local spec os arch asset sum
  # macOS builds are universal; Windows on ARM runs the x64 build emulated.
  for spec in macos:x86_64:yt-dlp_macos macos:aarch64:yt-dlp_macos \
    windows:x86_64:yt-dlp.exe windows:aarch64:yt-dlp.exe \
    linux:x86_64:yt-dlp_linux linux:aarch64:yt-dlp_linux_aarch64; do
    IFS=: read -r os arch asset <<<"$spec"
    sum=$(awk -v a="$asset" '$2 == a { print tolower($1) }' <<<"$sums")
    [ -n "$sum" ] || die "yt-dlp $tag publishes no sum for $asset"
    github_pin yt-dlp "$os" "$arch" "$repo" "$tag" "$asset" "$sum"
  done
}

pin_deno() {
  local repo=denoland/deno tag
  tag=$(gh api "repos/$repo/releases/latest" --jq .tag_name)
  local os target arch asset sum
  for os in macos windows linux; do
    case $os in
      macos) target=apple-darwin ;;
      windows) target=pc-windows-msvc ;;
      linux) target=unknown-linux-gnu ;;
    esac
    for arch in x86_64 aarch64; do
      asset="deno-$arch-$target.zip"
      sum=$(curl -fsSL "https://github.com/$repo/releases/download/$tag/$asset.sha256sum" | hash_in)
      [ -n "$sum" ] || die "deno $tag publishes no sum for $asset"
      github_pin deno "$os" "$arch" "$repo" "$tag" "$asset" "$sum"
    done
  done
}

pin_ffmpeg() {
  # Windows: yt-dlp's builds. Daily builds are deleted after a month or so;
  # the last build of each month is kept, so that is the one pinned.
  local repo=yt-dlp/FFmpeg-Builds tag month sums
  month=$(date -u +%Y-%m)
  tag=$(gh api "repos/$repo/releases?per_page=100" --paginate --jq '.[].tag_name' |
    grep -E '^autobuild-[0-9]{4}-[0-9]{2}-' | grep -v "^autobuild-$month-" | head -1)
  [ -n "$tag" ] || die "no month-end FFmpeg build in $repo"
  sums=$(curl -fsSL "https://github.com/$repo/releases/download/$tag/checksums.sha256")
  local arch build asset sum
  for arch in x86_64 aarch64; do
    case $arch in x86_64) build=win64 ;; aarch64) build=winarm64 ;; esac
    asset=$(awk -v b="-$build-gpl.zip" '$2 ~ /^ffmpeg-N-/ && substr($2, length($2) - length(b) + 1) == b { print $2 }' <<<"$sums")
    [ "$(wc -l <<<"$asset")" -eq 1 ] && [ -n "$asset" ] || die "$tag: no single $build GPL build"
    sum=$(awk -v a="$asset" '$2 == a { print tolower($1) }' <<<"$sums")
    github_pin ffmpeg windows "$arch" "$repo" "$tag" "$asset" "$sum"
  done

  # macOS and Linux: Martin Riedl's release builds (FFmpeg publishes none).
  # The newest release build per platform, as the download page lists them.
  local site=https://ffmpeg.martin-riedl.de page os riedl path
  page=$(curl -fsSL "$site/")
  for os in macos linux; do
    for arch in x86_64 aarch64; do
      case $arch in x86_64) riedl=amd64 ;; aarch64) riedl=arm64 ;; esac
      path=$(grep -oE "/download/$os/$riedl/[0-9]+_[0-9][0-9.]*/ffmpeg\.zip" <<<"$page" |
        sort -t/ -k5,5n | tail -1)
      [ -n "$path" ] || die "no ffmpeg release build for $os/$riedl on $site"
      sum=$(curl -fsSL "$site$path.sha256" | hash_in)
      [ -n "$sum" ] || die "$site$path has no sum"
      echo "ffmpeg $os $arch $sum $site$path"
    done
  done
}

lines=()
for tool in "${ALL[@]}"; do
  if wants "$tool"; then
    block=$("pin_${tool//-/_}")
  else
    block=$(keep_old "$tool")
  fi
  [ -n "$block" ] || die "nothing pinned for $tool"
  lines+=("$block")
done

{
  echo "# Written by scripts/pin-tools.sh: tool os arch sha256 url. Do not edit by hand."
  printf '%s\n' "${lines[@]}"
} >"$OUT"
echo "pinned in $OUT:"
cat "$OUT"
