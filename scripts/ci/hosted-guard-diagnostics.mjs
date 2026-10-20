import { spawnSync } from 'node:child_process';

// Diagnostic values never replace raw subprocess results used for acceptance.
const text = (value, limit) => {
  if (typeof value !== 'string') return { text: '', truncated: false };
  // An unquoted path can contain spaces or punctuation; never keep its fragments.
  const redacted = value
    .split(/\r\n|\r|\n/)
    .map((line) => (/(?:^|[^A-Za-z0-9])\/|\\/.test(line) ? '<path-bearing line redacted>' : line))
    .join('\n');
  return { text: redacted.slice(0, limit), truncated: redacted.length > limit };
};
const number = (value) => (Number.isFinite(value) ? value : null);
export function guardProcessDetails(label, timeoutMs, elapsedMs, result) {
  const error = result.error;
  return {
    label: text(label, 64),
    pid: number(result.pid),
    timeoutMs: number(timeoutMs),
    elapsedMs: number(elapsedMs),
    status: number(result.status),
    signal: result.signal == null ? null : text(result.signal, 64),
    error: error
      ? {
          name: text(error.name, 64),
          code: text(error.code, 64),
          errno: typeof error.errno === 'number' ? number(error.errno) : text(error.errno, 64),
          syscall: text(error.syscall, 64),
          message: text(error.message, 256),
        }
      : null,
    stdout: text(result.stdout, 512),
    stderr: text(result.stderr, 1024),
  };
}

// These are the only metadata commands; callers cannot supply a command or argv.
export function guardFailureMetadata(
  kind,
  caseDeadline,
  { reserveMs = 0, run = spawnSync, now = () => performance.now() } = {},
) {
  const commands =
    kind === 'curl'
      ? [['curl-version', '/usr/bin/curl', ['--version']]]
      : kind === 'swift'
        ? [
            ['swift-version', '/usr/bin/xcrun', ['swiftc', '--version']],
            ['sdk-version', '/usr/bin/xcrun', ['--show-sdk-version']],
            ['developer-selection', '/usr/bin/xcode-select', ['-p']],
          ]
        : [];
  const deadline = Math.min(now() + 1500, caseDeadline - reserveMs);
  const result = {
    kind: commands.length ? kind : 'unsupported',
    records: [],
    skipped: [],
    uncertainCompletion: false,
  };
  for (const [label, program, args] of commands) {
    const timeout = Math.floor(deadline - now());
    if (!Number.isFinite(timeout) || timeout <= 0) {
      result.skipped.push({ label, reason: 'diagnostic budget exhausted' });
      continue;
    }
    const began = now();
    let observed;
    try {
      observed = run(program, args, { encoding: 'utf8', timeout, maxBuffer: 4096 });
    } catch (error) {
      observed = { status: null, error };
    }
    result.uncertainCompletion ||= Boolean(
      observed.error || observed.signal || observed.status === null,
    );
    // Selection output is a path, not a diagnostic message; hide the complete path.
    const selection = typeof observed.stdout === 'string' ? observed.stdout.trim() : '';
    const developerKind =
      selection === '/Library/Developer/CommandLineTools'
        ? 'CommandLineTools'
        : /^\/Applications\/Xcode[^/]*\.app\/Contents\/Developer$/.test(selection)
          ? 'Xcode'
          : 'other';
    const selected =
      label === 'developer-selection'
        ? { ...observed, stdout: `<developer-selection:${developerKind}>` }
        : observed;
    result.records.push(guardProcessDetails(label, timeout, now() - began, selected));
  }
  return result;
}
