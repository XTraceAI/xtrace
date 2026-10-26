import { afterEach, expect, it, vi } from 'vitest';
import { revealInTranscript } from './transcript-reveal';

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

it('centres a short target and reveals the beginning of a tall target inside its owner', () => {
  const owner = document.createElement('div');
  const target = document.createElement('div');
  owner.append(target);
  document.body.append(owner);
  owner.style.overflowY = 'auto';
  owner.scrollIntoView = vi.fn();
  Object.defineProperty(owner, 'clientHeight', { value: 200 });
  owner.scrollTop = 100;
  owner.getBoundingClientRect = () => ({ top: 50, bottom: 250 }) as DOMRect;
  target.getBoundingClientRect = () => ({ top: 350, height: 40 }) as DOMRect;
  owner.scrollTo = vi.fn();
  target.scrollIntoView = vi.fn();
  revealInTranscript(owner, target);
  expect(owner.scrollTo).toHaveBeenLastCalledWith({ top: 320, behavior: 'smooth' });
  target.getBoundingClientRect = () => ({ top: 350, height: 400 }) as DOMRect;
  revealInTranscript(owner, target);
  expect(owner.scrollTo).toHaveBeenLastCalledWith({ top: 400, behavior: 'smooth' });
  expect(target.scrollIntoView).not.toHaveBeenCalled();
  owner.getBoundingClientRect = () => ({ top: 900, bottom: 1100 }) as DOMRect;
  revealInTranscript(owner, target);
  expect(owner.scrollIntoView).toHaveBeenCalledWith({ block: 'nearest', behavior: 'smooth' });
});

it('keeps the existing page reveal for an ordinary unbounded transcript', () => {
  const owner = document.createElement('div');
  const target = document.createElement('div');
  owner.append(target);
  document.body.append(owner);
  target.scrollIntoView = vi.fn();
  revealInTranscript(owner, target);
  expect(target.scrollIntoView).toHaveBeenCalledWith({ block: 'center', behavior: 'smooth' });
  revealInTranscript(null, target);
  expect(target.scrollIntoView).toHaveBeenCalledTimes(2);
});
