## Git workflow

- **Always ask before committing, pushing, or opening a PR.** Show what will be committed/pushed and wait for explicit approval each time — approval for one commit or push does not carry over to the next.
- **New feature = new branch from up-to-date `main`.** Run `git checkout main && git pull`, then `git checkout -b <type>/<short-name>` (e.g. `feat/…`, `fix/…`, `chore/…`) before writing any code.
- **Never push to `main`.** Every push to `main` deploys to production on Railway, so all changes reach it through a pull request.
- **Commit messages follow Conventional Commits** (`feat(call): …`, `fix(turn): …`, `chore: …`). Nothing enforces it; keep to it anyway.

## Repository layout

One npm package (a Node server and a React client built with Vite), plus the native desktop client in `native/`.

- **`server/`** is the single Node service: WebSocket signaling and in-memory chat (`index.ts`, `rooms.ts`), TURN credentials (`turn.ts`), and serving the built client.
- **`web/src/`** is the client. `App.tsx` only picks the screen for the call phase. `setup/` is the pre-join screen, `call/` the call screen plus the logic behind it (`controller.ts` owns media, signaling and the peer connection; React only renders its state), `chat/` the chat panel, `ui/` small components used by more than one screen.
- **`shared/protocol.ts`** holds every message shape sent over `/ws`, used by both the server and the client. Add or change messages there, never as inline types on one side. Types and plain constants only: no Node or DOM imports. When a message changes, update `shared/protocol.fixtures.ts` and run `npm run fixtures`, so the native client's types are checked against it.
- **`native/`** is the Rust + Slint desktop client, a Cargo workspace. Its own conventions are in `native/AGENTS.md`; read them before working there.
- **`infra/`** is the coturn config and the deploy guide. `.railway/railway.ts` defines the Railway service.
- **Code used by one screen stays in its folder.** Move it to `ui/` only once a second screen needs it.
- **Split a file by what it does once it grows past ~300 lines of code.** Move a hook, a component or a group of helpers into its own file.

## Reachable from China

The call is between mainland China and Uzbekistan, so everything has to work in China without a VPN.

- **Never add Google or other services blocked in China:** STUN servers, fonts, CDNs, analytics, error trackers, or any script or asset loaded from a third-party host. Ship it in the bundle or serve it from our own server.
- STUN and TURN come only from our coturn (`turn-hk.danendz.com`). Don't add public STUN servers as a fallback.

## Security

The service is public: anyone can open the site and connect to `/ws` or `/api`. Write every change assuming the other side is hostile.

- **Treat everything from a client as untrusted.** Check the shape and type of every message before reading it (valid JSON can still be `null`), and check sizes and lengths. A throw inside a socket handler crashes the server and drops every call.
- **IDs are not secrets.** `clientId` and room IDs are visible to the other person. Anything that grants access (resuming a seat, `/ws/speech`) must check the member's secret `token`, made with `node:crypto` and never sent to the other member.
- **Put a limit on anything a client can repeat or grow.** New endpoints, messages, and in-memory state get a rate limit or cap (helpers in `server/limits.ts`), especially anything that spends paid API quota or TURN relay.
- **Keep the CSP strict.** Don't add `'unsafe-inline'`, `'unsafe-eval'` or third-party hosts to `setSecurityHeaders`, and don't render user text as HTML (`dangerouslySetInnerHTML`, `innerHTML`).
- **Secrets stay in Railway variables**, never in source, logs, client code, or messages sent to the browser.
- **Keep dependencies patched.** Run `npm audit --omit=dev` when adding or upgrading a package, and don't add a dependency for something a few lines of code can do.

## Before saying it's done

- There are no tests or linter yet. Run `npm run typecheck` and `npm run build`, and both must pass.
- Don't test the call by opening the same room in two tabs under `npm run dev`: the two tabs never connect in dev, so this proves nothing. Check UI changes on a single tab (setup screen, waiting screen), and leave real two-person call testing to the user.

## UI text

- **Don't add hint or description text that repeats what the control already shows.** Add helper text only when it tells the user something they can't see: why to do something, where to find it, or what a non-obvious state means (like the permission help on the setup screen).

## Roadmap

- **`docs/ROADMAP.md` lists planned work.** When you finish a feature listed there, mark it `[x]` (or `[~]` with what's left) in the same branch and PR that ships it. Add newly planned work there too, under the right phase with its **Where** (Web + Native or Native only). Overlay and notification features are native only: don't add browser workarounds for them to the web app.

## Tests

- **Only write tests that matter.** Cover real behavior, tricky logic, and regressions for bugs that actually happened.
- Don't write tests that restate the implementation, check trivial rendering or static markup, test third-party libraries or the framework, or exist only to raise coverage.
- If a change doesn't need a test, don't add one — say so instead.
