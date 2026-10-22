---
name: xtrace-rubric-maker
description: Writes an xtrace-rubric.md for one installed skill, so XTrace can score a project from that skill's own output.
---

# XTrace rubric maker

You write one rubric file for one skill. XTrace runs that skill on a project,
then has Claude score the skill's own output with your rubric, giving the
project a score from 0 to 100. Your rubric says what the skill checks and how
its output becomes points.

The run's header names two folders:

- `TARGET`: a read-only copy of the skill (its `SKILL.md` and every file
  beside it).
- `OUT`: the only folder you may write to.

You write exactly one file: `OUT/xtrace-rubric.md`, or, when the skill cannot
be used to score projects (Step 2), `OUT/xtrace-refusal.md` instead. Never
both. Do not write, change or delete anything else, anywhere.
Do not run the skill, its scripts or any other program to try it out; read
its files instead. Nobody can answer questions during this run, so decide on
your own.

## How XTrace uses your rubric

Know this before you choose criteria, because it decides what the skill can
and cannot produce when it is scored:

1. XTrace runs the skill headless on one project: Claude Code follows the
   skill's `SKILL.md` with nobody at the keyboard. Nobody answers its
   questions or approves anything.
2. The project is read-only. The skill cannot change, fix, build or install
   anything in it.
3. There is no network: nothing can be downloaded, looked up or sent.
4. No MCP servers and no plugins are available, and the user's own Claude
   Code settings are not loaded. Only the skill's own files and ordinary
   command-line tools are there.
5. Everything the skill writes lands in a separate grade folder: its report,
   its files, whatever it prints.
6. Then Claude reads that output, and only that output, and scores it with
   your rubric. It does not look at the project again.

So a skill that stops to ask a question, needs the network, an MCP server or
a plugin, or needs to change the project will produce little or nothing.
Your rubric must say what to award then.

## Step 1: read the skill

1. Read `TARGET/SKILL.md` from start to end: its front matter (`name`,
   `description`) and its instructions.
2. Read the files it refers to and the other files in `TARGET` that explain
   what it does: scripts, templates, reference documents, example reports.
   Read them; never run them. Skip large data files.
3. Work out, in your own words:
   - what the skill examines in a project (code, documentation, settings,
     tests, history);
   - what its output looks like: the report or files it writes, the
     sections, scores, grades, checklists or findings it reports, and the
     severities or levels it uses;
   - which parts of that output say whether the project is in good shape.

If the header says the skill's current `xtrace-rubric.md` was refused by
XTrace, it also says why, and whether a copy of it is in `TARGET`. Write a new
one from the skill itself.

## Step 2: decide whether the skill can be scored

Refuse instead of writing a rubric when the skill cannot be used to score a
project as described above:

- it does not review a project: it builds, writes, generates, publishes,
  deploys, looks things up, or chats; or
- it cannot do its work headless, offline and read-only: it needs answers
  from a person, the network, an MCP server, a plugin, or changes to the
  project.

To refuse, write only `OUT/xtrace-refusal.md`: one short, plain reason of at
most 300 characters, in one or two sentences, with no heading, list or
formatting. For example: `This skill writes release notes from merged pull
requests; it does not review a project, so its output cannot be scored.`
Then stop; write no rubric. XTrace shows your reason to the user and does
not try again.

If the skill does review a project and can work as described, go on.

## Step 3: choose the criteria

Pick 1 to 20 criteria, usually 2 to 6. Each criterion is one thing the
skill's output says about the project, judged only from that output: the run
that uses your rubric must not examine the project again. Good criteria:

- use only checks the skill itself performs and reports. Never invent a new
  check, and never ask the scorer to judge something the skill's output does
  not state;
- follow the skill's own structure (its sections, checks or stages);
- reuse the skill's own scores, grades or severities when it has them (for
  example "start at 100 and subtract 20 for each critical finding");
- say plainly how many points to award for what.

For every criterion, also say what to award when that part of the output is
missing, when it is only partial, and when the skill stopped because it
needed something it could not get (an answer, the network, a tool, a change
to the project). Usually that is 0.

Give each criterion a whole number of points from 1 to 100. All criteria
together must add up to exactly 100. Give more points to what matters more.

## Step 4: write the file

Write `OUT/xtrace-rubric.md` in exactly this format. XTrace refuses the file
when any rule is broken.

### Header

The file starts with a line that is exactly `---`. The header ends at the
next line that is exactly `---`. Between them, one `key: value` per line,
with exactly these six keys, each once, and nothing else (no comments, no
quotes, no lists):

- `protocol: 1`
- `id:` a slug of the skill's name: lowercase letters `a`–`z`, digits and
  `-`, starting and ending with a letter or digit, at most 64 characters
  (for example `readme-check` for a skill named "README Check").
- `name:` the name people will see on XTrace's Scores page, on one line, at
  most 200 characters (usually the skill's own name).
- `version: 1` (XTrace sets the saved version itself)
- `description:` one line, at most 1,000 characters, saying what a score
  from this rubric measures.
- `target: project`

### Criteria

After the header, each criterion is a level-two heading, exactly
`## <criterion name> (<N> points)` (or `(1 point)` for one point), followed
by plain-language guidance on the lines below it. Every criterion needs
guidance. Two criteria may not share a name (ignoring case). Criterion names
are plain text of at most 200 characters, without line breaks.

Use no other `##` headings, except the optional Findings section below. Use
`###` or deeper headings, lists and paragraphs inside the guidance as you
like. Text between the header and the first `##` heading is free.

### Findings (optional)

One optional section headed exactly `## Findings`, with no points, saying
how to turn the skill's output into findings. A finding has a severity
(`critical`, `warning`, `info` or `pass`), a title, a detail and a fix. Map
the skill's own severities onto these and say where the fix comes from.

### Other rules

- The file is UTF-8 Markdown, at most 64 KiB.
- Points are whole numbers; never write fractions or percentages.
- A `##` line inside a fenced code block is guidance, not a heading; avoid
  them anyway.

## Worked example

For a skill that checks a project's README and reports "Install: ok /
incomplete / missing", "Usage: ok / missing" and a list of problems with
severities `high` and `low`:

```markdown
---
protocol: 1
id: readme-check
name: README Check
version: 1
description: How well a project's README explains installing and running it, as the README Check skill reports.
target: project
---

This rubric scores the README Check skill's report only.

## Install steps (60 points)

Award 60 when the report says the install steps are ok, 30 when it says they
are incomplete, and 0 when it says they are missing. Award 0 when the report
has no install verdict, is cut off before it, or the skill stopped early.

## Usage (40 points)

Award 40 when the report says the usage section is ok, otherwise 0
(including when the report has no usage verdict or the skill stopped early).

## Findings

One finding per problem in the report: `high` is `critical`, `low` is
`warning`. Use the problem's text as the title and its suggested change as
the fix.
```

Check before you finish: the header has exactly the six keys; every `##`
heading is a criterion with points or `## Findings`; the points add up to
exactly 100; every section has guidance, including what to award when its
output is missing. Then stop. Write nothing but `OUT/xtrace-rubric.md` (or,
when you refused, nothing but `OUT/xtrace-refusal.md`).
