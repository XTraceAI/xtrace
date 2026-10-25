import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const sources = {
  'hosted-launch': {
    'LAUNCH CONTEXT POLICY': 'hosted-launch-launch-context-policy.swift',
    'TELEMETRY ENVIRONMENT POLICY': 'hosted-launch-telemetry-environment-policy.swift',
    'TRANSITION POLICY': 'hosted-launch-transition-policy.swift',
    'STOP POLICY': 'hosted-launch-stop-policy.swift',
    'PROCESS QUERY': 'hosted-launch-process-query.swift',
    RECEIPT: 'hosted-launch-receipt.swift',
    'PROCESS START': 'hosted-launch-process-start.swift',
  },
  'hosted-ax': {
    'UPDATE UI POLICY': 'hosted-ax-update-ui-policy.swift',
    'TELEMETRY UI POLICY': 'hosted-ax-telemetry-ui-policy.swift',
  },
  'hosted-updater-cancel': {
    'CANCEL POLICY': 'hosted-updater-cancel-cancel-policy.swift',
    'REFRESH WAIT': 'hosted-updater-cancel-refresh-wait.swift',
  },
};
// Swift permits top-level executable statements in a single input file. Assemble
// fixed policy files with their existing entrypoint before invoking swiftc.
export async function hostedSwift(name, directory = dirname(fileURLToPath(import.meta.url))) {
  if (!Object.hasOwn(sources, name)) throw new Error('Unknown hosted Swift entrypoint');
  let source = await readFile(join(directory, `${name}.swift`), 'utf8');
  for (const filename of Object.values(sources[name])) {
    const marker = `// __${filename}__`;
    if (source.split(marker).length !== 2)
      throw new Error(`Expected one Swift policy input marker: ${filename}`);
    const policy = await readFile(join(directory, filename), 'utf8');
    source = source.replace(marker, () => policy);
  }
  if (/\/\/ __[^\n]+\.swift__/.test(source)) throw new Error('Unexpanded Swift policy input');
  return source;
}
export async function writeHostedSwift(directory, name, destination) {
  await writeFile(destination, await hostedSwift(name, directory));
  return destination;
}
export async function swiftPolicy(name) {
  const filename = Object.values(sources)
    .map((group) => group[name])
    .find(Boolean);
  if (!filename) throw new Error('Unknown hosted Swift policy');
  return readFile(new URL(filename, import.meta.url), 'utf8');
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [name, destination] = process.argv.slice(2);
  await writeHostedSwift(dirname(fileURLToPath(import.meta.url)), name, destination);
}
