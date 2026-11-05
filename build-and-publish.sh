#!/usr/bin/env bash
# Builds a signed, notarized Session Player .dmg and publishes it as a GitHub
# release tagged v<version>, using the version in package.json (tauri.conf.json
# reads it from there).
#
# Usage: ./build-and-publish.sh [patch|minor|major|<x.y.z>] [--no-publish]
#   patch|minor|major|<x.y.z>   bump the version with `npm version` and commit
#                               the bump first; omit to release the current one
#   --no-publish                build, notarize and verify only; skip the push
#                               and release
#
# One-time setup:
#   1. A "Developer ID Application" certificate in your login keychain
#      (Xcode > Settings > Accounts > Manage Certificates).
#   2. An app-specific password (account.apple.com) saved in the keychain:
#        security add-generic-password -a <apple-id-email> -s session-player-notary -w
#   3. gh authenticated for this repository (gh auth status).
#   4. An update signing key. Installed apps only accept updates signed with it,
#      so back up the key file and its password:
#        npx tauri signer generate -w ~/.tauri/session-player.key
#        security add-generic-password -a updater -s session-player-updater -w
#      (enter the key's password at the second command's prompt), then put the
#      contents of ~/.tauri/session-player.key.pub in plugins.updater.pubkey in
#      src-tauri/tauri.conf.json.
#
# The Apple ID comes from that keychain item and the Team ID from the
# certificate, so neither is stored in this repository. Override with
# APPLE_SIGNING_IDENTITY, APPLE_ID or APPLE_TEAM_ID if needed.

set -euo pipefail

KEYCHAIN_SERVICE="session-player-notary"
UPDATER_KEYCHAIN_SERVICE="session-player-updater"
UPDATER_KEY_FILE="$HOME/.tauri/session-player.key"
PUBLISH=1
BUMP=""

for arg in "$@"; do
  case "$arg" in
    --no-publish) PUBLISH=0 ;;
    patch | minor | major | [0-9]*.[0-9]*.[0-9]*) BUMP="$arg" ;;
    -h | --help)
      sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 1
      ;;
  esac
done

cd "$(dirname "$0")"

step() { printf '\n==> %s\n' "$*"; }
die() {
  echo "Error: $*" >&2
  exit 1
}

for tool in jq gh xcrun security npx; do
  command -v "$tool" >/dev/null || die "$tool is not installed"
done

# Publishing must not ship code that isn't committed.
[ -z "$(git status --porcelain)" ] || die "the working tree has uncommitted changes"
if [ "$PUBLISH" = 1 ]; then
  [ "$(git branch --show-current)" = "main" ] || die "publish from main"
fi

tag_exists() {
  git rev-parse -q --verify "refs/tags/$1" >/dev/null || gh release view "$1" >/dev/null 2>&1
}

if [ -n "$BUMP" ]; then
  step "Bumping version ($BUMP)"
  # npm's own commit and tag are skipped: the release below creates the tag.
  npm version "$BUMP" --no-git-tag-version >/dev/null
  VERSION=$(jq -r .version package.json)
  if tag_exists "v$VERSION"; then
    git checkout HEAD -- package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock
    die "v$VERSION already exists"
  fi
  git commit -q -m "Release $VERSION" -- package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock
  echo "Committed version $VERSION"
fi

VERSION=$(jq -r .version package.json)
TAG="v$VERSION"
DMG="src-tauri/target/release/bundle/dmg/Session Player_${VERSION}_aarch64.dmg"
UPDATE_ARCHIVE="src-tauri/target/release/bundle/macos/Session Player.app.tar.gz"
UPDATE_MANIFEST="src-tauri/target/release/bundle/macos/latest.json"

step "Checking release $TAG"
if [ "$PUBLISH" = 1 ] && tag_exists "$TAG"; then
  die "$TAG already exists; pass patch, minor or major to bump the version"
fi

step "Reading signing credentials"
if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  APPLE_SIGNING_IDENTITY=$(security find-identity -v -p codesigning |
    sed -n 's/.*"\(Developer ID Application: .*\)".*/\1/p' | head -1)
fi
[ -n "$APPLE_SIGNING_IDENTITY" ] || die "no Developer ID Application certificate found in the keychain"
# The Team ID is the code in parentheses at the end of the certificate name.
APPLE_TEAM_ID=${APPLE_TEAM_ID:-$(printf '%s' "$APPLE_SIGNING_IDENTITY" | sed -n 's/.*(\([A-Z0-9]*\))$/\1/p')}
APPLE_ID=${APPLE_ID:-$(security find-generic-password -s "$KEYCHAIN_SERVICE" 2>/dev/null |
  sed -n 's/.*"acct"<blob>="\(.*\)"/\1/p')}
[ -n "$APPLE_ID" ] || die "no keychain item named $KEYCHAIN_SERVICE (see setup in this script)"
APPLE_PASSWORD=$(security find-generic-password -s "$KEYCHAIN_SERVICE" -w)
echo "Signing as: $APPLE_SIGNING_IDENTITY"
export APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID APPLE_PASSWORD

[ -f "$UPDATER_KEY_FILE" ] || die "no update signing key at $UPDATER_KEY_FILE (see setup in this script)"
UPDATER_PUBKEY=$(jq -r .plugins.updater.pubkey src-tauri/tauri.conf.json)
[ "$UPDATER_PUBKEY" = "$(cat "$UPDATER_KEY_FILE.pub")" ] ||
  die "plugins.updater.pubkey in tauri.conf.json does not match $UPDATER_KEY_FILE.pub"
TAURI_SIGNING_PRIVATE_KEY=$(cat "$UPDATER_KEY_FILE")
TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(security find-generic-password -s "$UPDATER_KEYCHAIN_SERVICE" -w) ||
  die "no keychain item named $UPDATER_KEYCHAIN_SERVICE (see setup in this script)"
export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD

step "Building, signing and notarizing the app"
npm ci --ignore-scripts
# Update archives are turned on here, not in tauri.conf.json, so that `npm run bundle` keeps
# working on machines without the signing key.
npx tauri build --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":true}}'
[ -f "$DMG" ] || die "expected $DMG after the build"
[ -f "$UPDATE_ARCHIVE" ] || die "expected $UPDATE_ARCHIVE after the build"
[ -f "$UPDATE_ARCHIVE.sig" ] || die "expected $UPDATE_ARCHIVE.sig after the build"

# Tauri notarizes the .app inside the disk image; the disk image itself needs
# its own notarization so macOS opens a downloaded copy without a warning.
step "Notarizing the disk image"
xcrun notarytool submit "$DMG" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD" --wait
xcrun stapler staple "$DMG"

step "Verifying"
spctl -a -vvv -t open --context context:primary-signature "$DMG"
# Installed apps unpack this archive in place of themselves without any Gatekeeper
# prompt, so a copy that isn't notarized must not reach them.
UPDATE_CHECK=$(mktemp -d)
trap 'rm -rf "$UPDATE_CHECK"' EXIT
tar -xzf "$UPDATE_ARCHIVE" -C "$UPDATE_CHECK"
spctl -a -vvv -t exec "$UPDATE_CHECK/Session Player.app"
xcrun stapler validate "$UPDATE_CHECK/Session Player.app"

# gh replaces the spaces in an uploaded file's name with dots.
jq -n \
  --arg version "$VERSION" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg url "https://github.com/danmactough/session-player/releases/download/$TAG/Session.Player.app.tar.gz" \
  --rawfile signature "$UPDATE_ARCHIVE.sig" \
  '{version: $version, pub_date: $pub_date, platforms: {"darwin-aarch64": {url: $url, signature: ($signature | rtrimstr("\n"))}}}' \
  >"$UPDATE_MANIFEST"

if [ "$PUBLISH" = 0 ]; then
  step "Done (not published): $DMG"
  exit 0
fi

step "Publishing $TAG"
git push origin main
# Installed apps find this release through latest.json, so all three files go up together.
gh release create "$TAG" "$DMG" "$UPDATE_ARCHIVE" "$UPDATE_MANIFEST" --target main --title "Session Player $VERSION" --generate-notes

step "Published: $(gh release view "$TAG" --json url -q .url)"
