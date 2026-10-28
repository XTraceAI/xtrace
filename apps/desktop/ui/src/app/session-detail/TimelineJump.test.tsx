import type { SessionWarningItem } from '../../data/generated/SessionWarningItem';
import {
  answer as warningAnswer,
  dismissedAnswer,
  warning as suppliedWarning,
  second as suppliedSecond,
} from '../../../e2e/session-warnings.data';
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { queryKeys } from '../../data/query-client';
import { refreshQueries } from '../../data/subscribe-invalidation';
import type { SessionPageSelection } from '../../data/generated/SessionPageSelection';
import type { SessionPageStatus } from '../../data/generated/SessionPageStatus';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource } from '../../data/DataSource';
import { events, type DataEvent } from '../../data/ipc-names';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { MetricSessionStretch } from '../../data/generated/MetricSessionStretch';
import type { MetricSessionStretches } from '../../data/generated/MetricSessionStretches';
import type { MetricToolBlock } from '../../data/generated/MetricToolBlock';
import type { SessionSourceStatus } from '../../data/generated/SessionSourceStatus';
import type { SourceRecord } from '../../data/generated/SourceRecord';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';

/**
 * A stretch's first tool call, from the timeline to the transcript, on the real
 * page. Every identity, record and stretch here is written by the test.
 */
const exported = fixture as FixtureExport;
const template = exported.sessions[0].rows[0];
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));

const SESSION = '11111111-0000-4000-8000-000000000001';
const OTHER_SESSION = '11111111-0000-4000-8000-000000000002';
const CALLER = '22222222-2222-4222-8222-222222222222';
const LATER = '33333333-3333-4333-8333-333333333333';

let scrolled: { element: Element; options: unknown }[];
beforeEach(() => {
  scrolled = [];
  // jsdom lays nothing out and has no scrolling; record what would have been
  // scrolled, and on which element.
  Element.prototype.scrollIntoView = function (this: Element, options?: unknown) {
    scrolled.push({ element: this, options });
  } as Element['scrollIntoView'];
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
  delete (Element.prototype as Partial<Element>).scrollIntoView;
  delete (window as Partial<Window>).matchMedia;
});

const records = (caller = CALLER): SourceRecord[] => [
  {
    id: caller,
    role: 'assistant',
    at: null,
    blocks: [
      { kind: 'text', index: 0, text: 'Reading and running.' },
      { kind: 'tool_call', index: 1, name: 'Read', call_id: 'toolu_1', input: null },
      { kind: 'tool_call', index: 2, name: 'Bash', call_id: 'toolu_2', input: null },
    ],
  },
  {
    id: LATER,
    role: 'assistant',
    at: null,
    blocks: [{ kind: 'tool_call', index: 0, name: 'Grep', call_id: 'toolu_3', input: null }],
  },
];

const loaded = (list: SourceRecord[] = records()): SessionSourceStatus => ({
  state: 'loaded',
  generation: { generation: 'indexed', appended: false },
  sources: 1,
  record_ranges: [{ start: 0, end: list.length }],
  dropped_records: 0,
  gaps: [],
  records: list,
});

const stretch = (
  minute: number,
  first_tool: MetricSessionStretch['first_tool'],
): MetricSessionStretch => ({
  start_uuid: `start-${minute}`,
  end_uuid: `end-${minute}`,
  start: `2026-09-07T12:${String(minute).padStart(2, '0')}:00Z`,
  end: `2026-09-07T12:${String(minute + 3).padStart(2, '0')}:00Z`,
  duration_ms: 180_000,
  first_tool,
  // M-20 over the same stretch: measured, nothing repeated.
  active_duration_ms: 180_000,
  repeats: { state: 'measured', repeats: 0, worst: null },
  circling: false,
});

/** The metric's default thresholds, as the answer carries them. */
const THRESHOLDS = { active_ms: 240_000, repeats: 5 };

/** One valid target, each negative case, and a second valid target elsewhere. */
const measured: MetricSessionStretches = {
  state: 'measured',
  stretches: [
    stretch(0, { record_uuid: CALLER, block_index: 1 }), // Read, in a closed row
    stretch(5, null), // no locator
    stretch(10, { record_uuid: CALLER, block_index: 0 }), // a text block
    stretch(15, { record_uuid: '44444444-4444-4444-8444-444444444444', block_index: 0 }), // absent
    stretch(20, { record_uuid: LATER, block_index: 0 }), // Grep, in another row
  ],
  repeat_thresholds: THRESHOLDS,
};

function source(options: {
  transcript?: SessionSourceStatus | 'hold' | 'fail';
  transcripts?: Record<string, SessionSourceStatus>;
  stretches?:
    | MetricSessionStretches
    | ((days: number, call: number) => MetricSessionStretches | Promise<MetricSessionStretches>);
}) {
  const opened: string[] = [];
  const stretchReads: [string, number][] = [];
  const listeners = new Map<DataEvent, Set<() => void>>();
  let settle: (status: SessionSourceStatus) => void = () => {};
  const data: DataSource = {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => exported.app_info,
    dbCounts: async () => exported.db_counts,
    nativeIndexStatus: async () => exported.native_index,
    dashboard: async (days) => exported.dashboards.find((d) => d.window.days === days)!,
    tokensByHost: async (days) => {
      const report = exported.dashboards.find((d) => d.window.days === days)!;
      return { window: report.window, hosts: report.tokens_by_host };
    },
    today: async () => exported.today,
    sessionsList: async () => ({ window: exported.sessions[0].window, rows: [], next: null }),
    sessionRow: async (sessionId) => ({
      ...template,
      id: sessionId,
      host: 'claude',
      repo: '/Users/example/code/atlas',
      branch: 'feat/timeline',
    }),
    sessionStretches: async (sessionId, days) => {
      stretchReads.push([sessionId, days]);
      const given = options.stretches;
      return typeof given === 'function' ? given(days, stretchReads.length) : (given ?? measured);
    },
    sessionWarningContext: async () => ({
      state: 'unavailable' as const,
      reason: { reason: 'source' as const, source: { reason: 'not_indexed' as const } },
    }),
    sessionTranscript: (sessionId) => {
      opened.push(sessionId);
      const given = options.transcripts?.[sessionId] ?? options.transcript ?? loaded();
      if (given === 'fail') return Promise.reject(new Error('the command failed'));
      if (given === 'hold') return new Promise((resolve) => (settle = resolve));
      return Promise.resolve(given);
    },
    cancelSessionTranscript: async () => {},
    // Pull-request refresh is not exercised by this test.
    pullRequests: async () => Promise.reject(new Error('not used by this test')),
    pullRequestAnalytics: async () => Promise.reject(new Error('not used by this test')),
    pullRequestSessions: async () => Promise.reject(new Error('not used by this test')),
    refreshPullRequests: async () => Promise.reject(new Error('not used by this test')),
    cancelPullRequestRefresh: async () => Promise.reject(new Error('not used by this test')),
    subscribe: async (event, listener) => {
      const set = listeners.get(event) ?? new Set();
      set.add(listener);
      listeners.set(event, set);
      return () => void set.delete(listener);
    },
  };
  return {
    data,
    opened,
    stretchReads,
    settle: (status: SessionSourceStatus) => settle(status),
    /** The app's own refresh path: an ingest event invalidates its queries. */
    emit: (event: DataEvent) => listeners.get(event)?.forEach((listener) => listener()),
  };
}

function Go({ to }: { to: string }) {
  const navigate = useNavigate();
  return (
    <button type="button" onClick={() => void navigate(to)}>
      go {to}
    </button>
  );
}

function mount(data: DataSource, path = `/sessions/${SESSION}`, to?: string, probe?: ReactNode) {
  return render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={[path]}>
          {probe}
          {to && <Go to={to} />}
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

const segments = () =>
  within(screen.getByRole('list', { name: 'Hands-off stretches, in order' })).getAllByRole(
    'button',
  );
const segment = async (index: number) => {
  await screen.findByRole('list', { name: 'Hands-off stretches, in order' });
  return segments()[index];
};
/** Wait until the transcript's turns are on the page. */
const transcriptReady = () => screen.findByRole('button', { name: /Read 1 file/ });
const row = (name: RegExp) => screen.getByRole('button', { name });
const highlighted = () => [...document.querySelectorAll('.xt-tool-card[data-highlighted]')];
const live = () =>
  screen
    .getAllByRole('status')
    .find(
      (node) =>
        node.getAttribute('aria-live') === 'polite' && node.closest('.xt-session-conversation'),
    );

it('reveals a stretch’s first call by opening its closed row, and leaves focus where it was', async () => {
  const { data } = source({});
  mount(data);
  await transcriptReady();
  const readRow = row(/Read 1 file/);
  expect(readRow.getAttribute('aria-expanded')).toBe('false');

  const first = await segment(0);
  first.focus();
  fireEvent.click(first);

  // The row that holds the call opened, and only that one.
  expect(readRow.getAttribute('aria-expanded')).toBe('true');
  expect(row(/Ran 1 command/).getAttribute('aria-expanded')).toBe('false');
  // The card now standing where the anchor stood is the one highlighted.
  const [card] = highlighted();
  expect(highlighted()).toHaveLength(1);
  expect(card.getAttribute('data-block-id')).toBe(`${CALLER}:1`);
  expect(card.getAttribute('data-tool')).toBe('Read');
  // Scrolled to, gently, and nothing else was.
  expect(scrolled).toEqual([{ element: card, options: { block: 'center', behavior: 'smooth' } }]);
  // Focus stays on the segment the reader pressed.
  expect(document.activeElement).toBe(first);
  expect(first.getAttribute('aria-pressed')).toBe('true');
  // And it is said out loud.
  expect(live()?.textContent).toBe('Showing the first tool call of activity period 1: Read.');
});

it('respects a reader who asked for less motion', async () => {
  // Only the reduced-motion query answers yes; anything else the app asks
  // (the theme, say) answers no, as it would on a real machine.
  window.matchMedia = vi.fn((query: string) => ({
    matches: query === '(prefers-reduced-motion: reduce)',
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as Window['matchMedia'];
  const { data } = source({});
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(scrolled.map((entry) => entry.options)).toEqual([{ block: 'center', behavior: 'auto' }]);
  expect(window.matchMedia).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');
});

it('leaves an already-open row open rather than toggling it shut', async () => {
  const { data } = source({});
  mount(data);
  await transcriptReady();
  fireEvent.click(row(/Read 1 file/));
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(await segment(0));
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('true');
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);
});

it.each([
  [1, 'Which tool call this stretch started with was not recorded.'],
  [2, 'The block recorded at that place in the transcript is not a tool call.'],
  [3, 'That tool call is not in the transcript that was read.'],
])(
  'keeps stretch %i measured and says why its first call cannot be shown',
  async (index, reason) => {
    const { data } = source({});
    mount(data);
    await transcriptReady();
    const pressed = await segment(index);
    expect(pressed.getAttribute('aria-label')).toContain(reason);
    fireEvent.click(pressed);
    // Still a segment, still pressed, still showing its duration.
    expect(pressed.getAttribute('aria-pressed')).toBe('true');
    expect(within(pressed).getByText('3 min')).toBeTruthy();
    // Nothing opened, nothing highlighted, nothing scrolled.
    expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('false');
    expect(highlighted()).toHaveLength(0);
    expect(scrolled).toHaveLength(0);
    expect(live()?.textContent).toBe(
      `The first tool call of activity period ${index + 1} cannot be shown. ${reason}`,
    );
  },
);

it('refuses a position two records share, even when one of them is a tool call', async () => {
  // A fork can carry two records under one UUID; the id on the page is then
  // shared, and neither node can be chosen.
  const forked = [
    ...records(),
    {
      id: CALLER,
      role: 'assistant' as const,
      at: null,
      blocks: [{ kind: 'text' as const, index: 1, text: 'copied' }],
    },
  ];
  const { data } = source({ transcript: loaded(forked) });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
  expect(live()?.textContent).toMatch(/more than one block/i);
});

it('moves the highlight when another stretch is pressed', async () => {
  const { data } = source({});
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);
  fireEvent.click(segments()[4]);
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${LATER}:0`]);
  expect(live()?.textContent).toBe('Showing the first tool call of activity period 5: Grep.');
});

it('reveals without building a selector out of an identifier from the history', async () => {
  // The document is where a confirmed target is shown, never where one is
  // looked for: no query anywhere during a reveal names the block.
  const { data } = source({});
  mount(data);
  await transcriptReady();
  const onDocument = vi.spyOn(Document.prototype, 'querySelector');
  const onDocumentAll = vi.spyOn(Document.prototype, 'querySelectorAll');
  const onElement = vi.spyOn(Element.prototype, 'querySelector');
  const onElementAll = vi.spyOn(Element.prototype, 'querySelectorAll');
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);
  const queried = [onDocument, onDocumentAll, onElement, onElementAll]
    .flatMap((spy) => spy.mock.calls)
    .map(([selector]) => String(selector));
  expect(queried.filter((selector) => selector.includes(CALLER))).toEqual([]);
});

it('does not act later on a press made while the transcript was still being read', async () => {
  const { data, settle } = source({ transcript: 'hold' });
  mount(data);
  const first = await segment(0);
  expect(first.getAttribute('aria-label')).toContain('The transcript is still being read.');
  fireEvent.click(first);
  expect(live()?.textContent).toContain('The transcript is still being read.');
  // The transcript arrives. The earlier press does not quietly fire now: it
  // was answered, and a jump the reader did not ask for again is not theirs.
  await act(async () => settle(loaded()));
  await transcriptReady();
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('false');
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
  // Pressing it again, now, does.
  fireEvent.click(segments()[0]);
  expect(highlighted()).toHaveLength(1);
});

it('never carries a reveal into another session that happens to share the position', async () => {
  // The other session's transcript has a record under the same UUID with a
  // tool call at the same index — the one place a leaked request could land.
  const { data } = source({
    transcripts: { [OTHER_SESSION]: loaded(records(CALLER)) },
  });
  mount(data, `/sessions/${SESSION}`, `/sessions/${OTHER_SESSION}`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);
  scrolled.length = 0;

  fireEvent.click(screen.getByRole('button', { name: `go /sessions/${OTHER_SESSION}` }));
  await waitFor(() => expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0));
  await transcriptReady();
  expect(row(/Read 1 file/).getAttribute('aria-expanded')).toBe('false');
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
});

it('re-reads the stretches for a new range and never re-reads the transcript', async () => {
  const { data, opened, stretchReads } = source({});
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);

  // The page owns the range control, because what it shows is measured over it.
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(stretchReads).toContainEqual([SESSION, 30]));
  // One read of the text, however many windows its measurements are taken over.
  expect(opened).toEqual([SESSION]);
  // The press belonged to the old window's stretches, so it is cleared with it.
  await waitFor(() => expect(highlighted()).toHaveLength(0));
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
});

it('keeps the stretches and the measurements when the transcript cannot be read', async () => {
  const { data } = source({ transcript: 'fail' });
  mount(data);
  const first = await segment(0);
  await waitFor(() =>
    expect(first.getAttribute('aria-label')).toContain('The transcript is not available to show.'),
  );
  expect(segments()).toHaveLength(5);
  expect(screen.getByText(/indexed records · whole session$/)).toBeTruthy();
  fireEvent.click(first);
  expect(live()?.textContent).toContain('The transcript is not available to show.');
});

it('reveals the right call when identities differ only in whitespace and carry punctuation', async () => {
  const odd = ' a:b "c" ';
  const bare = 'a:b "c"';
  const list: SourceRecord[] = [
    {
      id: bare,
      role: 'assistant',
      at: null,
      blocks: [
        { kind: 'text', index: 0, text: 'bare' },
        { kind: 'tool_call', index: 1, name: 'Glob', call_id: 'toolu_bare', input: null },
      ],
    },
    {
      id: odd,
      role: 'assistant',
      at: null,
      blocks: [
        { kind: 'text', index: 0, text: 'padded' },
        { kind: 'tool_call', index: 1, name: 'Read', call_id: 'toolu_odd', input: null },
      ],
    },
  ];
  const { data } = source({
    transcript: loaded(list),
    stretches: {
      state: 'measured',
      stretches: [stretch(0, { record_uuid: odd, block_index: 1 })],
      repeat_thresholds: THRESHOLDS,
    },
  });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${odd}:1`]);
  expect(highlighted()[0].getAttribute('data-tool')).toBe('Read');
});

it('keeps focus on the segment across Enter and Space', async () => {
  // jsdom does not turn a key into a click; a native button does, which is
  // why every segment is one. What is held here is that pressing it from the
  // keyboard's own element leaves the reader on that element.
  const { data } = source({});
  mount(data);
  await transcriptReady();
  const first = await segment(0);
  first.focus();
  fireEvent.click(document.activeElement as HTMLElement);
  expect(document.activeElement).toBe(first);
  const last = segments()[4];
  last.focus();
  fireEvent.click(document.activeElement as HTMLElement);
  expect(document.activeElement).toBe(last);
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${LATER}:0`]);
});

it('does not bring a press back when the reader returns to the window it was made in', async () => {
  // A press is an index into one list. Hidden while another window is
  // selected, it would come back pointing at whatever now sits at that index;
  // it has to be gone, not hidden.
  const { data, stretchReads } = source({});
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted()).toHaveLength(1);

  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(stretchReads).toContainEqual([SESSION, 30]));
  await waitFor(() => expect(highlighted()).toHaveLength(0));

  // Back to the window the press was made in. The answer is still cached, so
  // the very same list comes back — which is exactly the case where a merely
  // hidden press would reappear, pointing at a stretch nobody selected.
  fireEvent.click(screen.getByRole('radio', { name: '7d' }));
  await screen.findByText('Measured over the last 7 days');
  await waitFor(() => expect(segments()).toHaveLength(5));
  expect(highlighted()).toHaveLength(0);
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(live()?.textContent).toBe('');
});

it('does not keep a press across a replaced answer for the same window', async () => {
  // The same window, asked again, comes back reordered. The index the reader
  // pressed is now a different stretch, so the press cannot survive it.
  const reordered: MetricSessionStretches = {
    state: 'measured',
    stretches: [...(measured.state === 'measured' ? measured.stretches : [])].reverse(),
    repeat_thresholds: THRESHOLDS,
  };
  const { data, stretchReads, emit } = source({
    stretches: (_days, call) => (call === 1 ? measured : reordered),
  });
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);

  // The app's own refresh: an ingest event invalidates the session queries and
  // the answer comes back different for the very same window.
  act(() => emit(events.importReceived));
  await waitFor(() => expect(stretchReads.length).toBeGreaterThan(1), { timeout: 3000 });
  await waitFor(() => expect(segments()[0].getAttribute('aria-label')).toContain('Stretch 1 of 5'));
  // Nothing selected, nothing highlighted, and nothing said about a stretch
  // that is no longer the one at that index.
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(highlighted()).toHaveLength(0);
  expect(live()?.textContent).toBe('');
});

it.each([
  [1, 'Which tool call this stretch started with was not recorded.'],
  [2, 'The block recorded at that place in the transcript is not a tool call.'],
  [3, 'That tool call is not in the transcript that was read.'],
])(
  'shows stretch %i’s reason on the screen, not only to a screen reader',
  async (index, reason) => {
    const { data } = source({});
    mount(data);
    await transcriptReady();
    fireEvent.click(await segment(index));
    const note = live()!;
    expect(note.textContent).toContain(reason);
    // Seen as well as announced: jsdom applies no stylesheet, so what is checked
    // is that the note is not the class this app hides things with, and is not
    // taken out of the accessibility tree either.
    expect(note.className).toBe('xt-timeline-note');
    expect(note.className).not.toContain('sr-only');
    expect(note.getAttribute('aria-hidden')).toBeNull();
    expect(note.getAttribute('data-reason')).toBeTruthy();
  },
);

it('shows why a position naming two blocks cannot be jumped to', async () => {
  const forked = [
    ...records(),
    {
      id: CALLER,
      role: 'assistant' as const,
      at: null,
      blocks: [{ kind: 'text' as const, index: 1, text: 'copied' }],
    },
  ];
  const { data } = source({ transcript: loaded(forked) });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  const note = live()!;
  expect(note.textContent).toMatch(/more than one block/i);
  expect(note.className).toBe('xt-timeline-note');
  expect(note.getAttribute('data-reason')).toBe('ambiguous');
});

it('shows why nothing can be jumped to while the transcript is unavailable', async () => {
  const { data } = source({ transcript: 'fail' });
  mount(data);
  const first = await segment(0);
  await waitFor(() =>
    expect(first.getAttribute('aria-label')).toContain('The transcript is not available to show.'),
  );
  fireEvent.click(first);
  const note = live()!;
  expect(note.textContent).toContain('The transcript is not available to show.');
  expect(note.className).toBe('xt-timeline-note');
  expect(note.getAttribute('data-reason')).toBe('transcript_unavailable');
});

/*
 * M-20's most repeated call, from the timeline to the transcript. The position
 * is the earliest call of the stretch's largest repeated group; it goes through
 * the very same confirmation and the very same reveal as a first call.
 */

/** A measured M-20 answer for one stretch, naming its most repeated call. */
const repeating = (
  base: MetricSessionStretch,
  tool: string,
  count: number,
  representative: MetricToolBlock,
  circling: boolean,
): MetricSessionStretch => ({
  ...base,
  active_duration_ms: 300_000,
  repeats: {
    state: 'measured',
    repeats: count - 1,
    worst: { tool_name: tool, count, representative },
  },
  circling,
});

const withRepeats: MetricSessionStretches = {
  state: 'measured',
  stretches: [
    // Starts with Read; Bash, the call after it, repeated seven times.
    repeating(
      stretch(0, { record_uuid: CALLER, block_index: 1 }),
      'Bash',
      7,
      {
        record_uuid: CALLER,
        block_index: 2,
      },
      true,
    ),
    // Its most repeated call is the very call it started with.
    repeating(
      stretch(5, { record_uuid: LATER, block_index: 0 }),
      'Grep',
      3,
      {
        record_uuid: LATER,
        block_index: 0,
      },
      false,
    ),
    // Not counted: an older index never compared these calls.
    {
      ...stretch(10, { record_uuid: CALLER, block_index: 1 }),
      repeats: { state: 'unknown', reason: 'missing_key' },
      circling: null,
    },
  ],
  repeat_thresholds: THRESHOLDS,
};

const showRepeat = () => screen.getByRole('button', { name: /^Show repeated call/ });

it('shows a stretch’s repeated call through the same reveal, and keeps focus on the button', async () => {
  const { data } = source({ stretches: withRepeats });
  mount(data);
  await transcriptReady();
  // Nothing is offered until a stretch is chosen.
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  fireEvent.click(await segment(0));
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:1`]);
  expect(screen.getByText(/the most repeated call was Bash, 7 calls/)).toBeTruthy();

  scrolled.length = 0;
  const button = showRepeat();
  expect(button.tagName).toBe('BUTTON');
  button.focus();
  fireEvent.click(button);
  // The Bash row opened, its call is the one highlighted, and it was scrolled to.
  expect(row(/Ran 1 command/).getAttribute('aria-expanded')).toBe('true');
  const [card] = highlighted();
  expect(highlighted()).toHaveLength(1);
  expect(card.getAttribute('data-block-id')).toBe(`${CALLER}:2`);
  expect(card.getAttribute('data-tool')).toBe('Bash');
  expect(scrolled.map((entry) => entry.element)).toEqual([card]);
  // The stretch stays the selected one, focus stays where the reader pressed,
  // and what happened is said.
  expect(segments()[0].getAttribute('aria-pressed')).toBe('true');
  expect(document.activeElement).toBe(button);
  expect(live()?.textContent).toBe('Showing the earliest matching call: Bash.');
});

it('reveals the same call again when the repeated call is the one the stretch started with', async () => {
  // The two positions coincide. Each press is its own request, so the second
  // is answered rather than taken for the first.
  const { data } = source({ stretches: withRepeats });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(1));
  expect(live()?.textContent).toBe('Showing the first tool call of activity period 2: Grep.');
  scrolled.length = 0;
  fireEvent.click(showRepeat());
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${LATER}:0`]);
  expect(scrolled).toHaveLength(1);
  expect(live()?.textContent).toBe('Showing the earliest matching call: Grep.');
});

it('refuses a repeated call whose position two records share', async () => {
  const forked = [
    ...records(),
    {
      id: CALLER,
      role: 'assistant' as const,
      at: null,
      blocks: [{ kind: 'text' as const, index: 2, text: 'copied' }],
    },
  ];
  const { data } = source({ stretches: withRepeats, transcript: loaded(forked) });
  mount(data);
  await transcriptReady();
  fireEvent.click(await segment(0));
  scrolled.length = 0;
  const button = showRepeat();
  expect(button.getAttribute('aria-label')).toMatch(/cannot be shown: More than one block/);
  fireEvent.click(button);
  // Nothing moved to a guess: the earlier highlight went with the earlier press.
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
  expect(live()?.textContent).toBe(
    'The repeated call of activity period 1 cannot be shown. More than one block in the transcript sits where that call was recorded, so which one is meant is not known.',
  );
  expect(live()?.getAttribute('data-reason')).toBe('ambiguous');
});

it('keeps what M-20 measured and says why a repeated call cannot be shown without the transcript', async () => {
  const { data } = source({ stretches: withRepeats, transcript: 'fail' });
  mount(data);
  const first = await segment(0);
  await waitFor(() =>
    expect(first.getAttribute('aria-label')).toContain('The transcript is not available to show.'),
  );
  // Measured from the index, so shown whatever the text says.
  expect(within(first).getByText('0 h 5 m active · 6 repeats · Bash ×7')).toBeTruthy();
  expect(within(first).getByText('Circling')).toBeTruthy();
  fireEvent.click(first);
  fireEvent.click(showRepeat());
  expect(highlighted()).toHaveLength(0);
  expect(live()?.textContent).toBe(
    'The repeated call of activity period 1 cannot be shown. The transcript is not available to show.',
  );
});

it('offers no repeated call for a stretch whose repeats were not counted, and says why', async () => {
  const { data } = source({ stretches: withRepeats });
  mount(data);
  await transcriptReady();
  const unknown = await segment(2);
  expect(within(unknown).getByText('0 h 3 m active · repeats unknown')).toBeTruthy();
  expect(unknown.getAttribute('data-circling')).toBeNull();
  fireEvent.click(unknown);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  expect(
    screen.getByText(
      'Stretch 3: repeats not counted. Some of its tool calls were indexed before this app compared calls, or their arguments could not be compared.',
    ),
  ).toBeTruthy();
});

it('drops a repeated-call press when the range changes', async () => {
  const { data, stretchReads } = source({ stretches: withRepeats });
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  fireEvent.click(showRepeat());
  expect(highlighted().map((card) => card.getAttribute('data-block-id'))).toEqual([`${CALLER}:2`]);

  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  await waitFor(() => expect(stretchReads).toContainEqual([SESSION, 30]));
  await waitFor(() => expect(highlighted()).toHaveLength(0));
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  // And back: the cached answer returns, the press does not.
  fireEvent.click(screen.getByRole('radio', { name: '7d' }));
  await screen.findByText('Measured over the last 7 days');
  await waitFor(() => expect(segments()).toHaveLength(3));
  expect(highlighted()).toHaveLength(0);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  expect(live()?.textContent).toBe('');
});

it('drops a repeated-call press when the answer for the same window is replaced', async () => {
  // Refreshed, the same window says the stretches in another order. The press
  // named a place in the old list, and its call is not re-found in the new one.
  const reordered: MetricSessionStretches = {
    ...withRepeats,
    stretches: [...(withRepeats.state === 'measured' ? withRepeats.stretches : [])].reverse(),
  };
  const { data, stretchReads, emit } = source({
    stretches: (_days, call) => (call === 1 ? withRepeats : reordered),
  });
  mount(data, `/sessions/${SESSION}?range=7d`);
  await transcriptReady();
  fireEvent.click(await segment(0));
  fireEvent.click(showRepeat());
  expect(highlighted()).toHaveLength(1);

  act(() => emit(events.importReceived));
  await waitFor(() => expect(stretchReads.length).toBeGreaterThan(1), { timeout: 3000 });
  await waitFor(() => expect(segments()[0].getAttribute('data-repeats')).toBe('unknown'));
  expect(screen.queryAllByRole('button', { pressed: true })).toHaveLength(0);
  expect(highlighted()).toHaveLength(0);
  expect(screen.queryByRole('button', { name: /^Show repeated call/ })).toBeNull();
  expect(live()?.textContent).toBe('');
});

it('navigates every native repeat representative without selecting initially or replacing the conversation', async () => {
  const answer = structuredClone(withRepeats) as Extract<
    MetricSessionStretches,
    { state: 'measured' }
  >;
  answer.stretches.push(
    repeating(stretch(20, null), 'Missing', 9, { record_uuid: 'absent', block_index: 7 }, false),
  );
  const { data } = source({ stretches: answer });
  mount(data);
  await transcriptReady();
  const navigation = screen.getByRole('navigation', { name: 'Repeated activity' });
  const next = within(navigation).getByRole('button', { name: 'Next ↓' });
  expect(navigation.textContent).toContain('3 notices');
  expect(highlighted()).toHaveLength(0);
  expect(scrolled).toHaveLength(0);
  const before = [...document.querySelectorAll('[data-record-id]')];
  next.focus();
  fireEvent.click(next);
  expect(navigation.textContent).toContain('1 of 3');
  expect(document.activeElement).toBe(next);
  expect(highlighted()[0]?.getAttribute('data-block-id')).toBe(`${CALLER}:2`);
  fireEvent.click(next);
  expect(navigation.textContent).toContain('2 of 3');
  expect(highlighted()[0]?.getAttribute('data-block-id')).toBe(`${LATER}:0`);
  fireEvent.click(next);
  expect(navigation.textContent).toContain('3 of 3');
  expect(highlighted()).toHaveLength(0);
  expect(document.querySelector('.xt-session-conversation [role="status"]')?.textContent).toContain(
    'not in the transcript',
  );
  expect([...document.querySelectorAll('[data-record-id]')]).toEqual(before);
});

it('keeps one active announcement when Agent activity measurements are open', async () => {
  const { data } = source({ stretches: withRepeats });
  mount(data);
  await transcriptReady();
  fireEvent.click(screen.getByText(/Agent activity · Last/));
  fireEvent.click(screen.getByRole('button', { name: 'Next ↓' }));
  expect(
    screen
      .getAllByRole('status')
      .filter((node) => node.textContent === 'Showing the earliest matching call: Bash.'),
  ).toHaveLength(1);
  expect(document.querySelector('.xt-timeline [role="status"]')?.textContent).toBe('');
});

it('enables adjacent record groups only for one native source, never for unknown legacy source boundaries', async () => {
  const single = loaded([records()[1]!, { ...records()[1]!, id: 'another' }]);
  const multiple = { ...single, sources: 2, record_ranges: [] };
  const { data } = source({ transcripts: { [SESSION]: single, [OTHER_SESSION]: multiple } });
  const other = `/sessions/${OTHER_SESSION}`;
  mount(data, `/sessions/${SESSION}`, other);
  await waitFor(() => expect(document.querySelectorAll('[data-record-id]')).toHaveLength(2));
  expect(screen.getAllByRole('button', { name: /Tool activity/ })).toHaveLength(1);
  fireEvent.click(screen.getByRole('button', { name: `go ${other}` }));
  await waitFor(() =>
    expect(screen.getAllByRole('button', { name: /Tool activity/ })).toHaveLength(2),
  );
  expect(screen.queryByRole('separator')).toBeNull();
});

const pageText = (text: string): SourceRecord[] => [
  { id: text, role: 'user', at: null, blocks: [{ kind: 'text', index: 0, text }] },
];
function sessionPage(selection: SessionPageSelection): SessionPageStatus {
  const body =
    selection.kind === 'anchor'
      ? records()
      : pageText(selection.kind === 'first' ? 'synthetic beginning' : 'synthetic later portion');
  const start = selection.kind === 'first' ? 0 : selection.kind === 'anchor' ? 100 : 200;
  return {
    state: 'loaded',
    digest: 'a'.repeat(64),
    records: body,
    start,
    end: start + body.length,
    generation: { generation: 'unrecorded' },
    record_ranges: [{ start: 0, end: body.length }],
    dropped_records: 0,
    total_records: 300,
    previous: start ? 'previous-cursor' : null,
    next: 'next-cursor',
    target:
      selection.kind === 'anchor'
        ? { record: selection.record_uuid === CALLER ? 0 : 1, block: selection.block }
        : null,
  };
}
function codexSource(
  limit: 'source_bytes' | 'output_bytes' = 'source_bytes',
  host: 'claude' | 'codex' = 'codex',
) {
  const first = stretch(0, { record_uuid: CALLER, block_index: 1 });
  first.repeats = {
    state: 'measured',
    repeats: 8,
    worst: { tool_name: 'Grep', count: 9, representative: { record_uuid: LATER, block_index: 0 } },
  };
  const h = source({
    transcript: {
      state: 'unavailable',
      reason:
        host === 'claude'
          ? { reason: 'too_large', limit: 'bytes', reached: 99, ceiling: 10 }
          : { reason: 'reader_limit', limit },
    },
    stretches: { state: 'measured', stretches: [first], repeat_thresholds: THRESHOLDS },
  });
  const originalRow = h.data.sessionRow;
  h.data.sessionRow = async (...args) => {
    const row = await originalRow(...args);
    return row ? { ...row, host } : null;
  };
  const reads: { selection: SessionPageSelection; id: string }[] = [];
  const cancelled: string[] = [];
  h.data.sessionTranscriptPage = async (_session, selection, id) => {
    reads.push({ selection, id });
    return sessionPage(selection);
  };
  h.data.cancelSessionTranscript = async (id) => {
    cancelled.push(id);
  };
  return { ...h, reads, cancelled };
}
it.each([
  ['codex', 'source_bytes'],
  ['codex', 'output_bytes'],
  ['claude', 'source_bytes'],
] as const)(
  'pages large %s conversations after %s refusal and checks exact first/repeated calls',
  async (host, limit) => {
    const h = codexSource(limit, host);
    mount(h.data);
    expect(await screen.findByText('synthetic beginning')).toBeTruthy();
    expect(screen.getByText('Showing messages 1–1 of 300.')).toBeTruthy();
    const later = screen.getByRole('button', { name: 'Later messages' });
    later.focus();
    fireEvent.click(later);
    expect(await screen.findByText('synthetic later portion')).toBeTruthy();
    expect(document.activeElement).toBe(later);
    expect(h.reads[1].selection).toEqual({ kind: 'cursor', cursor: 'next-cursor' });
    fireEvent.click(screen.getByRole('button', { name: 'Earlier messages' }));
    await waitFor(() =>
      expect(h.reads[2].selection).toEqual({ kind: 'cursor', cursor: 'previous-cursor' }),
    );
    const first = await segment(0);
    expect(first.getAttribute('aria-label')).toContain('loads and checks');
    first.focus();
    fireEvent.click(first);
    await screen.findByText('Showing the first tool call of activity period 1: Read.');
    expect(highlighted()[0].getAttribute('data-block-id')).toBe(`${CALLER}:1`);
    expect(document.activeElement).toBe(first);
    expect(h.reads.at(-1)?.selection).toEqual({
      kind: 'anchor',
      expected: 'tool_call',
      record_uuid: CALLER,
      block: 1,
      generation: 'a'.repeat(64),
    });
    expect(screen.queryByText('No result for this call is in the record.')).toBeNull();
    const repeat = screen.getByRole('button', { name: 'Show repeated call: Grep, stretch 1' });
    repeat.focus();
    fireEvent.click(repeat);
    await screen.findByText('Showing the earliest matching call: Grep.');
    expect(highlighted()[0].getAttribute('data-block-id')).toBe(`${LATER}:0`);
    expect(document.activeElement).toBe(repeat);
    const before = h.reads.length;
    fireEvent.click(repeat);
    await waitFor(() => expect(h.reads.length).toBe(before + 1));
    expect(h.reads.at(-1)?.selection).toEqual({
      kind: 'anchor',
      expected: 'tool_call',
      record_uuid: LATER,
      block: 0,
      generation: 'a'.repeat(64),
    });
  },
);
it.each(['codex', 'claude'] as const)(
  'drops a delayed page when a newer exact call takes over and cancels on exit (%s)',
  async (host) => {
    const h = codexSource('source_bytes', host);
    let old: (value: SessionPageStatus) => void = () => {};
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      if (selection.kind === 'cursor')
        return new Promise((resolve) => {
          old = resolve;
        });
      return sessionPage(selection);
    };
    mount(h.data, `/sessions/${SESSION}`, '/sessions');
    await screen.findByText('synthetic beginning');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    await waitFor(() => expect(h.reads).toHaveLength(2));
    const pending = h.reads[1].id;
    fireEvent.click(await segment(0));
    await screen.findByText('Showing the first tool call of activity period 1: Read.');
    expect(h.cancelled).toContain(pending);
    await act(async () =>
      old({
        ...sessionPage({ kind: 'first' }),
        state: 'loaded',
        records: pageText('obsolete delayed content'),
        digest: 'a'.repeat(64),
        start: 0,
        end: 1,
        generation: { generation: 'unrecorded' },
        record_ranges: [{ start: 0, end: 1 }],
        dropped_records: 0,
        total_records: 1,
        previous: null,
        next: null,
        target: null,
      }),
    );
    expect(screen.queryByText('obsolete delayed content')).toBeNull();
    const current = h.reads.at(-1)!.id;
    fireEvent.click(screen.getByRole('button', { name: 'go /sessions' }));
    await waitFor(() => expect(h.cancelled).toContain(current));
    expect(highlighted()).toHaveLength(0);
  },
);
it.each(['codex', 'claude'] as const)(
  'refuses a changed generation and explicitly starts a fresh one (%s)',
  async (host) => {
    const h = codexSource('source_bytes', host);
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      return selection.kind === 'first'
        ? sessionPage(selection)
        : { state: 'unavailable', reason: { reason: 'changed' } };
    };
    mount(h.data);
    await screen.findByText('synthetic beginning');
    fireEvent.click(await segment(0));
    await screen.findByText(
      'This conversation changed while you were reading it. Load it again to continue.',
    );
    expect(screen.queryByText('synthetic beginning')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Load conversation again' }));
    await screen.findByText('synthetic beginning');
    expect(h.reads.at(-1)?.selection).toEqual({ kind: 'first' });
  },
);
it.each(['codex', 'claude'] as const)(
  'keeps an exact-anchor changed refusal across a changed background stretches answer until explicit reload (%s)',
  async (host) => {
    const h = codexSource('source_bytes', host);
    let anchorReply: (status: SessionPageStatus) => void = () => {};
    let refreshed = false;
    const originalStretches = h.data.sessionStretches;
    h.data.sessionStretches = async (...args) => {
      const answer = await originalStretches(...args);
      return refreshed && answer.state === 'measured'
        ? {
            ...answer,
            stretches: answer.stretches.map((stretch) => ({
              ...stretch,
              duration_ms: stretch.duration_ms + 60_000,
            })),
          }
        : answer;
    };
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      return selection.kind === 'anchor'
        ? new Promise((resolve) => {
            anchorReply = resolve;
          })
        : sessionPage(selection);
    };
    mount(h.data);
    await screen.findByText('synthetic beginning');
    const first = await segment(0);
    const originalLabel = first.getAttribute('aria-label');
    fireEvent.click(first);
    await waitFor(() => expect(h.reads).toHaveLength(2));
    expect(h.reads[1].selection).toEqual({
      kind: 'anchor',
      expected: 'tool_call',
      record_uuid: CALLER,
      block: 1,
      generation: 'a'.repeat(64),
    });
    await act(async () => anchorReply({ state: 'unavailable', reason: { reason: 'changed' } }));
    const changed =
      'This conversation changed while you were reading it. Load it again to continue.';
    await screen.findByText(changed);
    expect(first.getAttribute('aria-pressed')).toBe('true');
    refreshed = true;
    act(() => h.emit(events.turnCompleted));
    await waitFor(() => expect(first.getAttribute('aria-label')).not.toBe(originalLabel));
    await waitFor(() => expect(first.getAttribute('aria-pressed')).toBe('false'));
    expect(h.reads).toHaveLength(2);
    expect(h.reads.filter((read) => read.selection.kind === 'first')).toHaveLength(1);
    expect(screen.getByText(changed)).toBeTruthy();
    expect(screen.queryByText('synthetic beginning')).toBeNull();
    expect(highlighted()).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Load conversation again' }));
    await screen.findByText('synthetic beginning');
    expect(h.reads).toHaveLength(3);
    expect(h.reads[2].selection).toEqual({ kind: 'first' });
  },
);
it.each([
  ['codex', loaded()],
  ['cursor', { state: 'unavailable', reason: { reason: 'reader_limit', limit: 'source_bytes' } }],
  ['codex', { state: 'unavailable', reason: { reason: 'reader_limit', limit: 'native_rows' } }],
] as const)('keeps the existing full-reader behavior for %s / %#', async (host, status) => {
  const h = source({ transcript: status as SessionSourceStatus });
  const originalRow = h.data.sessionRow;
  h.data.sessionRow = async (...args) => {
    const row = await originalRow(...args);
    return row ? { ...row, host } : null;
  };
  h.data.sessionTranscriptPage = vi.fn(async () => sessionPage({ kind: 'first' }));
  mount(h.data);
  await screen.findByRole('heading', { name: 'atlas · feat/timeline' });
  await waitFor(() => expect(h.opened).toHaveLength(1));
  expect(h.data.sessionTranscriptPage).not.toHaveBeenCalled();
  expect(screen.queryByRole('navigation', { name: 'Conversation portions' })).toBeNull();
});

it('pages every measured repeat group including unavailable calls, with checked notices and one conversation announcement', async () => {
  const h = codexSource();
  h.data.sessionStretches = async () => ({
    state: 'measured',
    repeat_thresholds: THRESHOLDS,
    stretches: [
      repeating(stretch(0, null), 'Read', 7, { record_uuid: CALLER, block_index: 1 }, false),
      repeating(stretch(5, null), 'Grep', 9, { record_uuid: LATER, block_index: 0 }, true),
      repeating(stretch(10, null), 'Missing', 8, { record_uuid: 'absent', block_index: 0 }, false),
    ],
  });
  h.data.sessionTranscriptPage = async (_session, selection, id) => {
    h.reads.push({ selection, id });
    return selection.kind === 'anchor' && selection.record_uuid === 'absent'
      ? { state: 'unavailable', reason: { reason: 'read', detail: { reason: 'target_missing' } } }
      : sessionPage(selection);
  };
  mount(h.data);
  await screen.findByText('synthetic beginning');
  const navigation = screen.getByRole('navigation', { name: 'Repeated activity' });
  const next = within(navigation).getByRole('button', { name: 'Next ↓' });
  const previous = within(navigation).getByRole('button', { name: '↑ Previous' });
  expect(navigation.textContent).toContain('3 notices');
  expect(document.querySelectorAll('[data-repeat-index]')).toHaveLength(0);
  next.focus();
  fireEvent.click(next);
  await screen.findByText('Showing the earliest matching call: Read.');
  expect(navigation.textContent).toContain('1 of 3');
  expect(highlighted()[0]?.getAttribute('data-block-id')).toBe(`${CALLER}:1`);
  expect(document.querySelectorAll('[data-repeat-index]')).toHaveLength(2);
  expect(document.activeElement).toBe(next);
  fireEvent.click(screen.getByText(/Agent activity · Last/));
  fireEvent.click(next);
  await screen.findByText('Showing the earliest matching call: Grep.');
  expect(navigation.textContent).toContain('2 of 3');
  expect(highlighted()[0]?.getAttribute('data-block-id')).toBe(`${LATER}:0`);
  expect(
    screen
      .getAllByRole('status')
      .filter((node) => node.textContent === 'Showing the earliest matching call: Grep.'),
  ).toHaveLength(1);
  expect(document.querySelector('.xt-timeline [role="status"]')?.textContent).toBe('');
  fireEvent.click(next);
  await screen.findByText(/The repeated call of activity period 3 cannot be shown/);
  expect(navigation.textContent).toContain('3 of 3');
  expect(highlighted()).toHaveLength(0);
  previous.focus();
  fireEvent.click(previous);
  await screen.findByText('Showing the earliest matching call: Grep.');
  expect(navigation.textContent).toContain('2 of 3');
  expect(document.activeElement).toBe(previous);
  expect(h.reads.filter((read) => read.selection.kind === 'first')).toHaveLength(1);
});
it('does not create a bounded inline notice from a loadable locator or a text block at that position', async () => {
  const h = codexSource();
  h.data.sessionStretches = async () => ({
    state: 'measured',
    repeat_thresholds: THRESHOLDS,
    stretches: [
      repeating(stretch(0, null), 'Read', 7, { record_uuid: CALLER, block_index: 1 }, false),
    ],
  });
  h.data.sessionTranscriptPage = async (_session, selection, id) => {
    h.reads.push({ selection, id });
    const page = sessionPage(selection);
    if (selection.kind === 'first' && page.state === 'loaded')
      return {
        ...page,
        records: [
          {
            id: CALLER,
            role: 'assistant',
            at: null,
            blocks: [{ kind: 'text', index: 1, text: 'synthetic text at the recorded position' }],
          },
        ],
      };
    return page;
  };
  mount(h.data);
  await screen.findByText('synthetic text at the recorded position');
  expect(document.querySelectorAll('[data-repeat-index]')).toHaveLength(0);
  fireEvent.click(screen.getByRole('button', { name: 'Next ↓' }));
  await screen.findByText('Showing the earliest matching call: Read.');
  expect(document.querySelectorAll('[data-repeat-index]')).toHaveLength(1);
  expect(document.querySelector('[data-repeat-index]')?.getAttribute('data-current')).toBe('true');
});

it.each([
  ['codex', 'later'],
  ['codex', 'changed'],
  ['claude', 'later'],
  ['claude', 'changed'],
] as const)(
  'preserves the %s bounded %s state while an uncached range row is pending and after it settles',
  async (host, state) => {
    const h = codexSource('source_bytes', host);
    const readRow = h.data.sessionRow;
    let settleRow: ((row: Awaited<ReturnType<DataSource['sessionRow']>>) => void) | null = null;
    h.data.sessionRow = (id, days) =>
      days === 30
        ? new Promise((resolve) => {
            settleRow = resolve;
          })
        : readRow(id, days);
    if (state === 'changed') {
      h.data.sessionTranscriptPage = async (_session, selection, id) => {
        h.reads.push({ selection, id });
        return selection.kind === 'first'
          ? sessionPage(selection)
          : { state: 'unavailable', reason: { reason: 'changed' } };
      };
    }
    mount(h.data, `/sessions/${SESSION}?range=7d`);
    await screen.findByText('synthetic beginning');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    const text =
      state === 'later'
        ? 'synthetic later portion'
        : 'This conversation changed while you were reading it. Load it again to continue.';
    await screen.findByText(text);
    const activeRead = h.reads.at(-1)!.id;
    fireEvent.click(screen.getByRole('radio', { name: '30d' }));
    await waitFor(() => expect(settleRow).not.toBeNull());
    const whilePending = screen.queryByText(text);
    const cancelledWhilePending = h.cancelled.includes(activeRead);
    await act(async () => settleRow!(await readRow(SESSION, 30)));
    await screen.findByText('Measured in this range');
    expect({
      visibleWhilePending: whilePending !== null,
      cancelledWhilePending,
      requests: h.reads.map((read) => read.selection.kind),
    }).toEqual({
      visibleWhilePending: true,
      cancelledWhilePending: false,
      requests: ['first', 'cursor'],
    });
    expect(screen.getByText(text)).toBeTruthy();
    expect(h.reads.filter((read) => read.selection.kind === 'first')).toHaveLength(1);
    if (state === 'changed') {
      fireEvent.click(screen.getByRole('button', { name: 'Load conversation again' }));
      await screen.findByText('synthetic beginning');
      expect(h.reads.at(-1)!.selection).toEqual({ kind: 'first' });
    }
  },
);

it.each(['missing', 'claude'] as const)(
  'ends the bounded read when a successful new range row is %s',
  async (host) => {
    const h = codexSource();
    const readRow = h.data.sessionRow;
    h.data.sessionRow = async (id, days) => {
      const found = await readRow(id, days);
      return days !== 30 ? found : host === 'missing' ? null : { ...found!, host };
    };
    mount(h.data, `/sessions/${SESSION}?range=7d`);
    await screen.findByText('synthetic beginning');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    await screen.findByText('synthetic later portion');
    const activeRead = h.reads.at(-1)!.id;
    fireEvent.click(screen.getByRole('radio', { name: '30d' }));
    await waitFor(() => expect(h.cancelled).toContain(activeRead));
    expect(screen.queryByText('synthetic later portion')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Later messages' })).toBeNull();
    expect(h.reads).toHaveLength(2);
  },
);

it.each(['codex', 'claude'] as const)(
  'starts a fresh bounded read when the actual session changes (%s)',
  async (host) => {
    const h = codexSource('source_bytes', host);
    const readPage = h.data.sessionTranscriptPage!;
    const sessions: string[] = [];
    h.data.sessionTranscriptPage = (session, selection, id) => {
      sessions.push(session);
      return readPage(session, selection, id);
    };
    const other = `/sessions/${OTHER_SESSION}`;
    mount(h.data, `/sessions/${SESSION}`, other);
    await screen.findByText('synthetic beginning');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    await screen.findByText('synthetic later portion');
    const activeRead = h.reads.at(-1)!.id;
    fireEvent.click(screen.getByRole('button', { name: `go ${other}` }));
    await screen.findByText('synthetic beginning');
    expect(sessions).toEqual([SESSION, SESSION, OTHER_SESSION]);
    expect(h.reads.at(-1)!.selection).toEqual({ kind: 'first' });
    expect(h.cancelled).toContain(activeRead);
    expect(highlighted()).toHaveLength(0);
  },
);

it.each(['codex', 'claude'] as const)(
  'retains the bounded portion when a new range row fails to load (%s)',
  async (host) => {
    const h = codexSource('source_bytes', host);
    const readRow = h.data.sessionRow;
    h.data.sessionRow = (id, days) =>
      days === 30 ? Promise.reject(new Error('Synthetic row read failure')) : readRow(id, days);
    mount(h.data, `/sessions/${SESSION}?range=7d`);
    await screen.findByText('synthetic beginning');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    await screen.findByText('synthetic later portion');
    const activeRead = h.reads.at(-1)!.id;
    fireEvent.click(screen.getByRole('radio', { name: '30d' }));
    await screen.findByText(/This session’s measurements could not be loaded/);
    expect(screen.getByText('synthetic later portion')).toBeTruthy();
    expect(h.cancelled).not.toContain(activeRead);
    expect(h.reads).toHaveLength(2);
  },
);

it.each([
  ['codex', 'later'],
  ['codex', 'changed'],
  ['claude', 'later'],
  ['claude', 'changed'],
] as const)(
  'preserves the %s bounded %s state while a stale cached-null range row is refetching',
  async (host, state) => {
    const h = codexSource('source_bytes', host);
    const readRow = h.data.sessionRow;
    let settleRow: ((row: Awaited<ReturnType<DataSource['sessionRow']>>) => void) | null = null;
    let rowReads = 0;
    h.data.sessionRow = (id, days) =>
      days === 30
        ? new Promise((resolve) => {
            rowReads += 1;
            settleRow = resolve;
          })
        : readRow(id, days);
    if (state === 'changed') {
      h.data.sessionTranscriptPage = async (_session, selection, id) => {
        h.reads.push({ selection, id });
        return selection.kind === 'first'
          ? sessionPage(selection)
          : { state: 'unavailable', reason: { reason: 'changed' } };
      };
    }
    let client: QueryClient | undefined;
    function ClientProbe() {
      client = useQueryClient();
      return null;
    }
    mount(h.data, `/sessions/${SESSION}?range=7d`, undefined, <ClientProbe />);
    await screen.findByText('synthetic beginning');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    const text =
      state === 'later'
        ? 'synthetic later portion'
        : 'This conversation changed while you were reading it. Load it again to continue.';
    await screen.findByText(text);
    const activeRead = h.reads.at(-1)!.id;
    const key = queryKeys.session(30, SESSION);
    // An inactive answer from before discovery remains cached when the real refresh path invalidates it.
    await act(async () => {
      client!.setQueryData(key, null);
      refreshQueries(client!, [key], () => {
        throw new Error('Synthetic refresh failed');
      });
    });
    expect(client!.getQueryState(key)).toMatchObject({
      data: null,
      status: 'success',
      fetchStatus: 'idle',
      isInvalidated: true,
    });
    expect(client!.getQueryCache().find({ queryKey: key })!.getObserversCount()).toBe(0);
    expect(rowReads).toBe(0);
    fireEvent.click(screen.getByRole('radio', { name: '30d' }));
    await waitFor(() => expect(settleRow).not.toBeNull());
    const cacheWhilePending = client!.getQueryState(key);
    expect(cacheWhilePending).toMatchObject({
      data: null,
      status: 'success',
      fetchStatus: 'fetching',
    });
    const visibleWhilePending = screen.queryByText(text) !== null;
    const cancelledWhilePending = h.cancelled.includes(activeRead);
    await act(async () => settleRow!(await readRow(SESSION, 30)));
    await screen.findByText('Measured in this range');
    expect({
      visibleWhilePending,
      cancelledWhilePending,
      requests: h.reads.map((read) => read.selection.kind),
    }).toEqual({
      visibleWhilePending: true,
      cancelledWhilePending: false,
      requests: ['first', 'cursor'],
    });
    expect(screen.getByText(text)).toBeTruthy();
    expect(rowReads).toBe(1);
  },
);

function warningPage(selection: SessionPageSelection): SessionPageStatus {
  const page = sessionPage(selection);
  if (selection.kind !== 'anchor' || selection.expected === 'tool_call' || page.state !== 'loaded')
    return page;
  const record: SourceRecord =
    selection.block === null
      ? {
          id: selection.record_uuid,
          role: 'user',
          at: null,
          blocks: [
            { kind: 'text', index: 0, text: `Original warning message ${selection.record_uuid}` },
          ],
        }
      : {
          id: selection.record_uuid,
          role: 'user',
          at: null,
          blocks: [
            {
              kind: 'tool_result',
              index: selection.block,
              call_id: 'original-call',
              failed: true,
              parts: [{ kind: 'text', text: `Original warning result ${selection.record_uuid}` }],
            },
          ],
        };
  return {
    ...page,
    records: [record],
    record_ranges: [{ start: 0, end: 1 }],
    target: { record: 0, block: selection.block },
    end: page.start + 1,
  };
}

it.each(['claude', 'codex'] as const)(
  'selects exact %s warning targets without extra reads on Dismiss, Undo or same-point refetch',
  async (host) => {
    const h = codexSource('source_bytes', host);
    let current = warningAnswer;
    h.data.sessionWarnings = async (ids, timezone) => ({
      timezone,
      sessions: ids.map((session_id) => ({ session_id, answer: current })),
    });
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      return warningPage(selection);
    };
    Object.defineProperty(h.data, 'warningDismissals', {
      value: {
        dismiss: async (session_id: string, warning_id: string) => {
          current = dismissedAnswer;
          return { session_id, warning_id, dismissed: true };
        },
        restore: async (session_id: string, warning_id: string) => {
          current = warningAnswer;
          return { session_id, warning_id, dismissed: false };
        },
      },
    });
    let client!: QueryClient;
    function ClientProbe() {
      client = useQueryClient();
      return null;
    }
    mount(
      h.data,
      `/sessions/${SESSION}?warning=${encodeURIComponent(suppliedWarning.id)}`,
      undefined,
      <ClientProbe />,
    );
    await screen.findByText('Showing the warning’s original tool block.');
    expect(h.reads.filter((read) => read.selection.kind === 'anchor')).toHaveLength(1);
    fireEvent.click(await screen.findByRole('button', { name: suppliedSecond.name }));
    await screen.findByText('Original warning message other');
    await screen.findByText('Showing the original warning message.');
    expect(h.reads.filter((read) => read.selection.kind === 'anchor')).toHaveLength(2);
    const selected = screen.getByRole('list', { name: 'Selected warning' });
    expect(selected.contains(document.activeElement)).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: suppliedWarning.name }));
    await screen.findByText('Showing the warning’s original tool block.');
    expect(h.reads.filter((read) => read.selection.kind === 'anchor')).toHaveLength(3);
    const reads = h.reads.length;
    const activeRead = h.reads.at(-1)!.id;
    fireEvent.click(within(selected).getByRole('button', { name: 'Dismiss' }));
    const undo = await within(selected).findByRole('button', { name: 'Undo' });
    await waitFor(() => expect(document.activeElement).toBe(undo));
    fireEvent.click(undo);
    await waitFor(() =>
      expect(document.activeElement).toBe(
        within(selected).getByRole('button', { name: 'Dismiss' }),
      ),
    );
    current = { ...warningAnswer, summary: warningAnswer.summary + ' refreshed' };
    await act(() => client.invalidateQueries({ queryKey: ['sessions', 'warnings'] }));
    expect(h.reads).toHaveLength(reads);
    expect(h.cancelled).not.toContain(activeRead);
  },
);

it.each(['missing', 'cursor'] as const)(
  'ends the Claude bounded read when a successful range row is %s',
  async (nextHost) => {
    const h = codexSource('source_bytes', 'claude');
    const readRow = h.data.sessionRow;
    h.data.sessionRow = async (id, days) => {
      const row = await readRow(id, days);
      return days !== 30 ? row : nextHost === 'missing' ? null : { ...row!, host: nextHost };
    };
    mount(h.data, `/sessions/${SESSION}?range=7d`);
    await screen.findByText('synthetic beginning');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    await screen.findByText('synthetic later portion');
    const activeRead = h.reads.at(-1)!.id;
    fireEvent.click(screen.getByRole('radio', { name: '30d' }));
    await waitFor(() => expect(h.cancelled).toContain(activeRead));
    expect(screen.queryByText('synthetic later portion')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Later messages' })).toBeNull();
    expect(h.reads).toHaveLength(2);
  },
);

it('starts a fresh Claude page and cancels the old read when the data source changes', async () => {
  const first = codexSource('source_bytes', 'claude');
  const second = codexSource('source_bytes', 'claude');
  const view = mount(first.data);
  await screen.findByText('synthetic beginning');
  fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
  await screen.findByText('synthetic later portion');
  const activeRead = first.reads.at(-1)!.id;
  view.rerender(
    <ThemeProvider>
      <DataProvider source={second.data}>
        <MemoryRouter initialEntries={[`/sessions/${SESSION}`]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await screen.findByText('synthetic beginning');
  expect(first.cancelled).toContain(activeRead);
  expect(second.reads.map((read) => read.selection.kind)).toEqual(['first']);
  expect(screen.queryByText('synthetic later portion')).toBeNull();
});

it('keeps adjacent Claude tool messages in separate source groups', async () => {
  const h = codexSource('source_bytes', 'claude');
  h.data.sessionTranscriptPage = async (_session, selection, id) => {
    h.reads.push({ selection, id });
    const page = sessionPage(selection);
    if (page.state !== 'loaded') return page;
    return {
      ...page,
      records: ['primary', 'agent'].map((source) => ({
        id: `${source}-message`,
        role: 'assistant',
        at: null,
        blocks: [{ kind: 'tool_call', index: 0, name: 'Read', call_id: source, input: null }],
      })),
      start: 0,
      end: 2,
      total_records: 2,
      previous: null,
      next: null,
      record_ranges: [
        { start: 0, end: 1 },
        { start: 1, end: 2 },
      ],
    };
  };
  mount(h.data);
  const groups = await screen.findAllByRole('button', { name: 'Tool activity · 1 saved message' });
  expect(groups).toHaveLength(2);
  expect(screen.queryByRole('button', { name: 'Tool activity · 2 saved messages' })).toBeNull();
  groups.forEach((group) => fireEvent.click(group));
  expect(screen.queryByText('No result for this call is in the record.')).toBeNull();
});

function supplyWarnings(h: ReturnType<typeof source>, items: SessionWarningItem[]) {
  h.data.sessionWarnings = async (ids, timezone) => ({
    timezone,
    sessions: ids.map((session_id) => ({
      session_id,
      answer: { ...warningAnswer, warnings: items, dismissed: [] },
    })),
  });
}
const messageWarning = {
  ...suppliedWarning,
  points_at: { record_uuid: 'warning-message', block: null, sent_by: 'person' as const },
};
const resultWarning = {
  ...suppliedSecond,
  points_at: { record_uuid: 'warning-result', block: 128, sent_by: 'person' as const },
};

it.each(['claude', 'codex'] as const)(
  'reveals exact full-read %s warning messages and high-index results without another read',
  async (host) => {
    const original: SourceRecord[] = [
      {
        id: 'warning-message',
        role: 'user',
        at: null,
        blocks: [
          { kind: 'text', index: 0, text: 'Original correction ' + 'long text '.repeat(1000) },
        ],
      },
      {
        id: 'warning-result',
        role: 'user',
        at: null,
        blocks: [
          {
            kind: 'tool_result',
            index: 128,
            call_id: 'original-result',
            failed: true,
            parts: [{ kind: 'text', text: 'Long original result ' + 'result words '.repeat(1000) }],
          },
        ],
      },
    ];
    const h = source({ transcript: loaded(original) });
    const row = h.data.sessionRow;
    h.data.sessionRow = async (...args) => {
      const found = await row(...args);
      return found ? { ...found, host } : null;
    };
    h.data.sessionTranscriptPage = vi.fn(async () => sessionPage({ kind: 'first' }));
    supplyWarnings(h, [messageWarning, resultWarning]);
    mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`);
    await screen.findByText('Showing the original warning message.');
    const message = document.querySelector('[data-record-id="warning-message"]');
    expect(message?.getAttribute('data-highlighted')).toBe('true');
    expect(scrolled.some((item) => item.element === message)).toBe(true);
    const selected = screen.getByRole('list', { name: 'Selected warning' });
    expect(selected.contains(document.activeElement)).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: resultWarning.name }));
    await screen.findByText('Showing the warning’s original tool block.');
    expect(
      document
        .querySelector('[data-block-id="warning-result:128"]')
        ?.getAttribute('data-highlighted'),
    ).toBe('true');
    expect(
      screen
        .getByRole('button', { name: 'Tool activity · 1 saved message' })
        .getAttribute('aria-expanded'),
    ).toBe('true');
    expect(selected.contains(document.activeElement)).toBe(true);
    expect(h.opened).toHaveLength(1);
    expect(h.data.sessionTranscriptPage).not.toHaveBeenCalled();
  },
);

it.each(['claude', 'codex'] as const)(
  'keeps a %s changed-source refusal across warning changes until explicit reload',
  async (host) => {
    const h = codexSource('source_bytes', host);
    supplyWarnings(h, [messageWarning, resultWarning]);
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      return selection.kind === 'anchor' && selection.record_uuid === 'warning-result'
        ? { state: 'unavailable', reason: { reason: 'changed' } }
        : warningPage(selection);
    };
    mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`);
    await screen.findByText('Showing the original warning message.');
    fireEvent.click(screen.getByRole('button', { name: resultWarning.name }));
    await screen.findAllByText(/This conversation changed while you were reading it/);
    const count = h.reads.length;
    fireEvent.click(screen.getByRole('button', { name: messageWarning.name }));
    await waitFor(() =>
      expect(screen.getByRole('list', { name: 'Selected warning' }).textContent).toContain(
        messageWarning.reason,
      ),
    );
    expect(h.reads).toHaveLength(count);
    expect(document.querySelectorAll('.xt-conversation-body [data-highlighted]')).toHaveLength(0);
    expect(screen.queryByText('Showing the original warning message.')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Load conversation again' }));
    await screen.findByText('synthetic beginning');
    expect(h.reads.at(-1)!.selection).toEqual({ kind: 'first' });
    fireEvent.click(screen.getByRole('button', { name: messageWarning.name }));
    await screen.findByText('Showing the original warning message.');
    expect(
      h.reads.filter(
        (read) =>
          read.selection.kind === 'anchor' && read.selection.record_uuid === 'warning-message',
      ),
    ).toHaveLength(2);
  },
);

it.each(['claude', 'codex'] as const)(
  'keeps %s warning feedback after a cursor read reports a changed source',
  async (host) => {
    const h = codexSource('source_bytes', host);
    supplyWarnings(h, [messageWarning, resultWarning]);
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      return selection.kind === 'cursor'
        ? { state: 'unavailable', reason: { reason: 'changed' } }
        : warningPage(selection);
    };
    mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`);
    await screen.findByText('Showing the original warning message.');
    fireEvent.click(screen.getByRole('button', { name: 'Later messages' }));
    await waitFor(() => expect(live()?.textContent).toMatch(/This conversation changed/));
    const count = h.reads.length;
    fireEvent.click(screen.getByRole('button', { name: resultWarning.name }));
    await waitFor(() => expect(live()?.textContent).toMatch(/This conversation changed/));
    expect(h.reads).toHaveLength(count);
    expect(document.querySelectorAll('.xt-conversation-body [data-highlighted]')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: 'Load conversation again' }));
    await screen.findByText('synthetic beginning');
    expect(h.reads.at(-1)!.selection).toEqual({ kind: 'first' });
  },
);

it.each(['claude', 'codex'] as const)(
  'drops a stale %s warning anchor when a newer warning takes over',
  async (host) => {
    const h = codexSource('source_bytes', host);
    supplyWarnings(h, [messageWarning, resultWarning]);
    let finish!: (page: SessionPageStatus) => void;
    let pending!: SessionPageSelection;
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      if (selection.kind === 'anchor' && selection.record_uuid === 'warning-message') {
        pending = selection;
        return new Promise((resolve) => {
          finish = resolve;
        });
      }
      return warningPage(selection);
    };
    mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`);
    await waitFor(() => expect(finish).toBeDefined());
    const stale = h.reads.at(-1)!.id;
    fireEvent.click(screen.getByRole('button', { name: resultWarning.name }));
    await screen.findByText('Showing the warning’s original tool block.');
    await act(() => finish(warningPage(pending)));
    expect(screen.queryByText('Original warning message warning-message')).toBeNull();
    expect(
      document
        .querySelector('[data-block-id="warning-result:128"]')
        ?.getAttribute('data-highlighted'),
    ).toBe('true');
    expect(h.cancelled).toContain(stale);
  },
);

it.each(
  (['claude', 'codex'] as const).flatMap((host) =>
    (['wrong_uuid', 'wrong_role', 'wrong_block', 'duplicate_uuid'] as const).map(
      (mismatch) => [host, mismatch] as const,
    ),
  ),
)('refuses a %s bounded warning when the page returns %s', async (host, mismatch) => {
  const h = codexSource('source_bytes', host);
  const item = mismatch === 'wrong_block' ? resultWarning : messageWarning;
  supplyWarnings(h, [item]);
  h.data.sessionTranscriptPage = async (_session, selection, id) => {
    h.reads.push({ selection, id });
    const page = warningPage(selection);
    if (page.state !== 'loaded' || selection.kind !== 'anchor') return page;
    if (mismatch === 'wrong_uuid') page.records[0]!.id = 'nearby';
    if (mismatch === 'wrong_role') page.records[0]!.role = 'assistant';
    if (mismatch === 'wrong_block') page.target!.block = 0;
    if (mismatch === 'duplicate_uuid') {
      page.records.push({ ...page.records[0]! });
      page.end += 1;
      page.record_ranges[0]!.end = 2;
    }
    return page;
  };
  mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(item.id)}`);
  await screen.findByText(/The original warning message cannot be shown/);
  expect(document.querySelectorAll('.xt-conversation-body [data-highlighted]')).toHaveLength(0);
  expect(screen.queryByText('Showing the original warning message.')).toBeNull();
});

it('lets a timeline press take over a warning until the same warning is explicitly selected again', async () => {
  const h = codexSource('source_bytes', 'claude');
  supplyWarnings(h, [messageWarning]);
  h.data.sessionTranscriptPage = async (_session, selection, id) => {
    h.reads.push({ selection, id });
    return warningPage(selection);
  };
  mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`);
  await screen.findByText('Showing the original warning message.');
  fireEvent.click(await segment(0));
  await screen.findByText('Showing the first tool call of activity period 1: Read.');
  expect(
    document.querySelector('[data-block-id="' + CALLER + ':1"]')?.getAttribute('data-highlighted'),
  ).toBe('true');
  expect(
    h.reads.filter(
      (read) => read.selection.kind === 'anchor' && read.selection.expected === 'message',
    ),
  ).toHaveLength(1);
  fireEvent.click(screen.getByRole('button', { name: messageWarning.name }));
  await screen.findByText('Showing the original warning message.');
  expect(
    h.reads.filter(
      (read) => read.selection.kind === 'anchor' && read.selection.expected === 'message',
    ),
  ).toHaveLength(2);
});

it.each(['claude', 'codex'] as const)(
  'keeps the %s warning target through a pending cached-null range row and new stretches',
  async (host) => {
    const h = codexSource('source_bytes', host);
    supplyWarnings(h, [messageWarning]);
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      return warningPage(selection);
    };
    const readRow = h.data.sessionRow;
    let settle!: (row: Awaited<ReturnType<DataSource['sessionRow']>>) => void;
    h.data.sessionRow = (id, days) =>
      days === 30
        ? new Promise((resolve) => {
            settle = resolve;
          })
        : readRow(id, days);
    let client!: QueryClient;
    function Probe() {
      client = useQueryClient();
      return null;
    }
    mount(
      h.data,
      `/sessions/${SESSION}?range=7d&warning=${encodeURIComponent(messageWarning.id)}`,
      undefined,
      <Probe />,
    );
    await screen.findByText('Showing the original warning message.');
    const target = document.querySelector('[data-record-id="warning-message"]');
    const count = h.reads.length;
    const active = h.reads.at(-1)!.id;
    const key = queryKeys.session(30, SESSION);
    await act(async () => {
      client.setQueryData(key, null);
      await client.invalidateQueries({ queryKey: key, refetchType: 'none' });
    });
    fireEvent.click(screen.getByRole('radio', { name: '30d' }));
    await waitFor(() => expect(settle).toBeDefined());
    expect(document.querySelector('[data-record-id="warning-message"]')).toBe(target);
    expect(h.reads).toHaveLength(count);
    expect(h.cancelled).not.toContain(active);
    await act(async () => settle(await readRow(SESSION, 30)));
    await screen.findByText('Measured in this range');
    expect(document.querySelector('[data-record-id="warning-message"]')).toBe(target);
    expect(h.reads).toHaveLength(count);
    expect(h.cancelled).not.toContain(active);
    expect(screen.getByText('Showing the original warning message.')).toBeTruthy();
  },
);

it.each(['claude', 'codex'] as const)(
  'starts the exact %s warning read for the new session and abandons the previous one',
  async (host) => {
    const h = codexSource('source_bytes', host);
    supplyWarnings(h, [messageWarning]);
    const sessions: string[] = [];
    h.data.sessionTranscriptPage = async (session, selection, id) => {
      sessions.push(session);
      h.reads.push({ selection, id });
      return warningPage(selection);
    };
    const next = `/sessions/${OTHER_SESSION}?warning=${encodeURIComponent(messageWarning.id)}`;
    mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`, next);
    await screen.findByText('Showing the original warning message.');
    const active = h.reads.at(-1)!.id;
    const count = h.reads.length;
    fireEvent.click(screen.getByRole('button', { name: `go ${next}` }));
    await waitFor(() => expect(h.reads.length).toBeGreaterThan(count));
    await screen.findByText('Showing the original warning message.');
    expect(sessions.at(-1)).toBe(OTHER_SESSION);
    expect(h.cancelled).toContain(active);
    expect(h.reads.at(-1)!.selection.kind).toBe('anchor');
  },
);

it('abandons the previous source warning promise when the data source is replaced', async () => {
  const first = codexSource('source_bytes', 'claude');
  const second = codexSource('source_bytes', 'claude');
  supplyWarnings(first, [messageWarning]);
  supplyWarnings(second, [messageWarning]);
  let finish!: (page: SessionPageStatus) => void;
  let pending!: SessionPageSelection;
  first.data.sessionTranscriptPage = async (_session, selection, id) => {
    first.reads.push({ selection, id });
    if (selection.kind === 'anchor') {
      pending = selection;
      return new Promise((resolve) => {
        finish = resolve;
      });
    }
    return warningPage(selection);
  };
  second.data.sessionTranscriptPage = async (_session, selection, id) => {
    second.reads.push({ selection, id });
    const page = warningPage(selection);
    if (page.state === 'loaded' && selection.kind === 'anchor')
      page.records[0]!.blocks = [{ kind: 'text', index: 0, text: 'New source original warning' }];
    return page;
  };
  const path = `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`;
  const view = mount(first.data, path);
  await waitFor(() => expect(finish).toBeDefined());
  const old = first.reads.at(-1)!.id;
  view.rerender(
    <ThemeProvider>
      <DataProvider source={second.data}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await screen.findByText('New source original warning');
  await act(() => finish(warningPage(pending)));
  expect(screen.queryByText('Original warning message warning-message')).toBeNull();
  expect(first.cancelled).toContain(old);
  expect(second.reads.filter((read) => read.selection.kind === 'anchor')).toHaveLength(1);
});

it.each(['claude', 'codex'] as const)(
  'loads one new %s anchor when a different warning names the same point',
  async (host) => {
    const h = codexSource('source_bytes', host);
    supplyWarnings(h, [messageWarning, { ...resultWarning, points_at: messageWarning.points_at }]);
    h.data.sessionTranscriptPage = async (_session, selection, id) => {
      h.reads.push({ selection, id });
      return warningPage(selection);
    };
    mount(h.data, `/sessions/${SESSION}?warning=${encodeURIComponent(messageWarning.id)}`);
    await screen.findByText('Showing the original warning message.');
    expect(h.reads.filter((read) => read.selection.kind === 'anchor')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: resultWarning.name }));
    await waitFor(() =>
      expect(h.reads.filter((read) => read.selection.kind === 'anchor')).toHaveLength(2),
    );
    await screen.findByText('Showing the original warning message.');
    expect(h.reads.at(-1)!.selection).toMatchObject({
      kind: 'anchor',
      record_uuid: 'warning-message',
      block: null,
      expected: 'message',
    });
  },
);
