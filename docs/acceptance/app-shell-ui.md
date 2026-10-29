# App shell frontend acceptance

The app composes the shared Sidebar and TopBar around Dashboard, Sessions and session details, pull requests, recorded rule activity, Settings and first-launch views. Each page reads through the same data interface used by native IPC. Leaderboard has its own page for joining the public leaderboard: before joining, the join card sits in front of a blurred list drawn from made-up rows (hidden from assistive technology, and nothing is read from the leaderboard); once joined, the membership details and controls sit above the public list read from the leaderboard, with the member's own row marked. This document covers the shell; each product page has its own acceptance checks.

## Verified behavior

- Native presence takes precedence over a browser fixture flag. Unsupported development fixtures fail explicitly; a production browser shows an honest unavailable preview.
- Source replacement gives queries a fresh cache and observers. Query keys keep reversed request completion from overwriting another range. One 500ms event coordinator coalesces updates without starving continuous imports; Strict Mode, late registration, disposal and registration failure have regression coverage.
- Navigation updates the selected Sidebar item and TopBar breadcrumb. Nine routes and the not-found page support direct load and reload. Sessions preserve PR query context as text. Cmd+, opens Settings; Cmd+K focuses a real page Search when present. Leaderboard is an ordinary Sidebar row that opens `/leaderboard`, by pointer or keyboard; Rulebook stays the kit's disabled “soon” row, and `/rulebook` still opens directly.
- Settings supports system/light/dark appearance and a database refresh. Empty host/capture information stays unknown instead of displaying sample measurements. Synthetic F1 counts are 1 session, 25 records and 15 usage rows.
- Both browsers fit the shell at 1120×720, load bundled fonts without external requests, and continue navigation while offline. Native window controls are not drawn in HTML.
- A real production build made with the fixture environment flag excludes fixture data and the adapter chunk. Both browsers show that build without a fixture badge or database counts.

## Commands and results

These are historical shell-check results, not acceptance of the source synchronization candidate. Run its affected checks and record the exact candidate commit separately.

| Check                                    | Result                                                                                              |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `pnpm check`                             | Types, lint, formatting, 74 Vitest cases and the Node lint regression pass.                         |
| `pnpm e2e`                               | 46 cases pass in WebKit and Chromium, including 6 shell cases and the shared component regressions. |
| `pnpm e2e:production`                    | 2 cases pass against the actual production build.                                                   |
| `pnpm e2e --grep 'navigates real shell'` | Both engines pass after generating the final paired Dashboard captures.                             |

Run the browser commands sequentially because they share the output directory. The default server uses port 5174 (`E2E_PORT`); production uses 5194 (`E2E_PRODUCTION_PORT`). The production suite performs the build itself. These are installed Playwright engines, not an actual Safari 17 platform-floor run.

All screenshots are synthetic browser output using the portable fixture export. The native sidebar material is verified separately in [native acceptance](app-shell-native.md).

| Engine   | Dark Dashboard                               | Light Dashboard                               |
| -------- | -------------------------------------------- | --------------------------------------------- |
| WebKit   | [Capture](app-shell/shell-webkit-dark.png)   | [Capture](app-shell/shell-webkit-light.png)   |
| Chromium | [Capture](app-shell/shell-chromium-dark.png) | [Capture](app-shell/shell-chromium-light.png) |

See [the data and routing contract](../DATA-SOURCE.md) for interfaces, event mappings and launch commands.
