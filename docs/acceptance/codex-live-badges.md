# Live session status — UI acceptance

The shared live-status hook supports Codex and Claude Code. Its native states
are Running, Waiting for approval, Waiting for input, Idle and Unknown. The UI
uses those reported states; it does not infer Running from recent activity or
copy a parent's state to a child.

Sessions shows Running as an arc around the host glyph, with an accessible
label and tooltip naming the runtime. Waiting states have textual badges.
Idle and Unknown have no badge. Dashboard shows the running arc only; it does
not show textual live-status labels for waiting, idle or unknown rows. See
[Claude live status](claude-live-status.md) for the native Claude adapter.

## Displayed chats and read limits

- Sessions has one status hook over supported-host rows currently visible in
  the Indexed sessions table, reported by `useVisibleIds`. There is no separate
  eight-session recent panel. **Recently active** sorts the whole indexed list
  and is carried through paging and session navigation.
- Dashboard has one status hook. It requests only visible supported-host rows
  from its returned lanes. A parent placeholder with no returned lane has no
  live-state request. Expanding a group lets visible children request their
  own states.
- Each hook removes duplicate exact IDs and caps its selected set at 16 across
  Claude and Codex together. It selects in the supplied row order, then sorts
  the IDs into a stable request key. Reordering the same selected set does not
  reopen its lease.
- Sessions treats an unobserved, missing or over-cap state as Unknown, without
  displaying an Unknown badge. Dashboard retains a row's last observed state
  when it scrolls offscreen, while polling only the visible selection. A new
  scope or source clears those retained states; a later idle, unknown, missing
  or failed answer clears that row's running arc.
- Fixture and preview sources without `liveSessions` make no live claim. Tests
  inject a synthetic capability explicitly. Unsupported hosts are not requested.

## Polling and cleanup

The optional DataSource method is `liveSessions.read(ids, viewId: string | null)`.
Every snapshot returns `{ view_id, states }`. On entering a nonempty selection,
the UI first calls `read([], null)` to register an empty lease. Native issues
the token. Only a still-current registration may then call
`read(selectedIds, issuedToken)`. Registration states are never displayed.
The UI generates no lease IDs and keeps no tombstones.

Later polls call `read(selectedIds, issuedToken)` every two seconds using the
same token. A hook allows at most one read chain in flight, including a pending
registration or status read from a replaced scope. A changed selection
registers a fresh empty lease after any old chain finishes. Reordering the same
selected set retains its lease. Dashboard's retained offscreen display states
are separate from this subscription lifecycle.

Read errors clear the requested states, including Running, release the captured
old token, and discard it. The next timer poll explicitly registers a fresh
empty lease and then reads the selected IDs. Native rejects expired or released
tokens; the UI never tries to recreate them. Missing or invalid states become
Unknown. A selected-read reply with a different token is not adopted.

Navigation, filter/range changes, selected-row changes, and unmount release
only their captured token. A registration that arrives after navigation or
replacement releases its issued token immediately without any selected-ID
read. A late old status read is discarded and releases its own old token
again. Neither cleanup path releases a successor. Repeated releases are
harmless; failed releases are caught. The native worker owns its 15-second
lease fallback.

## Reproducible checks

The following test files are committed in the repository:

- `apps/desktop/ui/src/app/live-session-status.test.tsx`: native-issued
  registration, transitions, expiry/errors, late replies, ID bounds, StrictMode,
  mixed hosts, missing capability and retained Dashboard states.
- `apps/desktop/ui/src/app/SessionsPage.test.tsx`: visible supported-host reads,
  the combined 16-ID cap, lease release, recently-active sorting/navigation,
  running arcs and waiting badges without Idle or Unknown badges.
- `apps/desktop/ui/src/app/dashboard/ActivityLanes.live.test.tsx`: parent/child
  identity, visibility, mixed-host bounds, running arcs without textual status,
  missing capability and offscreen state retention/clearing.
- `apps/desktop/ui/src/app/session-titles.test.tsx` and
  `apps/desktop/ui/src/app/sub-sessions.test.tsx`: title and grouped-session
  regression coverage alongside live-state reads.
- `apps/desktop/ui/e2e/running-arc.spec.ts`: synthetic Claude/Codex running arcs
  at 1120×720, in both themes and at device pixel ratios 1 and 2. It checks arc
  geometry, accessible labels, idle rows and the transition back to Idle.

Run from the repository root:

```sh
pnpm --dir apps/desktop/ui test src/app/live-session-status.test.tsx src/app/SessionsPage.test.tsx src/app/dashboard/ActivityLanes.live.test.tsx src/app/session-titles.test.tsx src/app/sub-sessions.test.tsx
pnpm --dir apps/desktop/ui typecheck
pnpm --dir apps/desktop/ui exec playwright test e2e/running-arc.spec.ts --project=chromium --project=webkit
```

These are reproducible test commands, not a record of a new test run. Record
results against the candidate being reviewed. The browser suite starts its own
fixture server through the committed Playwright configuration and uses synthetic
live states; it does not verify a real Codex socket, Claude registry transition,
native installation or macOS window behavior.
