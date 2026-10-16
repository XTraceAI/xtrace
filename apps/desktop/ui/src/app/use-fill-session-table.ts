import { useLayoutEffect, type RefObject } from 'react';

/** Fill the current table viewport, using drawn rows rather than page counts. */
export function useFillSessionTable(
  sectionRef: RefObject<HTMLElement | null>,
  canLoad: boolean,
  fetchMore: () => void,
) {
  // Measure after every commit: grouping, details, filters and loading can all
  // change the drawn content. ResizeObserver also covers window/section changes
  // and font/layout changes that do not cause a React render. Scrolling does
  // not change either height and never starts another page on its own.
  useLayoutEffect(() => {
    if (!canLoad) return;
    const scroll = sectionRef.current?.querySelector<HTMLElement>('.xt-table-scroll');
    const content = scroll?.querySelector<HTMLElement>('.xt-data-table');
    if (!scroll || !content) return;
    let started = false;
    const fill = () => {
      if (started || scroll.clientHeight <= 0) return;
      // An empty-state message is not a session row. Keep paging if grouping
      // hid every loaded row, even in a short window where that message fits.
      const drawn = content.querySelector('.xt-table-block')
        ? content
        : content.querySelector<HTMLElement>('.xt-table-head');
      if (!drawn || drawn.getBoundingClientRect().height >= scroll.clientHeight - 1) return;
      started = true;
      fetchMore();
    };
    fill();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(fill);
    observer.observe(scroll);
    observer.observe(content);
    return () => observer.disconnect();
  });
}
