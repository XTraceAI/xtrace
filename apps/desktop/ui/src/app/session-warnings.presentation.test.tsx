import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter, useLocation } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { answer, dismissedAnswer, second, warning } from '../../e2e/session-warnings.data';
import { DataProvider } from '../data/DataProvider';
import { FixtureDataSource } from '../data/FixtureDataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import { SESSION_WARNINGS_TEXT } from '../data/generated/SessionWarningsText';
import { ThemeScope } from '../theme/ThemeProvider';
import {
  SessionWarningMarker,
  SessionWarningsDetail,
  WarningDot,
  WarningRow,
  beforeAxisLabel,
  localZone,
  useSessionWarning,
  warningHref,
} from './session-warnings';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function Where() {
  const location = useLocation();
  return <output data-testid="where">{location.pathname + location.search}</output>;
}

async function show(children: ReactNode, source = new FixtureDataSource(fixture as FixtureExport)) {
  const rendered = render(
    <DataProvider source={source}>
      <MemoryRouter>
        <ThemeScope theme="dark">{children}</ThemeScope>
        <Where />
      </MemoryRouter>
    </DataProvider>,
  );
  await screen.findByTestId('where');
  return rendered;
}

it('uses the native marker name and array cardinality, and omits unread or empty markers', async () => {
  await show(
    <>
      <SessionWarningMarker answer={undefined} to="/unused" />
      <SessionWarningMarker answer={{ state: 'missing' }} to="/unused" />
      <SessionWarningMarker answer={{ ...answer, count_label: null }} to="/unused" />
      <SessionWarningMarker answer={{ ...answer, warnings: [] }} to="/unused" />
      <SessionWarningMarker answer={answer} to={warningHref('s', '')} />
    </>,
  );
  const marker = screen.getByRole('link', { name: answer.count_label! });
  expect(screen.getAllByRole('link')).toHaveLength(1);
  expect(marker.textContent).toBe('2');
  expect(marker.tabIndex).toBe(0);
  fireEvent.focus(marker);
  const tip = await screen.findByTestId('warning-marker-tip');
  expect(tip.closest('[data-theme]')?.getAttribute('data-theme')).toBe('dark');
  expect(tip.querySelectorAll('.xt-warning-line')).toHaveLength(2);
  expect(tip.textContent).toContain(warning.reason);
  fireEvent.click(marker);
  expect(screen.getByTestId('where').textContent).toBe('/sessions/s?warning=');
});

it('encodes saved identifiers and carries only supported list state', () => {
  const carry = new URLSearchParams({
    q: 'name',
    host: 'codex',
    range: '7d',
    sort: 'recently_active',
    other: 'x',
  });
  expect(warningHref('s/1', 'correction:a#1', carry)).toBe(
    '/sessions/s%2F1?q=name&host=codex&range=7d&sort=recently_active&warning=correction%3Aa%231',
  );
  expect(beforeAxisLabel(1, 12)).toBe(
    SESSION_WARNINGS_TEXT.before_axis_one.replace('{hours}', '12'),
  );
  expect(beforeAxisLabel(2, 12)).toBe(
    SESSION_WARNINGS_TEXT.before_axis_many.replace('{count}', '2').replace('{hours}', '12'),
  );
});

it('names a linked dot, shows its supplied line on focus, and follows its link', async () => {
  await show(
    <WarningDot sessionId="s" warning={warning} left={0.5} to={warningHref('s', warning.id)} />,
  );
  const dot = screen.getByRole('link', { name: warning.name });
  expect(dot.style.left).toBe('50%');
  expect(dot.tabIndex).toBe(0);
  fireEvent.focus(dot);
  const tip = await screen.findByTestId('warning-tip');
  expect(tip.textContent).toBe(
    `${warning.day} ${warning.label} ${warning.reason}${SESSION_WARNINGS_TEXT.open_hint}`,
  );
  fireEvent.click(dot);
  expect(screen.getByTestId('where').textContent).toBe(warningHref('s', warning.id));
});

it('opens a button dot with its own button and supports tooltip suppression', async () => {
  const open = vi.fn();
  const parent = vi.fn();
  await show(
    <div onClick={parent}>
      <WarningDot sessionId="s" warning={warning} left={0} onOpen={open} suppressTooltip />
    </div>,
  );
  const dot = screen.getByRole('button', { name: warning.name });
  fireEvent.focus(dot);
  expect(screen.queryByTestId('warning-tip')).toBeNull();
  fireEvent.click(dot);
  expect(open).toHaveBeenCalledExactlyOnceWith(dot);
  expect(parent).toHaveBeenCalledTimes(1);
  expect(dot.getAttribute('aria-haspopup')).toBe('dialog');
});

it.each([
  { answer: undefined, pending: true, failed: false, text: SESSION_WARNINGS_TEXT.session_reading },
  { answer: undefined, pending: false, failed: true, text: SESSION_WARNINGS_TEXT.session_unread },
  {
    answer: undefined,
    pending: false,
    failed: false,
    text: SESSION_WARNINGS_TEXT.session_unavailable,
  },
  {
    answer: { state: 'missing' } as const,
    pending: false,
    failed: false,
    text: SESSION_WARNINGS_TEXT.session_missing,
  },
])('renders the supplied state notice: $text', async ({ text, ...props }) => {
  await show(<SessionWarningsDetail sessionId="s" {...props} />);
  const notice = screen.getByTestId('warnings-state');
  expect(notice.getAttribute('role')).toBe('status');
  expect(notice.textContent).toBe(text);
});

it('keeps supplied summary and incomplete checks without presenting empty warnings as healthy', async () => {
  await show(
    <SessionWarningsDetail
      sessionId="s"
      answer={{ ...answer, warnings: [], count_label: null }}
      pending={false}
      failed={false}
    />,
  );
  const summary = screen.getByTestId('warnings-summary');
  expect(summary.textContent).toBe(answer.summary);
  expect(summary.dataset.incomplete).toBe('true');
  expect(screen.queryByTestId('warnings-list')).toBeNull();
  const details = screen.getByText(answer.checks_summary).closest('details')!;
  fireEvent.click(within(details).getByText(answer.checks_summary));
  const checks = within(screen.getByTestId('warnings-checks')).getAllByRole('listitem');
  expect(checks.map((item) => item.textContent)).toEqual(
    answer.checks.map((check) => `${check.complete ? '✓' : '–'}${check.text}`),
  );
  expect(checks.map((item) => item.dataset.complete)).toEqual(['true', undefined]);
});

it('shows supplied returned/refused notes and keeps busy actions focusable without firing', async () => {
  const press = vi.fn();
  await show(
    <ol>
      <WarningRow warning={second} undo={false} highlighted busy refused onPress={press} />
    </ol>,
  );
  const row = screen.getByTestId('warning-item');
  expect(row.dataset.pointsAt).toBe('other');
  expect(row.dataset.highlighted).toBe('true');
  expect(screen.getByRole('alert').textContent).toBe(SESSION_WARNINGS_TEXT.dismiss_failed);
  expect(screen.getByTestId('warning-returned').textContent).toBe(second.returned);
  const button = screen.getByRole('button', { name: SESSION_WARNINGS_TEXT.dismiss });
  expect(button.tabIndex).toBe(0);
  expect(button.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(button);
  expect(press).not.toHaveBeenCalled();
  expect(document.getElementById(button.getAttribute('aria-describedby')!)?.textContent).toContain(
    second.reason,
  );
});

it('focuses the selected warning or summary and omits unsupported dismissal controls', async () => {
  await show(
    <SessionWarningsDetail
      sessionId="s"
      answer={answer}
      pending={false}
      failed={false}
      highlight={warning.id}
    />,
  );
  const row = screen
    .getAllByTestId('warning-item')
    .find((item) => item.dataset.warningId === warning.id)!;
  await waitFor(() => expect(document.activeElement).toBe(row));
  expect(row.dataset.pointsAt).toBe('synthetic#0');
  expect(row.dataset.highlighted).toBe('true');
  expect(screen.queryByRole('button')).toBeNull();
  cleanup();
  await show(
    <SessionWarningsDetail
      sessionId="s"
      answer={answer}
      pending={false}
      failed={false}
      highlight=""
    />,
  );
  expect(document.activeElement).toBe(screen.getByTestId('warnings-summary'));
});

function QueriedDetail() {
  const query = useSessionWarning('s', 'UTC');
  return (
    <SessionWarningsDetail
      sessionId="s"
      answer={query.data}
      pending={query.isPending}
      failed={query.isError}
      actions={() => <button>Extra action</button>}
      below={() => <p>Supplied below content</p>}
    />
  );
}

it('dismisses and undoes through the shared hook, focusing the returned control', async () => {
  const source = new FixtureDataSource(fixture as FixtureExport);
  let current = answer;
  vi.spyOn(source, 'sessionWarnings').mockImplementation(async () => ({
    timezone: 'UTC',
    sessions: [{ session_id: 's', answer: current }],
  }));
  const controls = {
    dismiss: vi.fn(async (session_id: string, warning_id: string) => {
      current = dismissedAnswer;
      return { session_id, warning_id, dismissed: true };
    }),
    restore: vi.fn(async (session_id: string, warning_id: string) => {
      current = answer;
      return { session_id, warning_id, dismissed: false };
    }),
  };
  Object.defineProperty(source, 'warningDismissals', { value: controls });
  await show(<QueriedDetail />, source);
  const items = await screen.findAllByTestId('warning-item');
  const row = items.find((item) => item.dataset.warningId === warning.id)!;
  expect(
    within(row)
      .getAllByRole('button')
      .map((button) => button.textContent),
  ).toEqual(['Extra action', SESSION_WARNINGS_TEXT.dismiss]);
  fireEvent.click(within(row).getByRole('button', { name: SESSION_WARNINGS_TEXT.dismiss }));
  const moved = await screen.findByTestId('dismissed-warning');
  const undo = within(moved).getByRole('button', { name: SESSION_WARNINGS_TEXT.restore });
  await waitFor(() => expect(document.activeElement).toBe(undo));
  expect(within(moved).queryByText('Extra action')).toBeNull();
  expect(within(moved).queryByText('Supplied below content')).toBeNull();
  expect(screen.getByTestId('warnings-summary').textContent).toBe(dismissedAnswer.summary);
  fireEvent.click(undo);
  await waitFor(() => expect(screen.queryByTestId('dismissed-warning')).toBeNull());
  await waitFor(() =>
    expect((document.activeElement as HTMLElement).dataset.action).toBe('dismiss'),
  );
  expect(controls.dismiss).toHaveBeenCalledExactlyOnceWith('s', warning.id);
  expect(controls.restore).toHaveBeenCalledExactlyOnceWith('s', warning.id);
});

it('uses the Mac timezone and preserves the installed UTC fallback', () => {
  const native = Intl.DateTimeFormat().resolvedOptions().timeZone;
  expect(localZone()).toBe(native || 'UTC');
  vi.spyOn(Intl, 'DateTimeFormat').mockImplementation(() => {
    throw new Error('unavailable');
  });
  expect(localZone()).toBe('UTC');
});
