import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Sidebar, type SidebarKey } from '../src/kit/Sidebar';
import { BrandMark } from '../src/kit/BrandMark';
import { ThemeProvider, useTheme } from '../src/theme/ThemeProvider';
import '../src/index.css';
import type { AccountUsage } from '../src/data/generated/AccountUsage';
import type { AccountUsageIssue } from '../src/data/generated/AccountUsageIssue';

const accountUsage: AccountUsage = {
  claude: {
    state: 'available',
    issue: null,
    checked_at: 1790618400,
    windows: [
      {
        bucket_key: 'claude',
        window_key: 'seven_day',
        scope: 'all_models',
        name: 'Claude',
        window: 'Weekly',
        used_percent: 100,
        duration_minutes: 10080,
        resets_at: Math.floor(Date.now() / 1000) + 3600,
      },
      {
        bucket_key: 'claude',
        window_key: 'five_hour',
        scope: 'all_models',
        name: 'Claude',
        window: 'Session',
        used_percent: 0,
        duration_minutes: 300,
        resets_at: null,
      },
    ],
  },
  codex: {
    state: 'available',
    issue: null,
    checked_at: 1790618400,
    windows: [
      {
        bucket_key: 'codex',
        window_key: 'primary',
        scope: 'all_models',
        name: 'Codex',
        window: 'Weekly',
        used_percent: 26,
        duration_minutes: 10080,
        resets_at: Math.floor(Date.now() / 1000) + 7 * 86400,
      },
    ],
  },
};

// `?claude=<case>` and `?codex=<case>` change that provider's reading:
// `<issue>` shows it stale 18 hours after it was read, with that issue
// (`none` for no issue); `reset-<issue>` does too, with its weekly reset
// passed; `missing-<issue>` shows no reading, with that issue.
const search = new URLSearchParams(location.search);
for (const host of ['claude', 'codex'] as const) {
  const value = search.get(host);
  if (!value) continue;
  const issueOf = (name: string) => (name === 'none' ? null : (name as AccountUsageIssue));
  if (value.startsWith('missing-')) {
    accountUsage[host] = {
      state: 'unavailable',
      issue: issueOf(value.slice('missing-'.length)),
      checked_at: null,
      windows: [],
    };
    continue;
  }
  const resetPassed = value.startsWith('reset-');
  const now = Math.floor(Date.now() / 1000);
  accountUsage[host] = {
    ...accountUsage[host],
    state: 'stale',
    issue: issueOf(resetPassed ? value.slice('reset-'.length) : value),
    checked_at: now - 18 * 3600,
    windows: accountUsage[host].windows.map((window) =>
      resetPassed && window.duration_minutes === 10080
        ? { ...window, resets_at: now - 3600 }
        : window,
    ),
  };
}

function Fixture() {
  const { theme, toggle } = useTheme();
  const [activeKey, setActiveKey] = useState<SidebarKey>('dashboard');
  const [inset, setInset] = useState(false);
  return (
    <div style={{ display: 'flex', gap: 40 }}>
      <div
        style={{ width: 228, height: 900, background: 'var(--canvas)' }}
        data-testid="sidebar-preview"
      >
        <Sidebar
          activeKey={activeKey}
          onNavigate={setActiveKey}
          rulebookCount={5}
          accountUsage={accountUsage}
          listener={{ status: 'listening', port: 47421 }}
          version="0.1.0"
          updateLabel="up to date"
          onSettings={() => {}}
          theme={theme}
          onToggleTheme={toggle}
          surfaces={[
            { host: 'Codex', surface: 'cli', status: 'capturing' },
            { host: 'Codex', surface: 'desktop', status: 'not-capturing' },
            { host: 'Other host', surface: 'new-surface', status: 'unknown' },
          ]}
          topInset={inset ? 74 : 16}
        />
      </div>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 24, paddingTop: 32 }}>
        {[20, 26, 34].map((size) => (
          <BrandMark key={size} size={size as 20 | 26 | 34} />
        ))}
        <button className="refresh-button" onClick={() => setInset(!inset)}>
          Reserve native controls
        </button>
        <p data-testid="outside">Component preview</p>
      </div>
    </div>
  );
}
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <Fixture />
    </ThemeProvider>
  </StrictMode>,
);
