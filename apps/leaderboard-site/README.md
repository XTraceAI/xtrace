# XTrace Leaderboard site

This folder is the public leaderboard website and its JSON API.
People join from the XTrace app with a nickname and an email.
The app then sends their 7-day leverage, and the site lists it next to their nickname.
The page shows only the nickname and the number. Emails are never shown or returned.

The exact API is in [API.md](API.md).

## How it is built

- Plain Vercel Functions in `api/`, one file per endpoint. They only forward to `src/app.ts`.
- `vercel.json` sends `/` to `api/page.ts`, which renders the page.
- The logic in `src/` takes a small `query(text, params)` function.
  Production passes Neon. Tests and the local server pass PGlite, a real Postgres that runs in the process.
- The rule for who is listed (a number updated in the last 48 hours) lives only in `src/listing.ts`.
  The page, `/api/leaderboard` and `/api/me` all use it.
- Nickname and email rules live only in `src/validate.ts`.
- The database table is in `schema.sql`.

## Run the tests

From the repository root:

```sh
pnpm install
pnpm --dir apps/leaderboard-site test
pnpm --dir apps/leaderboard-site typecheck
```

The root `pnpm check` runs both as well.

## Run it on your computer

```sh
pnpm --dir apps/leaderboard-site dev:local -- --port 8787
```

This serves the same `api/` files at `http://127.0.0.1:8787`.
It needs no network and no Vercel login.
The database is in memory and is gone when you stop the server.
To keep it, add `--data <folder>`.

To point a test copy of the app at it, set `XTRACE_LEADERBOARD_URL=http://127.0.0.1:8787`.

## Deploy (first time)

1. In Vercel, create a new project from this GitHub repository.
2. Set **Root Directory** to `apps/leaderboard-site`. Leave the framework as **Other**.
   Leave the build command empty. Leave the install command as it is:
   `vercel.json` sets it.
   That install command uses the repository's pnpm version (10.33.0) and skips the desktop app's packages.
   It turns off the strict Node version check. Without that, Vercel's install fails
   whenever its Node 22 version differs from the exact one the repository asks for.
3. Open the project's **Storage** tab, add **Neon** from the Marketplace, and connect it to the project.
   This sets `DATABASE_URL` for the project.
4. Create the table. Copy `DATABASE_URL` from the project's environment variables, then run from the repository root:

   ```sh
   DATABASE_URL='<the Neon connection string>' pnpm --dir apps/leaderboard-site db:migrate
   ```

   It prints `Schema applied: 1 statement(s).` Running it again is safe.

5. Deploy. Open the deployment's address and check that the page says "No one is listed yet."
6. In the project's **Domains** settings, add `leaderboard.xtrace.ai`, then add the DNS record Vercel shows you.

## Later changes

- Code changes deploy when they reach the branch Vercel watches.
- If `schema.sql` changes, run step 4 again before deploying the code that needs it.
- To remove someone by hand, delete their row from `members` in the Neon console.
