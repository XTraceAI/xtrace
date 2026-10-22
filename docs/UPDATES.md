# Desktop updates

For installation, see [setup](SETUP.md). The [latest release page](https://github.com/XTraceAI/xtrace-desktop/releases/latest)
identifies the current update release and its download assets. Newer testing
candidates appear on [all releases](https://github.com/XTraceAI/xtrace-desktop/releases)
and may not be offered by the latest-release link or automatic updates. If the automatic update
fails, quit XTrace and replace it with the latest DMG copy in Applications. If
the copy in Applications is still named **XTrace Desktop**, the new **XTrace** copy
does not replace it: quit XTrace Desktop, move it to the Trash, then drag in the new
copy, so only one copy stays (see [setup](SETUP.md#download-and-install)).
A source version change alone does not publish a release or change the update feed.

The macOS release app checks at startup and every six hours. A newer update
downloads in the background with a ten-minute timeout; checks time out after
30 seconds. Tauri verifies its signature and signed release version before
the app offers **Restart to update** beside the version in the sidebar footer.
Update progress, status and **Retry update** also appear there; idle or disabled
updates show no control. The restart button installs the downloaded
update, then requests the normal Exit/restart path. Installation errors keep
the downloaded update for retry. Restart errors retry restart without installing
again. Navigation and React remounts do not duplicate checks or lose a ready update.

Updates replace the application bundle, not the application-data directory,
SQLite store, or original session files. The existing Exit handler closes
workers and the database after successful installation. No pre-install shutdown
or data migration is added here. Tauri handles bundle replacement and may ask
for administrator permission. A real signed macOS update and restart still need
release validation; compile and unit checks do not establish that behavior.

Releases through 0.1.7 installed as `XTrace Desktop.app`. An update writes the
new version into that same folder, so the name stays. The app does not rename
itself when it restarts to update, because the restart reopens it from that
folder. It renames the folder to `XTrace.app` the next time it quits normally.
It does not rename when the app ID is not `ai.xtrace.app` (test copies), or when
anything is already at `XTrace.app`. If it cannot rename (for example an account
that cannot change Applications), nothing changes and it keeps working under the
old name.

## Release configuration

The base configuration has no endpoint or trusted public key, so ordinary builds
leave updates disabled. Nonfixture native main windows with updates disabled offer
an information-only **Updates** control that opens public releases after a click,
including packaged debug and optimized builds. When native updates are enabled,
the sidebar uses the public updater instead. If the native mode cannot be read,
neither update control is exposed. The release overlay includes the real public
signing key and GitHub Releases endpoint. Debug builds, native fixture builds,
browser previews, and unsupported platforms never register/use the updater.
The main window can check, download and install separately; the tray cannot,
and the combined download-and-install permission is not granted.

Use the release configuration to build an updater release:

```sh
export TAURI_SIGNING_PRIVATE_KEY="/absolute/path/to/private/xtrace-updater.key"
# For an unprotected key:
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
# For a password-protected key, securely set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
# to its password in the local shell instead.
pnpm build:updater apps/desktop/src-tauri/tauri.updater.release.json
```

Configure the matching private-key file path only in the local release shell.
The release overlay requires `createUpdaterArtifacts: true` and signed-version
verification. The public key is copied as already encoded file content, not a
file path or a second base64 encoding. The example overlay remains an invalid
template with an empty public key and endpoints.
An updater-enabled build fails if configuration or `TAURI_SIGNING_PRIVATE_KEY`
is missing. No signing keys are created by the app or these instructions.

The repository `XTraceAI/xtrace-desktop` is now public, so its GitHub Releases
feed needs no authentication. The configured endpoint is
`https://github.com/XTraceAI/xtrace-desktop/releases/latest/download/latest.json`.
The release page and its assets are the source of truth for publication status.
`latest.json` must not offer an update before its real archive and
signature assets are available. Never embed credentials in the app or URL.

Keep the private signing key and password in private release storage, outside
the checkout and keep a secure backup. Key generation is complete; use the
existing matching key rather than generating a replacement. Supply the key path
and password to the build process as shown above without logging private key
contents or passwords. For manual signing, use the path flag shown below.
The public key is safe to include in the shipped overlay.
Keep the same key for future releases. Every release must increase its SemVer.

## Packaging and publishing (not automated here)

The `.github/workflows/release-native.yml` validates an exact candidate. On
default-branch dispatches, a separate protected job can sign and notarize a
drag-to-Applications disk image. Reviewed tag dispatches skip that packaging
job. The workflow does not publish updates. Release publication and updater
acceptance remain separate manual steps. Do not point a feed at unsigned test
bundles.

Use the pinned Tauri CLI **2.12.0** and updater **2.12.0**. Core Tauri stays at
2.11.5. The CLI records the app version in the signature; `requireSignedVersion`
must remain true. Signatures from the older CLI lacking that version are rejected.
The build accepts a file path in `TAURI_SIGNING_PRIVATE_KEY`, but CLI 2.12.0's
manual signer expects literal key contents in that variable. For manual signing,
remove that variable from the command's environment and pass its local file path
explicitly:

```sh
RELEASE_VERSION="$(node -p 'JSON.parse(require("fs").readFileSync("apps/desktop/src-tauri/tauri.conf.json", "utf8")).version')"
env -u TAURI_SIGNING_PRIVATE_KEY pnpm tauri signer sign --private-key-path "$TAURI_SIGNING_PRIVATE_KEY" --app-version "$RELEASE_VERSION" "/absolute/path/to/XTrace.app.tar.gz"
```

Run this from the exact release source checkout and replace the archive path
with the final archive to sign. Confirm its built app version equals `RELEASE_VERSION`. The shell expands the key path before `env` removes the
variable for the signer. Keep `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the local
environment: empty for an unprotected key, or securely supplied for a protected
key.

The release process must fail before packaging when updater signing configuration
is absent. Apple signing/notarization credentials are additionally required.
Before publishing, perform this sequence:

1. Build the release with the real overlay and `createUpdaterArtifacts: true`,
   Apple code signing, and notarization enabled. Check the bundler's notarization
   and stapling result for the `.app`. Never publish an archive made before stapling.
2. Validate the final `.app` with `codesign --verify --deep --strict`,
   `xcrun stapler validate`, and `spctl --assess --type execute`.
3. If the app was notarized/stapled after automatic updater packaging, discard
   the earlier updater archive/signature and recreate the `.app.tar.gz` from the
   final app (one top-level `.app` directory), then sign that final archive with
   CLI 2.12.0 and `--app-version`. Never modify the app/archive after updater signing.
4. Publish immutable HTTPS archive and signature assets. Publish the feed only
   after those assets are available. Put the **contents** of the `.sig` in the
   manifest, not its path. A static manifest uses `version` plus
   `platforms.darwin-aarch64`/`platforms.darwin-x86_64`, each with `url` and
   `signature`. A server returns 204 for no update, or `version`, `url`, and
   `signature` for an update. Publish only architectures actually built/tested.
5. On disposable signed app copies with synthetic data, verify valid update,
   bad signature, mismatched signed version, timeout, interrupted download,
   install permission failure, explicit restart, and retained local data.

See the [official updater guide](https://v2.tauri.app/plugin/updater/) and the
[pinned updater source](https://github.com/tauri-apps/plugins-workspace/tree/updater-v2.12.0/plugins/updater).
The release overlay configures the public endpoint and trusted public key.
Release packaging and publication remain separate manual steps.

## Beta testing limits

Some nested Guardian review conversations may appear under the root conversation rather than their immediate parent. This beta does not change those existing links.

A final signed launch from `/Applications` on a Mac that has never run XTrace,
macOS 14 runtime behavior, and a complete signed update/restart need evidence for
each release. They remain unverified unless that release's notes record the exact
checks and results. Signed packaging, native fixture/source checks and updater
download checks do not establish those results.

## Prepared 0.1.8 acceptance

The hosted harness targets the 0.1.8 candidate from source
`75d49d54f6717d32d72eeb7a933eebb13f5be148`, installed as `XTrace.app`, and uses
the published 0.1.7 (`XTrace Desktop.app`, archive
`XTrace-Desktop-0.1.7-macos-arm64.app.tar.gz`, 11,064,049 bytes, 79 entries) as
the update starting point. The update check covers the rename described above:
after Restart to update the folder keeps its earlier name; after a normal quit
only `XTrace.app` remains, with identical signed contents; reopening it opens the
Dashboard without Welcome.

### Values to fill before any hosted run

The signed build values are not committed yet. They are ten `null` values in the
`release` object of `scripts/ci/hosted-release.mjs`, between `BEGIN SIGNED BUILD
VALUES` and `END SIGNED BUILD VALUES`. Until all ten are filled, every hosted
runner (first launch, telemetry, updater failures and update) refuses before it
downloads, installs or launches anything. Fill them from the final signed,
notarized and stapled files, the same files that are published:

| Value                                 | Computed from                                                               |
| ------------------------------------- | --------------------------------------------------------------------------- |
| `bytes`, `sha256`                     | `XTrace-0.1.8-macos-arm64.dmg`: `stat -f %z` and `shasum -a 256`            |
| `inventoryEntries`, `inventorySha256` | `XTrace.app` inside that DMG, mounted read-only                             |
| `archiveBytes`, `archiveSha256`       | `XTrace-0.1.8-macos-arm64.app.tar.gz`: `stat -f %z` and `shasum -a 256`     |
| `signatureBytes`, `signatureSha256`   | `XTrace-0.1.8-macos-arm64.app.tar.gz.sig`: `stat -f %z` and `shasum -a 256` |
| `feedBytes`, `feedSha256`             | `latest.json`: `stat -f %z` and `shasum -a 256`                             |

Compute the inventory with the harness's own function, from the repository root:

```sh
node --input-type=module -e 'import { appInventory } from "./scripts/ci/hosted-release.mjs"; console.log(JSON.stringify(await appInventory(process.argv[1])));' "/path/to/mounted/XTrace.app"
```

It must give the same result for the app that `/usr/bin/tar -xzf` extracts from
the updater archive (for 0.1.7 both gave 79 entries and the same SHA-256), and the
archive must hold exactly `inventoryEntries` members under one `XTrace.app` folder.
Also replace `scripts/ci/fixtures/hosted-update-latest.json` (now a placeholder)
with the exact `latest.json` bytes; the tests then require it to match
`feedBytes` and `feedSha256`, name version 0.1.8, and point only at the public
v0.1.8 archive. `fixture.lockSha256` in `scripts/ci/hosted-updater-failures.mjs`
is this checkout's `Cargo.lock` (the 0.1.8 product lock); change it only if the
harness branch changes `Cargo.lock`.

### Limits

All hosted runners intentionally use synthetic data with `XTRACE_TELEMETRY=0`,
except the separate telemetry mode. This suppresses the default telemetry notice
as well as telemetry sending. Their reports and success messages identify that
scope.

At preparation, 0.1.8 is unpublished and none of its hosted checks have run.
They require the existing reviewed workflow guards and exact public assets; the
update additionally requires separate authorization to promote the pinned
production feed. Not covered by the update check: a Mac that already has
`XTrace.app` next to `XTrace Desktop.app` (both should stay unchanged), an account
that cannot change `/Applications`, quitting from the menu-bar item, the Dock or
Login Item entries after the rename, browser-download quarantine, and real-history
migration.
