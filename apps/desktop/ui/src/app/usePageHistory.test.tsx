import { StrictMode } from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { HashRouter, MemoryRouter, Navigate, useLocation, useNavigate } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import { TopBar } from '../kit/TopBar';
import { usePageHistory } from './usePageHistory';
import { useShellShortcuts } from './useShellShortcuts';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.history.replaceState(null, '', '/');
});

function Controls({ startup = false }: { startup?: boolean }) {
  const history = usePageHistory();
  useShellShortcuts(history);
  const navigate = useNavigate();
  const location = useLocation();
  return (
    <>
      <TopBar crumb={location.pathname} showRange={false} history={history} nativeDrag />
      <output aria-label="Location">
        {location.pathname}
        {location.search}
      </output>
      <button onClick={() => void navigate('/sessions')}>Open sessions</button>
      <button onClick={() => void navigate('/scores')}>Open scores</button>
      <button onClick={() => void navigate('?q=edited', { replace: true })}>Replace filters</button>
      <button onClick={() => void navigate(-1)}>Browser back</button>
      <button onClick={() => void navigate(1)}>Browser forward</button>
      <main>
        <input type="search" aria-label="Search test" />
      </main>
      {startup && location.pathname === '/' && <Navigate to="/dashboard" replace />}
    </>
  );
}
const click = (name: string) => fireEvent.click(screen.getByRole('button', { name }));
const disabled = (name: string) =>
  (screen.getByRole('button', { name }) as HTMLButtonElement).disabled;
const address = () => screen.getByLabelText('Location').textContent;
function mount(entries = ['/dashboard'], initialIndex?: number) {
  return render(
    <StrictMode>
      <MemoryRouter initialEntries={entries} initialIndex={initialIndex}>
        <Controls />
      </MemoryRouter>
    </StrictMode>,
  );
}

it('starts bounded, follows browser Back/Forward, and clears the forward branch on a new page', () => {
  mount();
  expect(disabled('Back')).toBe(true);
  expect(disabled('Forward')).toBe(true);
  click('Open sessions');
  click('Open scores');
  expect(disabled('Back')).toBe(false);
  expect(disabled('Forward')).toBe(true);
  click('Browser back');
  expect(address()).toBe('/sessions');
  expect(disabled('Forward')).toBe(false);
  click('Back');
  expect(address()).toBe('/dashboard');
  expect(disabled('Back')).toBe(true);
  click('Forward');
  expect(address()).toBe('/sessions');
  click('Back');
  click('Open scores');
  expect(disabled('Forward')).toBe(true);
  click('Back');
  expect(address()).toBe('/dashboard');
});

it('replaces filters without adding a step or losing the forward branch', () => {
  mount();
  click('Open sessions');
  click('Open scores');
  click('Back');
  click('Replace filters');
  expect(address()).toBe('/sessions?q=edited');
  expect(disabled('Back')).toBe(false);
  expect(disabled('Forward')).toBe(false);
  click('Forward');
  expect(address()).toBe('/scores');
  click('Back');
  expect(address()).toBe('/sessions?q=edited');
  click('Back');
  expect(address()).toBe('/dashboard');
  expect(disabled('Back')).toBe(true);
});

it('bounds startup redirects and a fresh mount even when older browser entries exist', () => {
  const first = render(
    <StrictMode>
      <MemoryRouter>
        <Controls startup />
      </MemoryRouter>
    </StrictMode>,
  );
  expect(address()).toBe('/dashboard');
  expect(disabled('Back')).toBe(true);
  click('Open sessions');
  first.unmount();
  mount(['/outside', '/dashboard', '/sessions'], 2);
  expect(disabled('Back')).toBe(true);
  click('Browser back');
  expect(address()).toBe('/dashboard');
  expect(disabled('Back')).toBe(true);
  expect(disabled('Forward')).toBe(true);
});

it('shares shortcut bounds with buttons, preserves existing shortcuts, and ignores extra modifiers', () => {
  mount();
  const key = (key: string, extra = {}) =>
    fireEvent.keyDown(window, { key, metaKey: true, ...extra });
  expect(key('[')).toBe(false);
  expect(address()).toBe('/dashboard');
  click('Open sessions');
  for (const extra of [
    { altKey: true },
    { ctrlKey: true },
    { shiftKey: true },
    { isComposing: true },
    { metaKey: false },
  ]) {
    key('[', extra);
    expect(address()).toBe('/sessions');
  }
  key('k');
  expect(document.activeElement).toBe(screen.getByRole('searchbox'));
  key('[');
  expect(address()).toBe('/dashboard');
  key(']');
  expect(address()).toBe('/sessions');
  key(',');
  expect(address()).toBe('/settings');
  expect(
    screen.getByRole('group', { name: 'Page history' }).getAttribute('data-tauri-drag-region'),
  ).toBe('false');
});

it('allows only one pending HashRouter POP, including mixed clicks and shortcuts', async () => {
  window.history.replaceState(null, '', '/#/dashboard');
  render(
    <StrictMode>
      <HashRouter>
        <Controls />
      </HashRouter>
    </StrictMode>,
  );
  click('Open sessions');
  const go = vi.spyOn(window.history, 'go').mockImplementation(() => {});
  click('Back');
  click('Back');
  fireEvent.keyDown(window, { key: '[', metaKey: true });
  expect(go).toHaveBeenCalledExactlyOnceWith(-1);
  go.mockRestore();
  click('Browser back');
  await waitFor(() => expect(address()).toBe('/dashboard'));
  expect(disabled('Back')).toBe(true);
  click('Forward');
  await waitFor(() => expect(address()).toBe('/sessions'));
  expect(disabled('Forward')).toBe(true);
});
