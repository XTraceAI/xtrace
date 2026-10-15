import { useState } from 'react';
import { matchRoutes, Outlet, useLocation, useNavigate } from 'react-router';
import { useData } from '../data/DataProvider';
import { Sidebar, type SidebarKey } from '../kit/Sidebar';
import { TopBar, type TimeRange } from '../kit/TopBar';
import { useTheme } from '../theme/ThemeProvider';
import { useAppInfo } from './useAppInfo';
import { pages } from './routes';
import { useShellShortcuts } from './useShellShortcuts';
import { useDashboardReport, useTokensByHost, type ShellOutletContext } from './dashboard/range';
import { hostTokenRows } from './dashboard/host-tokens';
import { windowLabel, zoneLabel } from './dashboard/present';
import '../styles/shell.css';

export function Shell() {
  const location = useLocation();
  const navigate = useNavigate();
  const { source, eventError } = useData();
  const { theme, toggle } = useTheme();
  const info = useAppInfo();
  const page = matchRoutes(pages, location)?.at(-1);
  const current = page?.route;
  const nativeMac = source.kind === 'native' && navigator.platform.startsWith('Mac');
  // One selected range for the Dashboard, its TopBar presets and the sidebar token totals.
  const [range, setRange] = useState<TimeRange>('7d');
  const tokens = useTokensByHost(range);
  const hosts = hostTokenRows(tokens.data);
  const outlet: ShellOutletContext = { range };
  const onDashboard = current?.path === '/dashboard';
  useShellShortcuts();
  function onNavigate(key: SidebarKey) {
    void navigate(`/${key}`);
  }
  return (
    <div className={`xt-app-shell${nativeMac ? ' native-mac' : ''}`}>
      <Sidebar
        activeKey={current?.active}
        onNavigate={onNavigate}
        leaderboardEnabled
        hosts={hosts}
        tokensCaption={
          source.kind === 'preview'
            ? 'Recorded tokens unavailable'
            : tokens.isError
              ? tokens.data === undefined
                ? 'Recorded tokens unavailable'
                : `Recorded tokens refresh failed · ${range}`
              : `Recorded tokens · ${range}`
        }
        tokensEmptyMessage={
          source.kind === 'preview' || tokens.isError
            ? null
            : tokens.data === undefined
              ? 'Reading recorded tokens…'
              : 'No host measurements'
        }
        onRetryTokens={
          source.kind !== 'preview' &&
          (tokens.isError || (tokens.data === undefined && tokens.isFetching))
            ? () => void tokens.refetch()
            : undefined
        }
        tokensRetryBusy={tokens.isFetching}
        surfaces={[]}
        listener={{ status: info.data?.listening === false ? 'off' : 'unknown' }}
        version={info.data?.version ?? '…'}
        theme={theme}
        onToggleTheme={toggle}
        onSettings={() => void navigate('/settings')}
        topInset={nativeMac ? 74 : 16}
        nativeChrome={nativeMac}
      />
      <div className="xt-shell-main">
        <TopBar
          crumb={current?.crumb ?? 'not-found'}
          subcrumb={location.pathname === '/rulebook/fires' ? 'fires' : page?.params.ruleId}
          {...(onDashboard ? { range, onRange: setRange } : { showRange: false as const })}
          nativeDrag={nativeMac}
          right={
            <>
              {onDashboard && <DashboardPeriod range={range} />}
              {info.data?.fixture && (
                <span className="xt-fixture-badge" role="status">
                  fixture {info.data.fixture}
                </span>
              )}
            </>
          }
        />
        <main className="xt-shell-outlet">
          {source.kind === 'preview' ? (
            <p className="xt-shell-notice">
              Browser preview · Open the desktop app to read local data.
            </p>
          ) : info.isPending ? (
            <p role="status" className="xt-shell-notice">
              Reading app data…
            </p>
          ) : info.isError ? (
            <p role="alert" className="xt-shell-notice">
              App data could not be loaded.{' '}
              <button type="button" className="refresh-button" onClick={() => void info.refetch()}>
                Refresh
              </button>
            </p>
          ) : null}
          {eventError && (
            <p role="alert" className="xt-shell-notice">
              Live updates are unavailable.
            </p>
          )}
          <Outlet context={outlet} />
        </main>
      </div>
    </div>
  );
}

/** The TopBar reads the same successful Dashboard report as the page. */
function DashboardPeriod({ range }: { range: TimeRange }) {
  const report = useDashboardReport(range);
  if (!report.isSuccess) return null;
  const period = report.data.window;
  return (
    <span
      className="xt-topbar-period"
      data-testid="report-period"
      title={`${windowLabel(period)} · ${zoneLabel(period)}`}
    >
      {windowLabel(period)}
      <span className="sr-only">, time zone {zoneLabel(period)}</span>
    </span>
  );
}
