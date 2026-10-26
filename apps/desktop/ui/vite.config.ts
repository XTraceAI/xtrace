import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';

const timezoneTests = 'src/kit/AccountUsageWidget.test.tsx';

export default defineConfig({
  plugins: [tailwindcss()],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 5173, strictPort: true },
  // Keep bundled fonts compatible with the native font-src 'self' policy.
  build: { target: 'safari17', assetsInlineLimit: 0 },
  test: {
    environment: 'jsdom',
    pool: 'vmThreads',
    isolate: true,
    projects: [
      {
        extends: true,
        test: {
          name: 'unit',
          include: ['src/**/*.test.{ts,tsx}'],
          exclude: [timezoneTests],
        },
      },
      {
        extends: true,
        test: {
          name: 'timezone',
          // Node applies process.env.TZ changes only in a real process.
          pool: 'forks',
          include: [timezoneTests],
        },
      },
    ],
  },
});
