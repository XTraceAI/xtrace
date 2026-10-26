# Continuous integration

Routine CI runs on Ubuntu 24.04 for PRs, merge groups and pushes to `main`.
Native validation runs locally before merging and on a hosted macOS 14 runner
only when explicitly preparing a downloadable release. Public disclosure review
remains manual. These workflows do not publish releases or change repository rules.

| Job         | Required hosted coverage                                                                 |
| ----------- | ---------------------------------------------------------------------------------------- |
| `policy`    | Reviewed source-commit DCO and substantive contribution descriptions.                    |
| `check`     | Types, lint, formatting and unit tests, run once.                                        |
| `ui`        | The full main WebKit browser suite, split across four machines.                          |
| `ui-extras` | WebKit gallery smoke and production-exclusion checks, run once.                          |
| `security`  | CI/supply-chain/publication regressions and source-history/diff scanning.                |
| `selection` | Selects browser jobs conservatively from verified PR event data and Git changes.         |
| `ci-ok`     | Requires selection and all applicable jobs; permits only explicitly unselected UI skips. |

`ci-ok` certifies these hosted checks only. It does not certify native tests,
dependency notices, local evidence or disclosure approval. Maintainers review
those results separately before merging. Browser tests install their Linux system
dependencies even with cached binaries and do not certify native WKWebView behavior.

The `check` job runs `pnpm check` once. The `ui` job runs four WebKit shards,
with individual tests eligible for separate shards through Playwright's
`fullyParallel` setting. A failed shard does not cancel the others; `ci-ok`
requires the combined `ui` result to succeed. A separate `ui-extras` job runs
gallery smoke and production checks once and is independently required by
`ci-ok`. Chromium remains available locally but is omitted from routine CI,
because XTrace ships only for macOS.

Each check or browser job has a 20-minute ceiling including setup. Individual
test limits, retries and worker settings are unchanged. CI prints browser
failures as they finish; the HTML reporter remains enabled. Successful hosted
runs of all four shards and `ui-extras` are required before claiming the full
browser gate passed.

On ordinary PR opens, reopens, source updates and ready-for-review events,
`selection` can skip both `ui` and `ui-extras` when every changed path is
allowlisted documentation or PR preparation tooling. The allowlist covers Markdown
under `docs/` and `change-notes/`, named root contribution/documentation files,
the PR template, the PR preparation skill, and direct `scripts/pr/*.mjs` files.
The root `package.json` is allowed only when its parsed contents are identical
apart from `scripts.test:ci`. Dependencies, other scripts, build files, UI files,
non-Markdown documentation inputs and every unknown path keep browser tests.
Renames check both old and new paths.

The selector verifies the event's base and source commits and the checked-out
HEAD, accepting only that source commit or its GitHub merge with the stated
base. Missing or malformed events, unavailable Git revisions, mismatched commits
and uncertain diffs keep the full browser suite. Main pushes, merge groups and
target-branch edits always keep it, including edits calling the reusable CI.
The selector installs no dependencies and receives no API credentials.

`policy`, `check` and `security` still run for every supported full CI event.
`ci-ok` always runs and requires a successful selector with an exact `true` or
`false` output. Only `ui` and `ui-extras` may be skipped after `false`; failures,
cancellations, missing jobs and other skips still fail. A skipped browser job
does not establish browser coverage for that commit.

The full 96-story gallery sweep runs locally before merging gallery, shared UI,
theme, font or brand-asset changes. Record its source SHA and results in the PR;
hosted success does not certify the full gallery. See [gallery checks](GALLERY.md#add-or-update-a-story).

`PR metadata` handles title/body edits with the same reviewed policy validator.
A target-branch edit calls the full Linux CI against the new base. Separate
concurrency groups prevent a text edit from cancelling base-change validation.
Review `base-ci / ci-ok` when present instead of relying on an older source result.

## Native validation before merging

Quit any running XTrace Desktop instance before the launch checks; the smoke
test does not terminate an existing app, and single-instance behavior prevents a
second copy from opening. Use a clean macOS checkout containing the exact combined source and current base.
After fetching the current target branch, record its full commit SHA and integrate
it into the candidate. Run:

```sh
AGENT_PLUGINS_SOURCE=bundle pnpm check:native --base FULL_REVIEWED_BASE_SHA --fixture-smoke
```

This self-contained development check verifies the committed producer bundle
and selects a synthetic launch with `--fixture-smoke`. It still
compiles the normal debug app first, then builds a separate debug configuration
with the `fixtures` feature and launches only that fixture build with fixed F1.
It uses a compiled test app ID, disposable data and native home, disables
automatic PR checks, and keeps build/copy paths beneath `.noindex`. The copy is
unregistered after the exact child exits. Evidence records `smokeMode:
"fixture-f1"`; it verifies a synthetic native window, not production runtime,
release runtime, tray interaction or macOS 14 compatibility on a newer Mac.
`--fixture-smoke` and `--release` are rejected together before any build.

The released validator also compiles distinct test IDs for normal debug and
release builds and launches verified disposable copies; the launcher and native
inspector require that explicit ID instead of the older implicit production ID.
All smoke modes keep their build and app copies under `.noindex`, unregister
only the owned app after confirmed child exit, and retain the copy if exit
cannot be confirmed.
These normal runtime paths do not enable fixtures. Their synthetic
native-index home does not isolate every provider reader: Claude usage startup
also resolves the real app home/configuration, and passive usage can invoke
Codex. Their production-runtime isolation remains unverified; resolve that
isolation before running those modes on a development Mac. Fixture smoke does
not establish safety or acceptance for those launch paths.

The command requires a clean checkout and verifies the base is an ancestor of the
tested head. It installs frozen dependencies; runs workspace Rust fmt, strict
all-target/all-feature Clippy and tests; verifies installed DTO/plugin hooks
against a temporary copy of that base; generates and scans the SBOM; checks
notices; and builds and launches the debug app. It stops on the first failure.
The temporary hook baseline is removed on success and failure. Installed hooks
cannot disappear compared with the base; absent hooks claim no coverage.

Record the returned source SHA, base SHA and actual macOS version in the PR,
with test counts and relevant behavior checks. Local validation on a newer OS
is not macOS 14 compatibility evidence. Source changes or a dirty checkout during
the run invalidate the result. Before merging, verify the target branch still
matches the recorded base and repeat affected validation when either input advances.
Routine PRs require this local evidence; it is a maintainer review requirement,
not an automatically enforced status check. No self-hosted runner is installed.

## Native validation for a downloadable release

Only an explicit `workflow_dispatch` starts `Release native validation`.
Select the default branch for the workflow and supply the full SHA of a candidate
already merged into that branch. That path still requires the candidate to be
in default-branch history. Alternatively, manually select an explicitly reviewed
`refs/tags/vSemVer` tag (for example, `refs/tags/v0.1.1`) and supply its full
candidate commit SHA. The workflow source commit (`github.workflow_sha`), GitHub
source commit (`github.sha`), resolved tag commit and checked-out HEAD must all
equal that candidate. Non-default branches, other ref types, malformed versions
and mismatched identities fail. A tagged beta can be prepared and published from
that exact reviewed source without first merging its app source into `main`;
the workflow must already exist on the default branch to enable manual dispatch.
The workflow checks out the exact candidate without persistent credentials and
verifies actual macOS 14 arm64 on both paths.

The workflow resolves the candidate's first parent as its reviewed baseline and
runs `pnpm check:native --base REVIEWED_BASE_SHA --release`. Comparing against that
distinct predecessor detects installed hook declarations removed by the candidate;
using the candidate itself as its baseline would lose that protection. Earlier
merged changes still require their own recorded local validation before merging.
Release mode additionally builds release code with a unique test app identifier
and launches a separately signed disposable copy with its own data directory.
It does not launch the final signed public bundle.
The validation job uploads the validated, secret-scanned SBOM and has no signing
values.

Before validating, the job runs `pnpm check:plugin-pin`. It finds the highest
`memhub--vX.Y.Z` release tag in the pinned `agent-plugins` repository and
compares each reader path listed in `.plugin-pin` with that release. It prints
either that the bundled readers are up to date with that release or a warning
naming the pinned version, the latest version and the reader paths that
changed. If it cannot look up the release, it also warns; it never reports up
to date without checking. It only warns: the release continues, and only a
malformed pin fails it. To act on a warning, refresh the bundled copy with
`scripts/vendor-readers.sh` and update `.plugin-pin` (see the pinned plugin
section below). Locally, `pnpm check:plugin-pin --checkout PATH` reads the
tags of an existing `agent-plugins` checkout instead of the network. A
candidate older than this check has no such script; the step then warns that
its bundled readers were not checked and the release continues.

On default-branch dispatches, only after validation passes does the `package`
job build the signed disk image. Reviewed tag dispatches skip this job. It runs
in the `macos-signing` GitHub environment. It compiles the release app without
any Apple values, then one step runs `node scripts/ci/release-dmg.mjs`, which:

1. runs `pnpm tauri bundle --bundles app,dmg` so Tauri signs the app with the
   hardened runtime and no extra entitlements, notarizes it and staples the ticket;
2. checks the app's Developer ID team, timestamp, hardened runtime, stapled ticket
   and Gatekeeper result, because Tauri only warns when notarization settings are
   incomplete;
3. mounts the disk image and checks the window layout with
   `scripts/ci/dmg-layout.mjs`: the app and the Applications shortcut must sit
   where `bundle.macOS.dmg` in `tauri.conf.json` places them, and the background
   picture must exist. Finder sometimes saves the window before it moves the icons,
   so a wrong layout is rebuilt up to three times before the job fails;
4. notarizes and staples the disk image, then requires
   `spctl -a -t open --context context:primary-signature`, `xcrun stapler validate`
   and `hdiutil verify` to pass and checks the mounted app again;
5. writes `artifacts/release/XTrace-VERSION-macos-arm64.dmg` and its
   SHA-256 file, which the workflow uploads as the `dmg-CANDIDATE_SHA` artifact.

The workflow never creates or edits a GitHub release; attaching the disk image to
a release is the release owner's separate step. To run the layout check on any
local disk image, use `node scripts/ci/dmg-layout.mjs PATH_TO.dmg`. Local installs
and `pnpm check:native` still build with `--bundles app` only.

The `macos-signing` environment must require a maintainer's approval and holds
these secrets:

| Secret                       | Value                                                                        |
| ---------------------------- | ---------------------------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | Base64 of the exported Developer ID Application `.p12`, with its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting that `.p12`                                   |
| `APPLE_API_KEY`              | App Store Connect API key ID (10 characters)                                 |
| `APPLE_API_ISSUER`           | Issuer ID for that key                                                       |
| `APPLE_API_PRIVATE_KEY`      | Full contents of the key's `AuthKey_KEYID.p8` file                           |

The signing identity is fixed in the workflow. The script writes the `.p8`
contents to a private temporary file and deletes it when packaging ends.

Before publishing each downloadable version, require successful native release
validation for its exact source plus the separate acceptance of the final signed
and notarized downloadable bytes. Signing, notarization, universal builds,
Gatekeeper behavior and update delivery are not certified by this unsigned app
launch. Tag validation is source validation only, not disclosure approval or a
waiver of macOS 14 QA, final-byte acceptance or publication checks. Changes to
source or packaging invalidate the relevant release evidence. First read the
plugin-reader warning, if any, from the `pnpm check:plugin-pin` step.

Final signed launch from `/Applications` on a Mac that has never run the app
requires separate acceptance for each release. Its release notes must identify
the checks actually completed; a source version bump does not establish that check.

`macos-14` is scheduled for retirement on November 2, 2026. Before retirement,
replace it with a runner that preserves actual macOS 14 floor testing. Building
with a deployment target of 14 on a newer OS does not test that floor.
See the [runner image](https://github.com/actions/runner-images/blob/main/images/macos/macos-14-arm64-Readme.md).

## Pinned plugin conformance

`.plugin-pin` names the one reviewed capture-producer revision Desktop is
validated against: the public `agent-plugins` repository, a full commit SHA,
the plugin root, the plugin's release version and the Git object IDs of the
reader sources the Desktop index will consume (`readers/`, `readers_cli.py`,
`cursor_flush.py`, and the whole `scripts` tree they import from, which the app
bundles as `vendor/agent-plugins` and verifies by the same object identities
without Git). `node scripts/ci/plugin-pin.mjs commit` prints a field;
`scripts/ci/plugin-pin.test.mjs` rejects malformed pins in hosted CI.

`scripts/ci/plugin-conformance.sh` owns the producer source, the environment
and unskipped execution. `AGENT_PLUGINS_SOURCE` defaults to `checkout`.
In checkout mode, without `AGENT_PLUGINS_DIR` it fetches exactly the
pinned commit from the public repository, without credentials, into the ignored
`artifacts/private/plugin-conformance/` directory; a supplied checkout is
accepted only at that commit. Either way the checkout must be clean and every
listed reader source must be the pinned object, so an edited file at the right
commit fails before any test runs. Checkout mode requires the pinned revision
to remain available upstream or an exact clean checkout supplied through
`AGENT_PLUGINS_DIR`. If the upstream no longer serves that revision, fetching
cannot complete this mode; a bundle-mode pass is not upstream verification.

Explicit `AGENT_PLUGINS_SOURCE=bundle` uses `vendor/agent-plugins` by default;
`AGENT_PLUGINS_DIR` can name a bundle's `plugins/memhub` directory instead.
Before any producer import or launch, a test-only Rust example calls the app's
existing `verify_bundle` against `.plugin-pin`: all four listed objects,
including the complete scripts tree with executable modes, must match. Extra
or missing files, altered bytes or modes, symlinks and bytecode caches fail.
The Python harnesses call that same verifier on their source and copied tree.
The gate runs six mutation regressions against all three Python harnesses.
It sets `PYTHONDONTWRITEBYTECODE=1` so imports cannot add caches to verified trees.
The explicit native release workflow selects this mode and uses the producer
bytes already in the candidate repository, with no external producer fetch.

Both modes require Python 3.10+ (`PYTHON` selects
the interpreter), clears `PYTHONOPTIMIZE`, `PYTHONPATH`, `PYTHONHOME` and
`PYTHONSTARTUP` so the harnesses' assertions and standard library stay intact
(an interpreter running with `-O` is rejected), runs
`cargo test --workspace --all-features --locked -- conformance --nocapture`
with both output streams captured to `artifacts/private/conformance/cargo-test.log`,
then runs `scripts/ci/assert-no-skipped-conformance.sh` on that log. The
validator requires every name in `scripts/ci/conformance-inventory.txt` to have
run and passed, and fails on any `SKIP` line, ignored test, failure, zero count
or missing name. Under a plain `cargo test` without the environment the
conformance tests print `SKIP` and return; that output can never satisfy the
gate. Missing source, missing Python, an unfetchable checkout pin or differing bundle object, a
removed hook or an absent named test are all red before merging.

The inventory currently requires:

| Test                               | Real contract exercised                                                                                                                                                                                                                                                                                                                                                     |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `conformance_flush_turn`           | The pinned Stop hook imports through both routing mechanisms, advancing its real cursor only after committed acknowledgements, including retry after a failed commit and metadata-only storage.                                                                                                                                                                             |
| `conformance_plugin_transport`     | The pinned decoder and token client against the headless binary: initialize, SSE `tools/list`, token mint/list/delete, notification and empty-import rejection.                                                                                                                                                                                                             |
| `conformance_native_reader_stream` | The pinned `readers_cli.py` over the fixture catalog's synthetic native Codex/Cursor files ([F18 and F20](FIXTURES.md)); see [acceptance](acceptance/plugin-conformance.md).                                                                                                                                                                                                |
| `conformance_native_import`        | The complete native import through the pinned readers into a disposable index: identity and counts per session, a repeated run adds nothing, source bytes unchanged; see [native import](acceptance/native-import.md).                                                                                                                                                      |
| `conformance_bundled_readers`      | Both modes verify the pinned bundled scripts and read F18 in place to its expected identities and counts, leaving sources unchanged. Checkout mode additionally compares every scripts file and LICENSE/NOTICE byte with the producer commit and compares the two independent indexes.                                                                                      |
| `conformance_exact_detail`         | The bundle's exact-detail mode reads F18's JSONL sessions and a synthetic paginated Codex group whole, matches ordinary export record for record, refuses the Cursor store beside a committed write-ahead log, and changes no source or temporary snapshot. Checkout mode additionally runs exact detail through the verified checkout and uses its own pagination fixture. |
| `conformance_codex_fork_import`    | A Codex conversation forked in Codex Desktop imports as its own session: its own ID and start, only its own records, and the original's counters at the cutoff as its usage baseline. Bundle mode supplies equivalent synthetic history data; checkout mode uses the pinned checkout's fixture.                                                                             |
| `conformance_tool_sent_import`     | The pinned bundle over a synthetic Codex and Cursor home: with `--automated-input-evidence` the lines differ from the plain export only by the evidence, and a scan into an index an earlier build filled binds one proof to each subagent notification, interrupted-turn note, app page record, Cursor summary and Cursor import banner, changing no record.               |

Record in the PR the pin commit printed by the hook and the `executed N of N`
line with its test names and the selected source mode. A bundle-mode pass does
not certify the independent checkout/index comparison, the producer checkout's
pagination fixture, or notices against upstream Git; run checkout mode for those
checks. A bundle execution is never reported as an independent checkout.
Every producer release train that Desktop adopts
updates `.plugin-pin`, the reader source
object IDs, the bundled copy (`sh scripts/vendor-readers.sh`, then the printed
scripts tree ID in the pin) and, when the reader stream changes on purpose, the fixture goldens
(`python3 scripts/conformance/test-reader-stream.py --write-golden`, then review
the diff). A pin whose readers emit a different stream fails the reader test
until the golden and the consumer are updated together.

## Disclosure and manual audits

Routine PRs have no disclosure checkbox or snapshot gate. Before changing the
repository to public, and again before the first downloadable release, run the
local comprehensive review after inspecting source/history, relevant PR and issue
text, comments and attachments. Follow [PUBLICATION.md](PUBLICATION.md) for the
procedure and token isolation. Ordinary PR events, comments, reviews, CI
completion and time passing do not start hosted publication scans. There is no
scheduled sweep or review-event relay. Old advisory results are not current
publication evidence.

`Publication content advisory` remains available for an explicitly requested
repository-wide audit through manual dispatch on the default branch. It reads
reviewed code and never executes a candidate's scanner with metadata credentials.
It has no automatic enforcement role; its GitHub Actions identity is spoofable.

## Reviewed merges

Serialize reviewed squash merges. Verify current head/base identities, hosted CI
provenance, DCO, local native results and unresolved findings together. Green
Linux CI alone does not authorize a merge or direct push.
Failed required local checks prevent merging just as failed hosted checks do.

Merge queues remain disabled pending separately approved activation and live
combined-result acceptance. Linux `merge_group` support and synthetic queue tests
remain, but they do not certify native or disclosure validation of a live queue.

## Contribution requirements

PRs use descriptive names and include substantive `## Change` and
`## Verification` sections. Describe the problem, resulting behavior, test setup,
actions, expected and actual results, and any checks still pending. Complete the
publication/release review in [PUBLICATION.md](PUBLICATION.md) before making the
repository or a downloadable artifact public. Public acceptance contracts
describe observable behavior and belong in [acceptance/](acceptance/).

The `security` job tests candidate code without API credentials. A separate
`policy` job checks out the default branch, or the full commit SHA in repository
variable `CI_POLICY_SHA`, before installing its pinned dependencies. Only that
reviewed source receives the read-only metadata token. The workflow itself
remains subject to manual provenance review.

For initial installation, a maintainer reviews the complete policy entrypoint,
its imports and dependency graph, and sets `CI_POLICY_SHA` to that exact 40-digit
commit SHA. Rerun CI on the same candidate after configuration. Missing policy
fails closed with a bootstrap diagnostic; a green source-only run is not live
policy validation. After the default branch contains the reviewed validator,
remove the bootstrap variable and verify a subsequent run uses the default
branch. Do not advance the variable automatically with PR pushes.

The validator resolves actual source PRs from merge-queue entries and reads each
PR's contributed commit list. Every author must have a matching final
`Signed-off-by` trailer. Bootstrap history and GitHub's synthetic queue commit are
not substitutes for those source commits. Incomplete API pagination, unknown
queue shapes and metadata changes during validation fail. The supported source
PR limit is 250 commits.

DCO and contribution descriptions are checked before merge. Main pushes validate
the resulting source, with policy limited to checking the push repository,
default branch and commit identities. They do not re-certify GitHub's synthesized
squash commit or reread mutable merged PR metadata. The repository must require
reviewed PR merges for this procedure; a green post-merge build does not
authorize a direct push.

## Local checks and integration hooks

```sh
pnpm check
pnpm test:ci
pnpm test:supply-chain
pnpm --dir apps/desktop/ui exec playwright install webkit chromium
pnpm e2e
AGENT_PLUGINS_SOURCE=bundle pnpm check:native --base FULL_REVIEWED_BASE_SHA --fixture-smoke
pnpm publication:test
pnpm secrets:check
```

The native validation command installs the pinned tools and adds their cache
directories to its child-command `PATH`. For separate supply-chain commands, add
those directories to `PATH`, or set `SYFT` and `CARGO_ABOUT` explicitly. Syft 1.51.1 produces the schema-validated CycloneDX 1.5
artifact; cargo-about 0.9.2 and actual npm license texts produce notices. The active
license allowlist includes MPL-2.0, Zlib and Unicode-3.0. See
[dependency obligations](DEPENDENCY_LICENSES.md) for retained notices and
corresponding-source access; unsupported obligations still fail.
SBOM generation succeeds independently of notice approval. CI scans the generated artifact for secrets and uploads only the
validated SBOM, without browser traces or native logs.

The native smoke launches the built debug app, observes that exact process's
finished launch and main window through AppKit/CoreGraphics for two seconds,
then terminates only its own child. It requires an interactive macOS session and
no existing instance of the app. It does not inspect WKWebView content, tray
interaction or production packaging. Those checks belong to later native and
release acceptance.

`node scripts/ci/run-hook.mjs plugin-conformance` invokes the installed
`scripts/ci/plugin-conformance.sh`; `.plugin-pin` and
`assert-no-skipped-conformance.sh` also make the runner mandatory, so none of the
three can disappear compared with the reviewed base. See
[pinned plugin conformance](#pinned-plugin-conformance) for what the hook
requires and records. `node scripts/ci/run-hook.mjs dto` invokes `scripts/ci/check-dto.sh` once
installed; a generated DTO directory makes that check mandatory. The DTO
hook must generate into temporary output and reject missing, extra or stale
committed exports. UI jobs consume those committed exports; the local native result supplies
the separate parity evidence. Any installed hook failure or removal compared with the reviewed base
fails native validation.

Actions use pinned commits. Rust, pnpm, Playwright and pinned supply-chain tools
have caches. Automated dependency-update PRs are deferred until their generated commits and metadata meet
the sign-off, verification and disclosure requirements; no Dependabot schedule is installed by
this change. Dependency updates use the normal reviewed PR process. Cache timing and manual combined-result validation need recorded run evidence.
The release-only workflow uses the bundled producer under `vendor/agent-plugins`
and supplies its plugin root to the mandatory native conformance hook. Before the
first workspace test, native validation builds the bundle verifier and checks the
bundled producer's identity and file modes against `.plugin-pin`. The workflow
tests enforce bundle mode and bootstrap order. This adds no PR job.
The 0.1.4 release completed native validation for its exact source on macOS 14
arm64. Each later release needs a new exact-source result. Live queue evidence
is required before queue activation.

Native Rust tests and Clippy enable all debug features so fixture-mode tests cannot silently disappear behind an optional feature. Production packaging uses its normal feature set and must still exclude debug fixture assets.

## Hosted UI capability probe

`Release native validation` also accepts `delivery_mode=probe`. Its default
`native` mode (including older dispatches that omit the input) keeps the existing
native validation and protected packaging flow. Probe mode runs a separate
`ui-capability-probe` job and cannot enter either of those jobs. Unsupported modes
fail before candidate code runs.

The probe can run from the default branch or a reviewed SemVer tag. For this
mode, `candidate_sha`, the workflow commit, the event source commit and the
checkout must all match. This permits reviewing and testing a tagged harness
before merging it. A probe tag is harness source; it is not a downloadable app
release and does not validate the candidate's native product code.

On a fresh GitHub-hosted macOS 14 arm64 image, the harness records OS/image
identity and the console user, rejects existing XTrace installations, data or
processes, and rejects provider tools in the app's discovery paths. It compiles
a harmless AppKit fixture with a unique `ai.xtrace.app.test.<UUID>` identity and
a nonpersistent WKWebView displaying fixed offline HTML. The page has no external
resources or product code; its ordinary button handler changes the visible text.
The fixture receives only a small environment allowlist with its own data,
native-home, Claude-config and temporary directories. It receives no inherited
account or Actions credentials and does not override `HOME`.

An external Swift Accessibility driver checks the exact process, executable,
bundle path and test ID. It must observe a window and readable web content, time
out on a deliberately missing control while the fixture remains unchanged for
three seconds after the page becomes readable, press the real test button,
and observe the visible label change. A window alone, missing Accessibility,
failed press, process exit or timeout is a failure. The harness never changes
TCC, requests permissions, disables Gatekeeper or injects product commands.
It stops only its own child, confirms exit, unregisters its fixture and removes
the temporary staging directory. Cleanup failure also fails the run.

The `hosted-ui-probe-<run>-<attempt>` artifact retains the summary and bounded AX
snapshots/logs, including failed probes. This proves only the fixture interaction
on the allocated image. It does not prove XTrace's WKWebView, signed first launch,
browser-download quarantine behavior, or update/restart. No released app is
downloaded, installed or launched by probe mode. The probe command still rejects
released-app modes. The separate first-launch job below requires its own explicit
opt-in; the update job described below has the same explicit opt-in.

## Signed 0.1.8 first launch on a hosted Mac

The harness targets the 0.1.8 candidate from source
`75d49d54f6717d32d72eeb7a933eebb13f5be148`, installed as `XTrace.app`. Its signed
build values (DMG, app inventory, updater archive, signature and feed sizes and
SHA-256s) are pinned in `scripts/ci/hosted-release.mjs` from the published
pre-release files; every runner refuses if any is missing (see
[updates](UPDATES.md#prepared-018-acceptance)).
These pins prepare future validation; they do not establish public availability
or runtime success. First launch requires its exact public source tag and DMG.
The update check also requires the exact candidate feed to be promoted through a
separate authorized release action. The harness itself must reach an allowed
reviewed workflow ref.

Both released-app jobs use synthetic data and explicitly set `XTRACE_TELEMETRY=0`.
That override disables telemetry and suppresses the default telemetry notice.
A passing result therefore covers a telemetry-disabled synthetic launch, not the
default telemetry notice or default telemetry behavior. Each report records these
limits in `validationLimits`, and the success output states the same limits.

After reviewing the harness and explicitly authorizing the released app's real
identity on a fresh hosted VM, dispatch `delivery_mode=first-launch` with
`allow_release_identity=true`. The boolean defaults to false, and the job fails
before checkout if it is absent or false. Use the exact reviewed harness SHA as
`candidate_sha` on the default branch or its matching reviewed tag. That is
distinct from the fixed product source `75d49d54f6717d32d72eeb7a933eebb13f5be148`.
The Node runner and both Swift tools also reject unauthorized or non-hosted use.

This job anonymously downloads the pinned public 0.1.8 DMG and checks its size,
SHA-256 and public source tag, resolving its single annotation layer when present.
It verifies strict signatures, the expected
Developer ID team, stapled tickets and Gatekeeper acceptance for the app and DMG.
It checks every app path, byte, size and mode against a pinned canonical
inventory before copying `XTrace.app` to the VM's `/Applications`, after copying
it, and after the test exits. It never rebuilds, re-signs or edits the product.

The download uses curl. Quarantine metadata is recorded on the DMG, mounted app
and installed app; the harness neither adds nor removes it. LaunchServices opens
the installed app under normal system policy. Security prompts are not approved
automatically; a blocking prompt or uncertain launch fails the check. This tests
signed first launch on the allocated hosted image, **not a browser-quarantined
download's first Open confirmation**.

Fresh-host checks reject existing current/old XTrace data, WebKit storage,
preferences, provider configs, provider CLI paths and Claude's account Keychain
item. No credential value is requested or logged. The launch receives owned
`XTRACE_DATA_DIR`, `XTRACE_NATIVE_HOME`, `CLAUDE_CONFIG_DIR`, `GH_CONFIG_DIR` and
temporary paths with a short system PATH. The helper then reads the owned
process's environment without logging values, confirms those paths and rejects
unexpected keys. LaunchServices can add system variables; `HOME`, if supplied
by the system, must remain the actual VM user's home. WebKit and preferences may
write under that fresh VM home; this is not a test of migrating existing data.
The launch configuration explicitly sets `SSH_AUTH_SOCK` to an empty string,
and the process check requires that exact empty value before UI testing.
After checking authorization, the helper removes its four automation markers
from its own environment before calling LaunchServices. The app must not inherit
them. Any unexpected process keys are reported together by name, never value.

The external AX driver requires a real Welcome screen, completed native scan,
verified bundled readers and version 0.1.8, with no native query error. It presses
Open Dashboard and requires the loaded empty-report result, Dashboard heading
and version. After stopping the exact PID/bundle/executable/kernel-start-time receipt,
the runner verifies the owned database's integrity and empty sessions/records,
and checks that neither default current/old app directory received a database.
It unregisters and removes only its owned app and staging files after confirmed
exit. Missing ownership or unconfirmed exit retains the copy and fails.

Stopping first requires strict kernel and app ownership. After a stop request, a
zero-byte process query with ESRCH while kill-zero still succeeds remains pending
within the existing ten-second graceful wait or five-second forced wait. It is
neither life nor exit evidence. Only the existing missing-start plus kill-zero
ESRCH check confirms absence; invalid PID/UID/start fields, a changed birth time
or permission failures still fail. Force termination requires a fresh strict
owner check. Bounded diagnostics identify the stop phase and owned-process facts.

Both signed jobs run the actual Swift stop decisions with controlled clocks and
actions, plus harmless owned command-line children. Those controls verify finite
waits, no signal on unknown ownership and reaping of their children; they do not
establish a signed-app shutdown result or the cause of an earlier hosted failure.

The `hosted-first-launch-<run>-<attempt>` artifact contains the launch receipt,
AX snapshots, signature/assessment logs, quarantine observations and result.
It does not contain the app environment or database. This mode never
publishes/promotes a release or exercises the updater. Actual update/restart
evidence remains separate and requires the production feed to offer the target.

The runner caps work commands against an eight-minute shared deadline, then
reserves two minutes for cleanup. Individual command and UI limits stay strict.
Its workflow step allows eleven minutes; the separately bounded preflight,
checkout, setup, tests, run and upload steps total twenty-one minutes inside the
twenty-two-minute job limit, leaving time to retain failure evidence. The offline
compiler/test step allows three minutes; the app execution and cleanup limits
are unchanged.

Public tag-metadata requests retain the curl version, HTTP status, selected
response headers and a bounded error-body excerpt. Raw response headers stay in
disposable staging; cookies, authorization and redirect headers are not retained.
Non-2xx, incomplete, oversized or invalid JSON responses fail before tag checking
or app download. These requests remain anonymous with no automatic retry or
credential fallback; a 403 alone does not establish its cause.

## Signed 0.1.7 to 0.1.8 update on a hosted Mac

`delivery_mode=update` uses the same fresh GitHub-hosted macOS 14 arm64 guard,
exact harness SHA and `allow_release_identity=true` requirement. Review the
harness and first-launch evidence before dispatch. The fixed production endpoint
must already serve the exact pinned 0.1.8 `latest.json`. Promotion is a separate
release-owner action that makes the update available to production users; this
job never changes the feed, publishes a release or substitutes an endpoint.
A different feed fails the prerequisite before any app launch.

The earlier app and its 0.1.8 successor both keep `XTRACE_TELEMETRY=0` and synthetic
data. Even a successful real update/restart does not validate the default telemetry
notice or default telemetry behavior; those require separate validation.

The job downloads the exact signed public 0.1.7 archive anonymously, checks its
pinned bytes, installs it as `/Applications/XTrace Desktop.app` on the disposable
VM (the folder name 0.1.7 users have) and verifies all 79 entries plus Developer
ID, stapling and Gatekeeper. It reuses the synthetic environment and fresh-host
checks above and writes a marker file into the owned data folder. Through
external Accessibility it completes Welcome, observes a usable native 0.1.7
Dashboard and the target-specific 0.1.8 ready message, records the identity of
the database file 0.1.7 created, then rechecks that the installed app is still
0.1.7. It presses the real Restart to update button once. No injected product
code, manual relaunch or automatic prompt approval can produce a passing result.

A saved transition binds the original process before the action. The helper
requires its exit and one distinct successor at the same exact bundle/executable
and app ID, with a fresh kernel process start time. It saves a separate ownership
receipt and checks the successor's inherited synthetic environment. Then:

- **After the restart:** the native 0.1.8 Dashboard loads and Welcome is never
  shown. `/Applications` holds `XTrace Desktop.app` and no `XTrace.app`; that
  folder now has exactly the pinned 0.1.8 contents and passes Developer ID,
  stapling and Gatekeeper; the marker file and the same database file remain.
- **After a normal quit:** the helper sends one Quit request (the Apple event the
  Dock and Cmd-Q send) and waits up to 20 seconds without forcing. Only
  `XTrace.app` then exists in `/Applications`, with contents identical to the
  updated folder and the pinned 0.1.8 inventory, and it passes `codesign`,
  `stapler validate` and `spctl`. The marker, database file, SQLite integrity and
  empty sessions/records are checked.
- **After reopening:** LaunchServices opens `/Applications/XTrace.app`; the
  native 0.1.8 Dashboard loads without Welcome. After another normal quit the
  same folder, contents, marker and database file remain, and neither default
  app directory has a database.
- **Leftovers:** no other XTrace folder or updater copy exists in `/Applications`,
  and no `tauri_current_app*` or `tauri_updated_app*` folder remains in the app's
  owned temporary folder or the runner's temporary folder.

This tests empty synthetic data, not migration of real histories,
browser-download quarantine prompts, a Mac that already has `XTrace.app`, or an
account that cannot change `/Applications`.

The `hosted-update-<run>-<attempt>` artifact retains receipts, AX observations,
feed and file identities, signature checks, quarantine metadata and cleanup
results. An uncertain restart stops the known original but retains the copy and
fails; a missing/ambiguous successor never authorizes broad process killing or
removal. A quit that does not finish fails; cleanup then uses the ordinary
graceful-then-forced stop. Cleanup removes whichever of the two folders this run
created. Work has a fourteen-minute deadline and a separate two-minute cleanup
reserve inside a seventeen-minute execution step and twenty-eight-minute job.
The offline compiler/test step allows three minutes without extending app runtime.
Adding this harness is not evidence that the hosted update has passed.

Both released-app modes use the existing isolated Homebrew Python SQLite reader
with `mode=ro` for the same integrity and empty-table checks. Main database bytes
must stay unchanged; SQLite may create or update WAL/SHM bookkeeping. Engine
versions and before/after file metadata are retained. No reader fallback or
database repair is performed.

### Default telemetry validation of signed 0.1.8

The explicit `delivery_mode=telemetry` option reuses the signed first-launch job
and its exact source/ref, hosted macOS 14 arm64, fresh-machine, signature,
notarization, app-byte and ownership checks. It requires
`allow_release_identity=true` and the exact reviewed candidate SHA. The ordinary
first-launch and update modes remain telemetry-disabled. Adding this mode does
not itself supply a passing hosted result or authorize a branch-based run.

This mode uses the genuine hosted HOME and a previously absent default app-data
folder, with synthetic native/provider paths. Before each signed launch, exact
system-curl controls and a fixed harmless LaunchServices probe qualify inherited
proxy settings against a separate rejecting loopback child. The listener never
forwards or resolves upstream requests and accepts no TLS or event bodies. Each
phase has a fresh proxy and receipt; the actual signed process environment is
inspected immediately after launch. This is a controlled proxy configuration,
not an OS-wide network sandbox.

The four phases are: the default notice; saved-off restart followed by Settings
on; environment-off precedence while the saved preference remains on; then normal
saved-on restart followed by Settings off. In the notice phase the driver only
finds the Welcome notice in the accessibility tree, which also lists content that
is not on screen. Since 0.1.8 the notice counts as shown only when half of it is
visible above the Welcome page's pinned bar, which it is not in the default
window, so nothing is sent from Welcome. The runner first requires that the
phase's proxy has seen no telemetry attempt yet; the driver then presses Open
Dashboard once, requires the Dashboard's corner card (its "on by default" title,
what is sent and never sent, and one enabled Turn off), and only then waits for
the blocked telemetry attempt; it then presses Turn off in that card. Each step
reports what it established (found in the tree, pressed, card present), not that
anything was seen on screen. Saved-on starts on the Dashboard with the disclosure
already recorded, so its startup attempt needs no click.
Each signed process must stop before read-only database assertions check saved
preferences, disclosure and the absence of sent ledger rows. Blocked connection
counts may vary. An exact updater warning is accepted only inside its Workspace
alert, with this phase's rejected GitHub CONNECT; every unrelated error/dialog
remains fatal. The harness never clicks Retry update.

Reports separate UI, saved-state and blocked-attempt evidence. They do not prove
successful signed HTTP delivery, real PostHog delivery, event payload contents or
before-first-pixel ordering. Source tests cover gates and payload policy. The
signed product and published assets remain unchanged.

Work has one shared 18-minute cap, followed by one 3-minute cleanup reserve.
Per-command ceilings are clipped to remaining time; they are not additional
budgets. The app step is 22 minutes and the telemetry job is 33 minutes, including
an always-run 2-minute evidence upload. Ordinary mode limits remain unchanged.
The `hosted-telemetry-<run>-<attempt>` artifact contains bounded reports, receipts,
UI snapshots and proxy facts. Probe and product profiles are claimed separately only at their first matching
launch after a final absence check; a late unexpected profile is retained.
Proxy handles enter the session before readiness, so failed startup cannot lose
the child. Readiness reserves bounded child-stop time inside the work cap, and
cleanup reserves existing proxy-stop time before earlier cleanup commands.
Unknown app or probe-child exit retains app, profile and staging proof; the runner records cleanup failure before stopping
its owned proxies. A confirmed proxy exit cannot erase an evidence-copy failure.
No report claims that hosted-machine disposal was verified.

Launch reports distinguish an attempt from a fixed-identity receipt confirmation.
`releasedAppLaunched` is `null` when an attempt has no confirmed receipt; that
means unknown, not never launched. Cleanup reports distinguish retained, removed
and unknown staging state; a failed recursive removal may have removed part of
its contents and always marks cleanup failed.
