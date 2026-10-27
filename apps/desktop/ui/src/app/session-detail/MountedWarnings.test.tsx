import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { filters, session, title, warningSource } from '../../../e2e/mounted-session-warnings.data';
import { answer, dismissedAnswer, second, warning } from '../../../e2e/session-warnings.data';
import { DataProvider } from '../../data/DataProvider';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
beforeEach(() => {
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      constructor(private callback: IntersectionObserverCallback) {}
      observe(target: Element) {
        queueMicrotask(() =>
          this.callback(
            [{ target, isIntersecting: true }] as IntersectionObserverEntry[],
            this as unknown as IntersectionObserver,
          ),
        );
      }
      unobserve() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function Where() {
  return <output data-testid="where">{useLocation().search}</output>;
}
async function mount(path: string) {
  const source = warningSource();
  const transcript = vi.spyOn(source, 'sessionTranscript');
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
          <Where />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await screen.findByTestId('where');
  return { source, transcript };
}

it('opens a list marker at the warning summary and retains filters in Back', async () => {
  const { transcript } = await mount(`/sessions?${filters}`);
  const marker = await screen.findByRole('link', { name: answer.count_label! });
  expect(marker.getAttribute('href')).toBe(`/sessions/${session.id}?${filters}&warning=`);
  fireEvent.click(marker);
  const summary = await screen.findByTestId('warnings-summary');
  expect(summary.textContent).toBe(answer.summary);
  await waitFor(() => expect(document.activeElement).toBe(summary));
  expect(screen.queryByRole('list', { name: 'Selected warning' })).toBeNull();
  expect(screen.getByRole('link', { name: '← All sessions' }).getAttribute('href')).toBe(
    `/sessions?${filters}`,
  );
  expect(transcript).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole('button', { name: warning.name }));
  const selected = await screen.findByRole('list', { name: 'Selected warning' });
  expect(selected.textContent).toContain(warning.reason);
  await waitFor(() =>
    expect(document.activeElement).toBe(within(selected).getByTestId('warning-item')),
  );
  expect(screen.getByTestId('where').textContent).toContain('warning=correction%3Asynthetic%230');
  expect(transcript).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole('link', { name: '← All sessions' }));
  await screen.findByRole('link', { name: new RegExp(`Open session ${title}`) });
  expect(screen.getByRole('searchbox', { name: 'Search sessions' }).getAttribute('value')).toBe(
    title,
  );
});

it('arrives at an exact warning, switches selection, dismisses and undoes without reopening the transcript', async () => {
  const { transcript } = await mount(
    `/sessions/${session.id}?${filters}&warning=${encodeURIComponent(warning.id)}`,
  );
  const selected = await screen.findByRole('list', { name: 'Selected warning' });
  await waitFor(() =>
    expect(document.activeElement).toBe(within(selected).getByTestId('warning-item')),
  );
  expect(screen.getByRole('button', { name: warning.name }).getAttribute('aria-current')).toBe(
    'true',
  );
  fireEvent.click(within(selected).getByRole('button', { name: 'Dismiss' }));
  await waitFor(() =>
    expect(screen.getByTestId('warnings-summary').textContent).toBe(dismissedAnswer.summary),
  );
  const undo = within(selected).getByRole('button', { name: 'Undo' });
  await waitFor(() => expect(document.activeElement).toBe(undo));
  expect(screen.getByTestId('where').textContent).toContain(
    `warning=${encodeURIComponent(warning.id)}`,
  );
  fireEvent.click(undo);
  await waitFor(() =>
    expect(document.activeElement).toBe(within(selected).getByRole('button', { name: 'Dismiss' })),
  );
  fireEvent.click(screen.getByRole('button', { name: second.name }));
  expect(selected.textContent).toContain(second.reason);
  expect(screen.getByTestId('where').textContent).toContain(
    `warning=${encodeURIComponent(second.id)}`,
  );
  expect(transcript).toHaveBeenCalledTimes(1);
});

it('keeps a missing warning selection at the summary without inventing a selected warning', async () => {
  await mount(`/sessions/${session.id}?warning=not-present`);
  const summary = await screen.findByTestId('warnings-summary');
  await waitFor(() => expect(document.activeElement).toBe(summary));
  expect(screen.queryByRole('list', { name: 'Selected warning' })).toBeNull();
});
