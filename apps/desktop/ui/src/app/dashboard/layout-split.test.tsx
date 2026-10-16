import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { clampFirst, readSplit, roundShare, splitStorageKey, writeSplit } from './layout-split';
import { SplitHandle } from './SplitHandle';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  localStorage.clear();
});

it('reads the default layout from empty, malformed or out-of-range storage', () => {
  expect(readSplit()).toEqual({ rows: null, columns: null });
  for (const raw of ['{', 'null', '7', '{"rows": 1, "columns": 0}', '{"rows": "0.5"}']) {
    localStorage.setItem(splitStorageKey, raw);
    expect(readSplit()).toEqual({ rows: null, columns: null });
  }
  localStorage.setItem(splitStorageKey, '{"rows": 0.4, "columns": 2}');
  expect(readSplit()).toEqual({ rows: 0.4, columns: null });
});

it('stores one axis beside the other and forgets the key when both are default', () => {
  expect(writeSplit('rows', 0.6)).toBe(true);
  expect(writeSplit('columns', 0.55)).toBe(true);
  expect(JSON.parse(localStorage.getItem(splitStorageKey)!)).toEqual({ rows: 0.6, columns: 0.55 });
  writeSplit('rows', null);
  expect(readSplit()).toEqual({ rows: null, columns: 0.55 });
  writeSplit('columns', null);
  expect(localStorage.getItem(splitStorageKey)).toBeNull();
});

it('treats storage that throws as the default layout and a lost write', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('denied');
  });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('denied');
  });
  expect(readSplit()).toEqual({ rows: null, columns: null });
  expect(writeSplit('rows', 0.5)).toBe(false);
});

it('keeps the first pane within both minimums', () => {
  expect(clampFirst(100, 600, 200, 150)).toBe(200);
  expect(clampFirst(500, 600, 200, 150)).toBe(450);
  expect(clampFirst(300, 600, 200, 150)).toBe(300);
  // Too little room for both minimums: the first keeps its own.
  expect(clampFirst(300, 300, 200, 150)).toBe(200);
  expect(roundShare(0.123456)).toBe(0.1235);
});

function Split() {
  return (
    <div data-testid="container">
      <section>first</section>
      <SplitHandle axis="rows" label="Resize the rows" names={['Top', 'Bottom']} />
      <section>second</section>
    </div>
  );
}

it('draws a stored share on its container and resets it from a double-click or Enter', () => {
  localStorage.setItem(splitStorageKey, '{"rows": 0.6}');
  render(<Split />);
  const container = screen.getByTestId('container');
  const handle = screen.getByRole('separator', { name: 'Resize the rows' });
  expect(handle.getAttribute('aria-orientation')).toBe('horizontal');
  expect(handle.tabIndex).toBe(0);
  expect(container.hasAttribute('data-split-rows')).toBe(true);
  expect(container.style.getPropertyValue('--rows-first')).not.toBe('');
  fireEvent.doubleClick(handle);
  expect(container.hasAttribute('data-split-rows')).toBe(false);
  expect(container.style.getPropertyValue('--rows-first')).toBe('');
  expect(localStorage.getItem(splitStorageKey)).toBeNull();

  localStorage.setItem(splitStorageKey, '{"rows": 0.6, "columns": 0.5}');
  cleanup();
  render(<Split />);
  fireEvent.keyDown(screen.getByRole('separator'), { key: 'Enter' });
  expect(screen.getByTestId('container').hasAttribute('data-split-rows')).toBe(false);
  expect(readSplit()).toEqual({ rows: null, columns: 0.5 });
});

it('leaves the default layout alone while nothing has been moved', () => {
  render(<Split />);
  expect(screen.getByTestId('container').hasAttribute('data-split-rows')).toBe(false);
  expect(screen.getByRole('separator').getAttribute('aria-valuetext')).toMatch(/, default layout$/);
  expect(localStorage.getItem(splitStorageKey)).toBeNull();
});

/** Model the browser's queued frames, observers and scroll clamping during a probe. */
function scrollingSplit() {
  vi.useFakeTimers();
  const frames = new Map<number, FrameRequestCallback>();
  let nextFrame = 0;
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    frames.set(++nextFrame, callback);
    return nextFrame;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
  let resized = () => {};
  let changed = () => {};
  const disconnect = vi.fn();
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(callback: () => void) {
        resized = callback;
      }
      observe() {}
      disconnect = disconnect;
    },
  );
  vi.stubGlobal(
    'MutationObserver',
    class {
      constructor(callback: () => void) {
        changed = callback;
      }
      observe() {}
      disconnect = disconnect;
    },
  );
  const view = render(
    <div data-testid="container">
      <section data-testid="first">first</section>
      <SplitHandle axis="rows" label="Resize the rows" names={['Top', 'Bottom']} />
      <section data-testid="second">
        <div>
          <div data-testid="scroll" tabIndex={0} />
        </div>
      </section>
    </div>,
  );
  const container = screen.getByTestId('container');
  const scroll = screen.getByTestId('scroll');
  const handle = screen.getByRole('separator');
  let top = 0;
  let probes = 0;
  let firstIsSmall = false;
  Object.defineProperty(scroll, 'scrollTop', {
    configurable: true,
    get: () => top,
    set: (value: number) => {
      top = value;
    },
  });
  for (const pane of ['first', 'second']) {
    vi.spyOn(screen.getByTestId(pane), 'getBoundingClientRect').mockImplementation(() => {
      const weight = Number(container.style.getPropertyValue(`--rows-${pane}`));
      const probing = container.hasAttribute('data-split-rows') && weight < 1;
      if (pane === 'first') {
        if (probing && !firstIsSmall) {
          probes += 1;
          top = 0; // A temporary large pane clamps its nested scroll position.
        }
        firstIsSmall = probing;
      }
      return { height: probing ? (pane === 'first' ? 70 : 80) : 200 } as DOMRect;
    });
  }
  const frame = () =>
    act(() => {
      const queued = [...frames.values()];
      frames.clear();
      queued.forEach((callback) => callback(performance.now()));
    });
  const advance = (ms: number) => act(() => vi.advanceTimersByTime(ms));
  const move = (position: number, left = 0) => {
    scroll.scrollTop = position;
    scroll.scrollLeft = left;
    fireEvent.scroll(scroll);
  };
  return {
    ...view,
    scroll,
    handle,
    frame,
    advance,
    move,
    resized: () => act(resized),
    changed: () => act(changed),
    probes: () => probes,
    frames,
    disconnect,
  };
}

it('keeps pending bounds out of nested scrolling and measures them after movement stops', () => {
  const split = scrollingSplit();
  split.move(13);
  split.frame(); // Even the initial already-queued frame must not probe mid-scroll.
  expect(split.probes()).toBe(0);
  expect(split.handle.getAttribute('aria-valuenow')).toBe('50');
  split.changed();
  split.resized();
  split.advance(150); // Bounds are due and their frame is now queued.
  split.move(40);
  split.frame();
  expect(split.probes()).toBe(0);
  split.advance(100);
  split.move(5, 8);
  split.advance(149);
  split.frame();
  expect(split.probes()).toBe(0);
  split.advance(1);
  split.frame();
  expect(split.probes()).toBe(1);
  expect(split.handle.getAttribute('aria-valuemin')).toBe('18');
  expect(split.handle.getAttribute('aria-valuemax')).toBe('80');
  expect(split.scroll.scrollTop).toBe(5);
  expect(split.scroll.scrollLeft).toBe(8);
  fireEvent.scroll(split.scroll); // Queued notification of the restored position.
  split.advance(300);
  split.frame();
  expect(split.probes()).toBe(1);
});

it('still measures immediately for splitter focus and restores a genuinely clamped list', () => {
  const split = scrollingSplit();
  split.move(35);
  split.frame();
  expect(split.probes()).toBe(0);
  fireEvent.focus(split.handle);
  expect(split.probes()).toBe(1);
  expect(split.scroll.scrollTop).toBe(35);
  fireEvent.scroll(split.scroll);
  split.advance(150);
  split.frame();
  expect(split.probes()).toBe(2); // The earlier pending measurement still completes.
  fireEvent.scroll(split.scroll);
  split.advance(300);
  split.frame();
  expect(split.probes()).toBe(2);
});

it('removes pending scroll work and observers when the splitter unmounts', () => {
  const split = scrollingSplit();
  split.move(13);
  split.unmount();
  expect(split.frames.size).toBe(0);
  expect(split.disconnect).toHaveBeenCalledTimes(2);
  fireEvent.scroll(split.scroll);
  expect(vi.getTimerCount()).toBe(0);
  split.advance(300);
  split.frame();
  expect(split.probes()).toBe(0);
});
