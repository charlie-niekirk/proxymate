#!/usr/bin/env bash
# Builds the universal (arm64 + x86_64) macOS release, signs it, notarises
# and staples it when credentials are present, checks the result, and stages
# what a GitHub release needs in $OUT_DIR:
#
#   Proxymate_<version>_universal.dmg
#   Proxymate_<version>_universal.app.tar.gz(.sig)   updater bundle + signature
#   updater-darwin.json                               latest.json `platforms` entries
#
# Credentials are Tauri's bundler environment variables (docs/RELEASING.md):
#   APPLE_SIGNING_IDENTITY, APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD
#   APPLE_API_KEY, APPLE_API_ISSUER, APPLE_API_KEY_PATH          (notarytool)
#   TAURI_SIGNING_PRIVATE_KEY, TAURI_SIGNING_PRIVATE_KEY_PASSWORD (updater)
# Without a signing identity the app is ad-hoc signed and not notarised;
# without the updater key no updater bundle is made. Both are reported.
#
# Other settings:
#   OUT_DIR       where to stage (default target/release/macos)
#   RELEASE_TAG   if set, must be v<version>
#   DOWNLOAD_URL  base URL of the release's assets, for updater-darwin.json
#                 (default: the GitHub release of v<version>)
#   PROXYMATE_ALLOW_UNSIGNED_DMG=1   don't fail when the DMG has no Developer ID signature
# Extra arguments go to `tauri build` (e.g. --config overrides).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CONF="$ROOT/apps/desktop/src-tauri/tauri.conf.json"
TARGET=universal-apple-darwin
BUNDLE="$ROOT/target/$TARGET/release/bundle"
OUT_DIR=${OUT_DIR:-$ROOT/target/release/macos}

note() { printf '\n==> %s\n' "$*"; }
warn() {
  # GitHub Actions turns this into an annotation; locally it's a plain line.
  if [[ -n ${GITHUB_ACTIONS:-} ]]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}
die() {
  if [[ -n ${GITHUB_ACTIONS:-} ]]; then echo "::error::$*"; else echo "error: $*" >&2; fi
  exit 1
}

VERSION=$(node -p "require(process.argv[1]).version" "$CONF")
if [[ -n ${RELEASE_TAG:-} && $RELEASE_TAG != "v$VERSION" ]]; then
  die "tag $RELEASE_TAG doesn't match the app version $VERSION (tauri.conf.json); bump the version first"
fi
DOWNLOAD_URL=${DOWNLOAD_URL:-https://github.com/charlie-niekirk/proxymate/releases/download/v$VERSION}

# ---- What this build can do ----
IDENTITY=${APPLE_SIGNING_IDENTITY:--}
export APPLE_SIGNING_IDENTITY=$IDENTITY
NOTARISE=0
if [[ $IDENTITY == "-" ]]; then
  warn "no APPLE_SIGNING_IDENTITY: the app is ad-hoc signed and NOT notarised. Gatekeeper blocks it on other Macs until the user allows it (docs/RELEASING.md)."
  # Tauri would try to notarise an ad-hoc app if these were set.
  unset APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID
elif [[ -n ${APPLE_API_KEY:-} && -n ${APPLE_API_ISSUER:-} && -n ${APPLE_API_KEY_PATH:-} ]]; then
  NOTARISE=1
else
  warn "signing as \"$IDENTITY\" but APPLE_API_KEY/APPLE_API_ISSUER/APPLE_API_KEY_PATH are missing: NOT notarised."
fi
UPDATER=0
config=()
if [[ -n ${TAURI_SIGNING_PRIVATE_KEY:-} ]]; then
  UPDATER=1
  config=(--config '{"bundle":{"createUpdaterArtifacts":true}}')
else
  warn "no TAURI_SIGNING_PRIVATE_KEY: no updater bundle or updater-darwin.json."
fi

# ---- Build (Tauri signs the binary and the app, notarises and staples the app, signs the DMG) ----
note "Building Proxymate $VERSION for $TARGET (identity: $IDENTITY, notarise: $NOTARISE, updater: $UPDATER)"
# A leftover DMG (an interrupted build) makes hdiutil fail with "File exists".
rm -rf "$BUNDLE/dmg" "$BUNDLE/macos"
# CI=true also skips the DMG's Finder layout script, which needs Automation access.
(cd "$ROOT" && CI=true npm run tauri -- build --ci --target "$TARGET" --bundles app,dmg ${config[@]+"${config[@]}"} "$@")

APP="$BUNDLE/macos/Proxymate.app"
DMG="$BUNDLE/dmg/Proxymate_${VERSION}_universal.dmg"
EXE="$APP/Contents/MacOS/proxymate"
[[ -d $APP ]] || die "no app bundle at $APP"
[[ -f $DMG ]] || die "no DMG at $DMG"

# ---- Notarise and staple the DMG (Tauri only does the app inside it) ----
if [[ $NOTARISE == 1 ]]; then
  note "Notarising the DMG"
  xcrun notarytool submit "$DMG" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" \
    --issuer "$APPLE_API_ISSUER" --wait --timeout 30m
  xcrun stapler staple "$DMG"
fi

# ---- Check what we ship ----
note "Checking the bundle"
archs=$(lipo -archs "$EXE")
[[ $archs == *x86_64* && $archs == *arm64* ]] || die "$EXE is not universal: $archs"
echo "architectures: $archs"
# Every Mach-O in the bundle must be signed; today that's only the main
# binary, which also runs as `--sysproxy-guardian` and `mcp-stdio`.
while IFS= read -r file; do
  if [[ $(file -b "$file") == *Mach-O* ]]; then
    codesign --verify --strict --verbose=2 "$file"
    # (Captured first: `grep -q` exiting early would fail the pipe under pipefail.)
    info=$(codesign -dv "$file" 2>&1)
    [[ $info == *flags=*runtime* ]] || die "$file is signed without the hardened runtime"
  fi
done < <(find "$APP" -type f)
codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv --verbose=2 "$APP" 2>&1 | grep -E '^(Identifier|Authority|TeamIdentifier|Timestamp|Runtime Version|CodeDirectory)' || true
if [[ $IDENTITY != "-" ]]; then
  dmg_info=$(codesign -dv --verbose=2 "$DMG" 2>&1 || true)
  if [[ $dmg_info == *"Authority=Developer ID Application"* ]]; then
    codesign --verify --strict --verbose=2 "$DMG"
  elif [[ ${PROXYMATE_ALLOW_UNSIGNED_DMG:-0} != 1 ]]; then
    die "the DMG isn't signed with the Developer ID"
  fi
fi
if [[ $NOTARISE == 1 ]]; then
  xcrun stapler validate "$APP"
  xcrun stapler validate "$DMG"
  spctl -a -vv -t exec "$APP"
  spctl -a -vv -t open --context context:primary-signature "$DMG"
fi

# ---- Stage ----
note "Staging in $OUT_DIR"
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
cp "$DMG" "$OUT_DIR/"
if [[ $UPDATER == 1 ]]; then
  archive="Proxymate_${VERSION}_universal.app.tar.gz"
  cp "$BUNDLE/macos/Proxymate.app.tar.gz" "$OUT_DIR/$archive"
  cp "$BUNDLE/macos/Proxymate.app.tar.gz.sig" "$OUT_DIR/$archive.sig"
  # The updater looks up `darwin-<arch>`; one universal bundle serves both.
  node -e '
    const [sigFile, url, out] = process.argv.slice(1);
    const entry = { signature: require("fs").readFileSync(sigFile, "utf8").trim(), url };
    require("fs").writeFileSync(out, JSON.stringify({ "darwin-aarch64": entry, "darwin-x86_64": entry }, null, 2) + "\n");
  ' "$OUT_DIR/$archive.sig" "$DOWNLOAD_URL/$archive" "$OUT_DIR/updater-darwin.json"
fi
ls -l "$OUT_DIR"
