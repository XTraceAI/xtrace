import assert from 'node:assert/strict';
import { rm } from 'node:fs/promises';
import { basename } from 'node:path';
import { guardProcessDetails, guardFailureMetadata } from './hosted-guard-diagnostics.mjs';

export function compilerControls({
  guardProcessDetails: details = guardProcessDetails,
  guardFailureMetadata: collectMetadata = guardFailureMetadata,
  console: logger = console,
  rm: remove = rm,
} = {}) {
  function recordCompiler(owner, label, timeout, elapsed, result) {
    owner.hasResult = true;
    owner.retain ||= Boolean(result.error || result.signal || result.status === null);
    let message = 'Compiler result diagnostics unavailable';
    try {
      message = JSON.stringify(details(label, timeout, elapsed, result));
      logger.log(message);
    } catch {
      /* Logging cannot change the status assertion. */
    }
    assert.equal(result.status, 0, message);
  }
  function diagnoseCompiler(owner, deadline) {
    if (!owner.hasResult || owner.metadataCollected) return;
    owner.metadataCollected = true;
    try {
      const metadata = collectMetadata('swift', deadline);
      owner.retain ||= metadata.uncertainCompletion;
      logger.log(JSON.stringify({ compilerMetadata: metadata }));
    } catch {
      owner.retain = true;
      try {
        logger.log('Compiler metadata unavailable; descendant cleanup unconfirmed');
      } catch {
        /* Keep the primary error. */
      }
    }
  }
  async function cleanupCompiler(directory, owner) {
    if (owner.retain) {
      try {
        logger.log(
          JSON.stringify({
            temporaryProofRetained: true,
            directory: basename(directory),
            compilerDescendants: 'unconfirmed',
            runnerTeardownVerified: false,
          }),
        );
      } catch {
        /* Keep the primary error. */
      }
    } else await remove(directory, { recursive: true, force: true });
  }
  return { recordCompiler, diagnoseCompiler, cleanupCompiler };
}

export function reportProxyFailure(
  error,
  proxy,
  deadline,
  { guardFailureMetadata: collectMetadata = guardFailureMetadata, console: logger = console } = {},
) {
  try {
    const reserveMs =
      proxy && proxy.child.exitCode === null && proxy.child.signalCode === null ? 8000 : 0;
    logger.log(JSON.stringify({ curlMetadata: collectMetadata('curl', deadline, { reserveMs }) }));
  } catch {
    /* Preserve the qualification error even when metadata fails. */
  }
  throw error;
}
