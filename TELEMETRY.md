# Telemetry

XTrace Desktop sends a small number of anonymous usage events so we can count
how many people use the app and how often. This page lists everything that can
be sent. A test in the app checks that this list and the code match exactly.

## What is never sent

No repository names, file or folder paths, prompts or other conversation text,
session titles, pull-request URLs, token counts, costs, or any raw identifier
read from your agent history. Every property below is either a fixed word from
a short list, the app's version, or your macOS version number.

## When telemetry is on

Telemetry is **on by default**. Nothing is sent until the app has shown you a
notice that says so (on the welcome page, or once in the main window after an
update). Events attempted before that are dropped, not saved for later.

You can turn it off at any time:

- Click **Turn off** in that notice.
- Use the **Telemetry** switch in **Settings**.
- Start the app with `XTRACE_TELEMETRY=0` in its environment. Also accepted:
  `XTRACE_TELEMETRY=false`, `XTRACE_TELEMETRY=off`, `XTRACE_TELEMETRY=no`
  (any capitalization). This always wins over the saved setting, and the
  Settings switch cannot turn telemetry back on while it is set.

Turning telemetry off takes effect at once: events waiting to be sent or
retried are dropped, a request in progress is stopped where possible, and
nothing dropped is sent later, even if you turn telemetry back on. An event the
server had already accepted cannot be recalled.

Only the app built with its own identifier, `ai.xtrace.app`, and using its own
data folder ever sends. Copies built with another identifier never send
anything, and neither do runs that keep their data in another folder
(`XTRACE_DATA_DIR`) or open disposable test data (fixture mode). A build made with
`XTRACE_POSTHOG_KEY` set to an empty value has no analytics project key and
never sends anything either.

## Events

<!-- telemetry-events:start -->

| Event                  | Properties of its own | Sent by this build                                                                                       |
| ---------------------- | --------------------- | -------------------------------------------------------------------------------------------------------- |
| `app_opened`           | none                  | Yes: when the app starts.                                                                                |
| `daily_active`         | none                  | Yes: at most once per local calendar day, when the app starts or its window or menu-bar popover is used. |
| `hourly_active`        | none                  | Yes: when the main window or the menu-bar popover is used, at most once an hour.                         |
| `plugin_detected`      | `host`                | Not sent yet.                                                                                            |
| `plugin_installed`     | `host`                | Not sent yet.                                                                                            |
| `rule_activated`       | `class`, `mode`       | Not sent yet.                                                                                            |
| `leaderboard_opted_in` | none                  | Yes: when you join the public leaderboard on the Leaderboard page and your emailed code is accepted.     |
| `cloud_connected`      | none                  | Not sent yet.                                                                                            |
| `update_installed`     | none                  | Not sent yet.                                                                                            |

<!-- telemetry-events:end -->

"Not sent yet" events are defined in the code so their shape is fixed, but the
features that would send them are not in the app yet.

Values of the event-specific properties:

- `host`: `claude`, `codex` or `cursor`.
- `class`: `hook`, `linter`, `test` or `judge`.
- `mode`: `advise` or `gate`.

## Properties on every event

<!-- telemetry-common:start -->

| Property         | Values                                                                                     |
| ---------------- | ------------------------------------------------------------------------------------------ |
| `surface`        | Always `desktop`.                                                                          |
| `app_version`    | The app's version, for example `0.1.0`.                                                    |
| `os_version`     | The macOS version number, for example `15.5`, or `unknown`.                                |
| `hosts_detected` | Which coding agents the local index holds history for: any of `claude`, `codex`, `cursor`. |

<!-- telemetry-common:end -->

Each event also carries:

- `distinct_id`: a random ID created on this Mac the first time an event is
  sent, saved in the app's database. It is not derived from your name, email,
  account or hardware.
- `timestamp`: when the event was made.
- `$set_once: {"desktop_user": true}`: marks the anonymous ID as a desktop user
  in PostHog.

## Where events go

Events are sent from the app's Rust code (not the web view) as an HTTPS request
to PostHog. No
analytics script runs in the app window, and there is no automatic click
tracking or session recording.

PostHog derives an approximate location (country and city) from the request's
IP address. Whether PostHog also stores the IP address itself is a setting of
the XTrace PostHog project, outside the app; the project is meant to have IP
storage turned off so only the derived location is kept.

## Leaderboard

The public leaderboard at [leaderboard.xtrace.ai](https://leaderboard.xtrace.ai)
is separate from telemetry. Nothing is sent to it unless you join it yourself on
the app's **Leaderboard** page.

- **When you join**, the app sends the nickname and the email you type. To
  check the email is yours, the leaderboard emails you a 6-digit code, which
  you enter in the app within 15 minutes. The leaderboard shows your nickname
  and never shows your email to anyone; it uses the email only to send
  sign-in codes, which keeps out spam and fake entries. The app does not save
  the email. Joining again with the same email (for
  example on a new Mac) signs you back in to your existing entry. Only the Mac
  you joined from most recently sends your number; any other Mac is told it
  is no longer joined.
- **While you are joined**, the app sends one number: your leverage for the
  last 7 days, as the Dashboard shows it for the 7-day range, to one decimal
  (or no number when it has none). It is sent right after you join, when the
  app starts, when the app first finishes reading your history, and once an
  hour while it runs. No sessions, projects, prompts
  or other data from your history are sent. Like any web request, each one
  also lets the leaderboard see your IP address and the app's version.
- **While you are joined**, the Leaderboard page also asks the leaderboard
  for your rank and reads its public list (the nicknames and numbers anyone
  can see at leaderboard.xtrace.ai), which the page shows with your row
  marked. The rank request sends your key; the list request sends no key and
  no data of yours. Like any web request, both let the leaderboard see your
  IP address and the app's version. The page reads the list when it opens,
  again after you join or change your nickname, after each hourly send while
  the page is open, and once more about two minutes after your last join,
  nickname change or number sent, when the leaderboard's copy of the list
  (refreshed every minute or two) surely shows it. Before you join, nothing
  is read: the blurred list behind the join card is a made-up example drawn
  by the app.
- **Change nickname** sends the new nickname.
- **Leave leaderboard** deletes your nickname, email and number from the
  leaderboard. The database host's backups can keep a copy for a limited time
  after that, until they expire.

The app keeps the key the leaderboard gave it when you joined, your nickname
and the last number it sent in its own database on this Mac. Only the app
built with its own identifier and using its own data folder sends to the
leaderboard; other copies, such as a test copy of the database, send nothing
to it. A copy started with `XTRACE_LEADERBOARD_URL` pointing at a leaderboard
server on this Mac sends to that local server only.

Turning telemetry off does not stop leaderboard updates, and leaving the
leaderboard does not change telemetry. When you join with telemetry on, the
`leaderboard_opted_in` event above is sent once your code is accepted; it says
only that someone joined, not who. Signing back in to an existing entry does
not send it.

## Retention

On this Mac, the app keeps a record of the last 200 events it tried to send,
with the properties sent and whether each was sent, is still waiting, or failed.
Older entries are deleted automatically. In PostHog, events are kept according
to the XTrace project's retention settings.

## Planned, not in this build

When connecting to XTrace Cloud exists, connecting will link this anonymous ID
to your XTrace account, so usage before and after connecting counts as one
person. The connect screen will say so before you connect. This build has no
cloud connection, so the ID is never linked to an account.
