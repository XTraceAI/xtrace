import { spawnSync } from 'node:child_process';
export function reservedCommand(budget, proxyStopReserve, systemEnvironment, spawn = spawnSync) {
  const run = (command, args, timeout = 30_000, env = systemEnvironment) => {
    const remaining = budget.remaining(timeout + proxyStopReserve()) - proxyStopReserve();
    if (remaining <= 0) throw new Error('Command would consume reserved proxy cleanup time');
    return spawn(command, args, {
      encoding: 'utf8',
      env,
      timeout: remaining,
      killSignal: 'SIGKILL',
      maxBuffer: 4 * 1024 * 1024,
    });
  };
  return run;
}
