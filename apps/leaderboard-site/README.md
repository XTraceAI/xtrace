# XTrace Leaderboard site

This folder is the public leaderboard website and its JSON API.
People join from the XTrace app with a nickname and an email.
The site emails them a 6-digit code, and they type it into the app to finish joining.
The nickname is checked only at that second step. If it is taken by then, the app can ask for
another nickname and send the same code again.
The first step answers the same way for every valid email, so it never shows whether an email has joined.
The app then sends their 7-day leverage, and the site lists it next to their nickname.
Joining again with the same email (for example on a new Mac) signs you back in to the same entry.
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
- Joining in two steps (code by email, then verify) is in `src/joining.ts`.
- Email goes through a small `Mailer` in `src/mail.ts`: Resend over plain `fetch` in production,
  a fake in tests, and a printout on the local server.
- Every rate limit is in `src/limits.ts`. Counts live in Postgres, so all Vercel instances share them.
  IP addresses and emails in the counters are stored only as keyed hashes (HMAC with `IP_HASH_SECRET`).
  IPv6 addresses are limited by their /64 prefix.
- The database tables are in `schema.sql`.

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
It never sends email. When someone joins, it prints `code for <email>: <code>` in its own window.
Type that code into the app.
The rate limits work as in production, and every request counts as coming from `127.0.0.1`.
So after 5 joins in one hour, restart the server (with the in-memory database) to join again.
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
4. Set up email with Resend. MemHub already sends from the domain `emails.xtrace.ai` through Resend,
   so you can reuse that Resend account and its verified domain.
   Sharing the account means a flood of joins would share MemHub's sending limits and reputation.
   So create a separate API key for the leaderboard (same verified domain), with sending access only.
   If it is ever abused, you can revoke that key without touching MemHub. Then, in the Vercel project's
   **Settings → Environment Variables**, add:
   - `RESEND_API_KEY`: that key.
   - `LEADERBOARD_FROM_EMAIL` (optional): the sender. It defaults to
     `XTrace Leaderboard <noreply@emails.xtrace.ai>`.
   - `IP_HASH_SECRET`: a random value of 32 bytes or more. One way to make it:
     `openssl rand -base64 48`. Keep it secret. Changing it later only resets the rate-limit counts.

   Without `RESEND_API_KEY`, joining answers "email unavailable".
   Without `IP_HASH_SECRET`, everything except the page and `/api/leaderboard` answers "server misconfigured".

5. Create the tables. Copy `DATABASE_URL` from the project's environment variables, then run from the repository root:

   ```sh
   DATABASE_URL='<the Neon connection string>' pnpm --dir apps/leaderboard-site db:migrate
   ```

   It prints `Schema applied: 5 statement(s).` Running it again is safe.

6. Deploy. Open the deployment's address and check that the page says "No one is listed yet."
7. In the project's **Domains** settings, add `leaderboard.xtrace.ai`, then add the DNS record Vercel shows you.
8. Add an outer layer against floods. In the project's **Firewall**, add a rate-limit rule for paths
   starting with `/api/` (for example 60 requests per minute per IP), and a stricter one for `/api/join`
   (for example 10 per minute per IP). The site has its own limits too;
   these rules stop a flood before it reaches the functions and the database.
9. In the Vercel team's **Settings → Billing**, set a spend limit, so a flood cannot run up a bill.
   In Resend, check the plan's sending limit too.

## Known limits

- Anyone who knows an email address can use up its 10 joins for the day: from several IPs at once, or from one IP over about 4 hours (3 per hour).
  That inbox then cannot start a new join until the next UTC midnight.
  A code that was already sent and has not expired still works, and existing members keep their entry.
- The email limits count the inbox, not the exact address: `name+tag@…` counts as `name@…`,
  and for Gmail the dots do not count either.

## Later changes

- Code changes deploy when they reach the branch Vercel watches.
- If `schema.sql` changes, run step 5 again before deploying the code that needs it.
- To remove someone by hand, delete their row from `members` in the Neon console.
