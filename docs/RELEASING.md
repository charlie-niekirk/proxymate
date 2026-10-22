# Releasing Proxymate (macOS)

A `v*` tag builds a universal (Apple silicon + Intel) `Proxymate.app` under the hardened runtime, **ad-hoc signed**, and
uploads it with the updater files to a **draft** GitHub Release. Publishing that draft is the release: it is also what
the in-app updater sees.

Proxymate has no paid Apple Developer account, so builds are **not signed with a Developer ID and not notarised**. Users
allow the app past Gatekeeper once (the draft's notes explain how, from `scripts/release/install-notes-macos.md`), and
after that updates install from inside the app. Developer ID signing and notarisation are wired up but dormant: see
"Later: Developer ID and notarisation".

| Piece | Where |
|---|---|
| Workflow | `.github/workflows/release.yml` |
| Build, sign, check, stage | `scripts/release/macos.sh` (runs the same locally) |
| Install notes (Gatekeeper override) | `scripts/release/install-notes-macos.md`, added to every ad-hoc draft |
| Bundle config | `apps/desktop/src-tauri/tauri.conf.json` (`bundle`, `bundle.macOS`, `plugins.updater`) |
| Entitlements | `apps/desktop/src-tauri/Entitlements.plist` (empty on purpose, see below) |
| Updater | `apps/desktop/src-tauri/src/updater.rs`; the setting is Settings › Updates |

Release assets:
- `Proxymate_<version>_universal.dmg`: what people download.
- `Proxymate_<version>_universal.app.tar.gz` and `.sig`: the updater bundle and its signature.
- `latest.json`: the update feed. The app reads `https://github.com/charlie-niekirk/proxymate/releases/latest/download/latest.json`,
  which GitHub serves from the newest **published, non-prerelease** release. Drafts and prereleases (`v1.2.0-beta.1`)
  never reach existing installs.

## One-time setup: the updater key

The only secret a release needs is the updater's private key. Only tag pushes and manual runs read it; pull-request
runs use a throwaway key.

| Secret | What it is |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | The updater private key (contents of the key file) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The key's password. Only if it has one; the current key has none |

The public key is committed in `tauri.conf.json` (`plugins.updater.pubkey`), and every installed copy trusts only that
key. The private key was generated with `npx tauri signer generate --ci -w ~/.tauri/proxymate-updater.key` (no password).
```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/proxymate-updater.key
```
Keep a backup of `~/.tauri/proxymate-updater.key` somewhere safe, such as a password manager. **If it is lost, no
installed copy can be updated again**: users would have to reinstall from a DMG.

A tag build fails if this secret is missing.

## Cutting a release

1. Bump the version in **all** of these, in one PR:
   - `apps/desktop/src-tauri/tauri.conf.json` `version`. This is the app's version and what the updater compares.
   - `Cargo.toml` `[workspace.package] version`.
   - `apps/desktop/ui/package.json` `version`.

   Use plain semver. A prerelease looks like `0.2.0-beta.1` and gets a prerelease GitHub Release.
2. Merge it, then tag `main`:
   ```bash
   git switch main && git pull
   git tag v0.2.0 && git push origin v0.2.0
   ```
   The workflow fails early if the tag doesn't match `tauri.conf.json`.
3. Wait for **Release** in Actions (about 15–20 min). It creates the draft `v0.2.0` with the DMG, the updater bundle and
   signature, `latest.json`, and install notes. Re-running it replaces the assets (not the notes).
4. Check the draft: download the DMG, drag the app to Applications, and follow the install notes as a user would.
   The app should open after **Open Anyway** and show "Listening on 127.0.0.1:9090". Optionally:
   ```bash
   lipo -archs /Applications/Proxymate.app/Contents/MacOS/proxymate     # x86_64 arm64
   codesign -dv /Applications/Proxymate.app 2>&1 | grep flags           # adhoc,runtime
   ```
5. Write the release notes (keep the install section) and **Publish**. Running copies find the update at their next
   launch, or straight away with **Proxymate › Check for Updates…**.

The notes inside `latest.json` only link to the GitHub release, so you can edit the published notes freely.

**Dry run.** Use Actions › Release › **Run workflow** (on `main` or a branch). It does the full build with the updater
key and attaches the files to the run, without creating a release. Pull requests that touch the packaging run the same
build with a throwaway updater key.

## What ad-hoc signing means for users

- **First launch:** macOS refuses to open the app ("Apple could not verify…"). The user clicks **Open Anyway** in System
  Settings › Privacy & Security (on macOS 13–14, Control-click › Open also works), or runs
  `xattr -dr com.apple.quarantine /Applications/Proxymate.app`. This is once per install from a DMG.
- **Updates** are verified with the updater key, not Apple's, and the updater's own downloads aren't quarantined, so an
  update doesn't bring the Gatekeeper prompt back.
- An ad-hoc signature is tied to the exact build, not to a developer. macOS may ask again, after an update, for
  permissions it remembers per app (for example Local Network access).
- Not yet checked: updating a copy in `/Applications` that went through Open Anyway. If macOS says Proxymate was
  prevented from modifying apps, allow it in System Settings › Privacy & Security › App Management.
- Apple silicon needs every binary to be signed; ad-hoc counts, so the universal app runs natively on both
  architectures.

## Later: Developer ID and notarisation

This needs a paid Apple Developer Program membership. Nothing in the build has to change: when these secrets exist, the
workflow signs with the Developer ID, Tauri notarises and staples the app and signs the DMG, and `macos.sh` notarises and
staples the DMG and checks both with `spctl` and `stapler validate`. The drafts then no longer get the install notes.

| Secret | What it is | How to get it |
|---|---|---|
| `APPLE_CERTIFICATE` | Developer ID Application certificate and private key, as base64 `.p12` | Below |
| `APPLE_CERTIFICATE_PASSWORD` | The password you gave the `.p12` export | |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID1234)` | `security find-identity -v -p codesigning` |
| `APPLE_API_KEY` | App Store Connect API **Key ID** (e.g. `ABC123DEFG`) | Below |
| `APPLE_API_ISSUER` | App Store Connect API **Issuer ID** (a UUID) | Below |
| `APPLE_API_PRIVATE_KEY` | The contents of `AuthKey_<KeyID>.p8` | Below. The workflow writes it to a file and passes `APPLE_API_KEY_PATH` |

**Developer ID Application certificate.** Only the Account Holder can create one.
1. In Xcode › Settings › Accounts › Manage Certificates, click **+** › **Developer ID Application**. You can also create it at
   developer.apple.com › Certificates.
2. In Keychain Access › login › My Certificates, right-click `Developer ID Application: …` (it must show the private key
   under it), choose **Export…**, save as `.p12` and set a password.
3. Add the secrets, then delete the export:
   ```bash
   base64 -i DeveloperID.p12 | gh secret set APPLE_CERTIFICATE
   gh secret set APPLE_CERTIFICATE_PASSWORD          # prompts
   gh secret set APPLE_SIGNING_IDENTITY              # paste the identity from `security find-identity -v -p codesigning`
   rm DeveloperID.p12
   ```

**App Store Connect API key (notarytool).**
1. Go to App Store Connect › Users and Access › Integrations › App Store Connect API › Team Keys and click **+**. Name it
   `proxymate-notary` with the **Developer** role.
2. Download `AuthKey_<KeyID>.p8`. Apple lets you download it only once. Note the Key ID, and the Issuer ID shown above
   the table.
3. Add the secrets:
   ```bash
   gh secret set APPLE_API_PRIVATE_KEY < AuthKey_ABC123DEFG.p8
   gh secret set APPLE_API_KEY --body ABC123DEFG
   gh secret set APPLE_API_ISSUER                    # prompts; paste the Issuer ID
   ```

**Checking a notarised release:**
```bash
spctl -a -vv -t open --context context:primary-signature Proxymate_0.2.0_universal.dmg
xcrun stapler validate Proxymate_0.2.0_universal.dmg
spctl -a -vv /Applications/Proxymate.app     # source=Notarized Developer ID
```

**Upgrade path:** add the secrets and cut the next tag. Existing ad-hoc installs update to the notarised build through the
updater; the bundle id stays `dev.proxymate.desktop`, so data and settings carry over.

**Rotating these:**
- **Developer ID certificate.** It expires after 5 years; replace it any time with the steps above, then update
  `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` and (if the name changed) `APPLE_SIGNING_IDENTITY`. Users are
  unaffected because the Team ID is the same. If it's compromised, revoke it at developer.apple.com and contact Apple:
  revocation can also block already-shipped builds, so ship a release signed with the new certificate first.
- **App Store Connect API key.** Revoke it in App Store Connect, create a new one, and update `APPLE_API_KEY`,
  `APPLE_API_ISSUER` and `APPLE_API_PRIVATE_KEY`. It is used only to submit to notarisation.

## Rotating the updater key

If it is compromised, or as routine:
1. Generate a new pair: `npx tauri signer generate -w ~/.tauri/proxymate-updater-2.key`.
2. Put the **new public key** in `tauri.conf.json`, but cut that release with the **old private key** still in
   `TAURI_SIGNING_PRIVATE_KEY`. Installed copies can only verify the old key, and the copy they install then trusts the
   new one.
3. After that release is published, set `TAURI_SIGNING_PRIVATE_KEY` to the new key.

If the old key is gone or must not be used again, users have to install the new version from the DMG by hand.

## How the app is signed

- **Hardened runtime, no App Sandbox.** The system proxy (`networksetup`), CA trust in the login keychain
  (`SecTrustSettings`), client-app detection (`libproc` on other processes), the Simulator and Android tools (spawning
  `xcrun`/`adb`) and the sysproxy guardian all need an unsandboxed process.
- **Entitlements: none.** None of the hardened-runtime exceptions is needed:
  - No JIT or unsigned executable memory: WebKit's JIT runs in its own Apple-signed WebContent process.
  - No library-validation exception: everything is statically linked, apart from system frameworks.
  - No dyld variables, and no `get-task-allow` (notarisation rejects it).
  - No Apple Events to other apps.
  - Network, files and the keychain need no entitlement outside the sandbox.

  The updater's admin-rights fallback (used when `/Applications` isn't writable) runs `do shell script` in-process, which
  isn't an Apple Event to another app either. Add an entitlement only with its reason here and in the plist's comment.
- **Executables.** The bundle has one Mach-O, `Contents/MacOS/proxymate`. It is also what runs as
  `proxymate --sysproxy-guardian <pid>` and `proxymate mcp-stdio`, so those are covered by the same signature and the
  hardened runtime. There are no sidecars. `macos.sh` fails if any Mach-O in the bundle is unsigned or lacks the runtime
  flag. If a sidecar is ever added (`bundle.externalBin`), Tauri signs it too, and the check covers it.
- **Minimum macOS 13.3.** The UI's CSS (Tailwind v4: `color-mix()`, `@property`, cascade layers) needs WebKit 16.4,
  which ships with 13.3.

## The updater

- `tauri-plugin-updater` reads the `latest.json` above and picks `darwin-aarch64` or `darwin-x86_64` (both point at the
  universal bundle). It checks the minisign signature against the committed public key before replacing the app, then
  restarts through the normal quit path, which restores the system proxy and removes live files.
- **Launch check:** 5 s after launch, unless Settings › Updates turns it off (`updates` row in `app.sqlite`). You only
  hear about it when an update is available. Debug builds skip it.
- **Proxymate › Check for Updates…** and Settings › Updates › Check Now report "up to date" and errors too.
- Update requests bypass the system proxy, which may be Proxymate itself.

**Testing the updater end to end, locally.** `PROXYMATE_UPDATE_TEST=1` only works in debug builds. It checks at launch
and installs without asking.
```bash
npx tauri signer generate --ci -w /tmp/test.key
# A 0.0.1 debug app that trusts the test key and reads a local feed
npm run tauri -- build --debug --bundles app --config '{"version":"0.0.1","plugins":{"updater":{"pubkey":"'"$(cat /tmp/test.key.pub)"'","endpoints":["http://127.0.0.1:8765/latest.json"],"dangerousInsecureTransportProtocol":true}}}'
# …then build a 0.0.2 updater bundle signed with the test key (createUpdaterArtifacts), write a latest.json pointing at
# it, serve both on :8765, and launch the 0.0.1 app with PROXYMATE_UPDATE_TEST=1. It replaces itself and restarts as 0.0.2.
```

## Local release build

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/release/macos.sh          # ad-hoc unless you export the variables above; output in target/release/macos
```
`npm run tauri build -- --target universal-apple-darwin` does the same build without the checks and staging.
