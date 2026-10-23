# Metric cards and definitions

These React components display supplied values and explain their meaning. The browser preview at `/e2e/metrics.html` uses synthetic inputs; screen integration and metric computation are separate work.

| Component     | Contract                                                                                                                                                                                                                                                                                                                                                                |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MetricCell`  | `value` accepts a number, string, null, or undefined. Finite numbers use optional `format` (default `String`); nonempty strings remain verbatim. Missing, blank, NaN, and infinite values render `Unmeasured`. Sizes: 10.5, 11 (default), 18, 24, 30 px. `align="right"` fills the allocated cell and aligns its contents right.                                        |
| `Unmeasured`  | Visible em dash, screen-reader text `Unmeasured: <reason>`, and a native title. Default reason: `This value was not measured`. Supply a specific reason when known. A measured zero remains zero.                                                                                                                                                                       |
| `StatTile`    | Required `label`, `ruleId`, and `icon`; optional `value`, `format`, `reason`, `unit`, `delta`, `deltaTone`, `aside`, `tip`, and `iconTone`. A 52 px card fills its container. Icon and label sit left; value, unit, delta, and aside sit right. The whole tile is a definition button. Long labels and asides truncate visually while preserving their accessible text. |
| `SectionCard` | Required `title` and `children`; optional `ruleId`, `meta`, `right`, and `footer`. Header height defaults to 40 px, with a 36 px variant. Panel padding defaults to `12px 14px 10px`; `padding` accepts a CSS value. Each section has a unique accessible heading association. The caller owns slot contents and actions.                                               |
| `RuleChip`    | Required typed `ruleId`; `size` is `normal` (default) or `sm`. A button named `Definition <id>` exposes the definition through `aria-describedby`. Do not nest inside another button.                                                                                                                                                                                   |
| `RulePopover` | Required `ruleId` and a single native button element as `children`; optional `context` adds a caller-supplied explanation. Composes Base UI Tooltip for hover, focus, Escape, and positioning.                                                                                                                                                                          |

```tsx
<StatTile
  label="Tokens"
  ruleId="M-04"
  icon="token"
  value={null}
  reason="Cursor Agent CLI usage is absent; token totals were not measured."
/>
```

`StatTile` shows a finite supplied delta only when its main value is measured. Positive changes use ▲, negative changes use ▼, and zero uses `0%`. `deltaTone="good"` uses success color; `bad` uses warning color. Direction does not choose the tone. The caller owns sample eligibility, unit conversion, and metric interpretation.

The shared `Icon` module supplies the reviewed artwork: `lanes` (info), `merge` (accent), `clock` (success), `bolt` (danger), `msg` (warning), `token` (info), and `shield` (success). `MetricIcon` adds the 28 px icon box and default tone. `iconTone` can override it with info, accent, success, danger, warning, or meta. Icons are decorative; the adjacent label supplies their meaning.

## Formatting

Formatters use `en-US`, accept numeric inputs only, and return an em dash for null, undefined, NaN, or infinity. Negative zero is normalized. Finite negative inputs retain their sign; domain validation belongs to the producer.

| Helper             | Input                                                                                                    | Example                                                         |
| ------------------ | -------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `tokens`           | Token count; plain below 1,000, K from 1,000, M from 100,000, B from 1,000,000,000; at most two decimals | 15,100,000 → `15.1M`; 400,000 → `0.4M`; 3,720,000,000 → `3.72B` |
| `count`            | Count, rounded to an integer with grouping                                                               | 4,639 → `4,639`                                                 |
| `hours`, `minutes` | Already-converted duration, at most one decimal; no conversion or suffix                                 | 62.3 → `62.3`                                                   |
| `percent`          | Fraction, at most one decimal                                                                            | 0.18 → `18%`                                                    |
| `delta`            | Signed fractional change                                                                                 | 0.18 → `▲18%`; -0.09 → `▼9%`; 0 → `0%`                          |

### One way to write each kind of value

Each kind of value below is written by one function, and every screen that shows
it calls that function, so the same value reads the same everywhere. `—` (em
dash) always means unknown or not measured; a measured zero is shown as a zero
(`0h00m`, `0 h`, `$0.00`), never as a dash.

| Value                      | Function                                                                                 | Example                                                                                                                                                                                                                                       |
| -------------------------- | ---------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Date and clock time        | `clock`, `calendarDay`, `dayRange` (`kit/clock`)                                         | This Mac's locale and its 12- or 24-hour setting, never a forced locale or hour cycle: `Oct 5, 2:30 PM` or `5 Oct, 14:30`. A report's own time zone is kept.                                                                                  |
| Agent time                 | `agentDuration` / `agentTime` (`app/agent-duration`)                                     | 11,568,000 ms → `3h12.8m`; read aloud as `3 hours 12.8 minutes`; not a duration (NaN, negative) reads `—`. Used by Sessions, a session's page and a stretch's active time, PR sessions, the Dashboard's effort chart and lanes, and the tray. |
| Money                      | `usd` (`kit/format`)                                                                     | Cents below $100, whole dollars from $100: `$12.40`, `$250`, `$1,235`; `<$0.01` for a positive amount under a cent.                                                                                                                           |
| Host name                  | `hostName` (`kit/hosts`)                                                                 | `Claude Code`, `Codex`, `Cursor`. The account-usage widget says `Claude` on purpose: it shows the Claude account's limits.                                                                                                                    |
| Host and surface           | `surfaceLabel` (`kit/hosts`)                                                             | `Claude Code · cli`; a surface with no label is `Codex · unknown surface`.                                                                                                                                                                    |
| Pull request check status  | `refreshStateLabel` / `refreshStatusWords` / `refreshStatusText` (`dashboard/pr-effort`) | `not checked yet`, `checked`, `could not be checked (timed out)`, `stale after a failed check (rate limited)`; the Merged PRs tile counts with the same words (`3 not checked yet`).                                                          |
| Hands-off time             | `handsOffTime` / `handsOffSpoken` (`app/metric-format`)                                  | Its own quantity, in minutes: a stretch's length, a session's or PR's median, the Dashboard's median, p90 and daily line all read `3.2 min` (aloud `3.2 minutes`).                                                                            |
| Pull request link evidence | `evidenceWords` (`pr-analytics`)                                                         | `exact`, `commit`, `inferred`; the stored `sha` is never shown.                                                                                                                                                                               |
| Token+model coverage gate  | `gatePercentText`, `gateVerdict` (`pr-analytics`)                                        | Rust decides pass or fail. The percent is rounded down to one decimal, so 8,999 of 10,000 reads `89.9%`, never a passing-looking `90%`.                                                                                                       |

## Definitions and accessibility

[The reviewed definition catalog](../design/rule-contract.json) contains 30 technical definitions. `rules.ts` imports it directly and exposes typed IDs. These definitions specify product behavior; displaying them does not implement or verify the metric, privacy, or capture behavior they describe.

M-08 is Leverage: agent hours divided by human time (the time between messages you sent agents at most the break length apart), both over the same whole local days. It replaced the earlier ratio of agent time to estimated typing time (M-07) on 2026-10-05; M-07 itself is unchanged.

M-21 is Session warnings: simple rules, with no model and no score, that say a session probably went wrong somewhere. A warning fires when one rule matches, and each carries its kind, session, time, the saved records it points at and a short reason such as `same prompt sent twice (10:05, 10:12)`. One function, `session_warnings::decide` in `xt-metrics`, decides them; it is not yet in the definition catalog above. The five rules, sidechain records left out of each:

- **Correction**: a message M-02 calls a person's, sent after an agent reply, contains `wrong`, `stop`, `again` or `I said`; or starts with `no` and has at most 8 words; or has `still` with `not`, `broken`, `fails`, `error`, `same`, `wrong`, `doesn't`, `happening` or `see` within five words. `still not working` and `No, use the other file` match; `I still need an export button` and `there is no test for it yet` do not.
- **Interruption**: an input the store flagged as `[Request interrupted by user…]` (`records.is_interrupted`); Codex's own `<turn_aborted>` message; or a tool call whose saved outcome is `declined`. A declined call followed by Claude Code's marker, with no agent record between, is one interruption.
- **Circling**: an M-09 stretch that M-20's own measurement and thresholds call circling once **expected repeats** are left out (`expected_repeat`): task-list tools (`TodoWrite`, `TaskCreate`, `TaskUpdate`, `TaskList`, `TaskGet`, `update_plan`), waiting tools (`ScheduleWakeup`, `Monitor`, `sleep`, `wait`, `wait_agent`, `AwaitShell`, `BashOutput`, `TaskOutput`), tools whose name contains `screenshot`, and any call that named no command, path or pattern, which M-20 counts as the same call as every other such call of its tool. The rule reads only stored tool names and M-20 keys. M-20 itself, and the Dashboard's number, still compare every call, so a stretch of task-list updates can be circling on the Dashboard and not warned about here; the two answer different questions over the same stretches with the same formula.
- **Repeated prompt**: the same person's message, lowercased with whitespace collapsed, sent two or more times. Messages of fewer than 4 words and messages that open with a tag (`<ci-monitor-event …>`, `<task-notification>`, any `<word`) are not compared.
- **Repeated failure**: two or more calls with the same stored M-20 comparison key whose saved outcome is `failed`. For a tool whose calls name no command, path or pattern the key identifies only the tool, so the reason says `navigate failed 5 times` rather than `the same navigate call failed 5 times`.

Tool outcomes are saved when a session is indexed (migration 27, `tool_results`): which call a result answers and whether it failed (`is_error` true), was declined (Claude Code's "The user doesn't want to proceed with this tool use"), succeeded, or is unknown. No result text is kept. Claude results without `is_error` are successes (the API's default); Codex and Cursor results never state it, so their outcome is unknown and the failure rule cannot run for them. Records indexed before migration 27 have no saved outcome until their file is read again.

The text rules read the message's kept content, else the index's 280-character preview of a person's message, and compare only those first 280 characters. Each rule also says whether it read everything it needed; when words, outcomes, a classification or M-20's count were not kept or not known, it lists what was missing instead of reporting no warnings.

The [coverage report](acceptance/metrics/rule-coverage.json) records the approved-source comparison using only IDs and SHA-256 hashes. Ordinary tests check the expected ID set, important amended clauses, and catalog/report parity. The report records a completed comparison; it is not an independent approval mechanism. Definition changes require source review and an updated report.

Hover or keyboard focus opens the exact definition and optional context. The trigger is explicitly tabbable, and the popup has `role="tooltip"` with a matching `aria-describedby` association. Escape dismisses while retaining trigger focus; Tab leaves normally. The popup contains passive text and does not intercept pointer clicks or move keyboard focus. Body portals preserve the nearest `ThemeScope` and escape ancestor clipping.

The longest usage and coverage definitions are tested at 1120×720. The component follows the app's supported desktop size; it does not promise that long definitions fit arbitrarily small windows. Native title tooltips are reserved for unmeasured reasons and truncated asides, avoiding a second full-definition tooltip over the Base UI popup.

See [acceptance evidence](acceptance/metrics.md) for checks, screenshots, and preview instructions.
