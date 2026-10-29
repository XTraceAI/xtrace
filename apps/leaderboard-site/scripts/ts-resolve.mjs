// Lets Node run this package's TypeScript directly (tests, local server, migrate).
// The sources import siblings as `./name.js`, the form Vercel's compiler expects;
// on disk only `./name.ts` exists, so a missing relative `.js` falls back to `.ts`.
import { registerHooks } from 'node:module';

registerHooks({
  resolve(specifier, context, nextResolve) {
    try {
      return nextResolve(specifier, context);
    } catch (error) {
      const relative = specifier.startsWith('./') || specifier.startsWith('../');
      if (error?.code === 'ERR_MODULE_NOT_FOUND' && relative && specifier.endsWith('.js'))
        return nextResolve(`${specifier.slice(0, -3)}.ts`, context);
      throw error;
    }
  },
});
