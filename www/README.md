# www

The lane site. One page, static, no framework.

```bash
bun install
bun run dev
bun run build      # -> dist/
```

`src/scripts/acts.ts` holds the four beats of the animated terminal. Every
output line there is the shape the CLI actually prints; only the numbers come
from an example repo. Change the CLI's output and change that file with it.

The extent grid in the right pane is one element for all four beats. The driver
sets `data-act` on `#disk` and CSS does the rest, so the blocks never re-mount:
they arrive shared, un-share as the lane writes, and land or are freed at
`lane done`. The 106.5 MiB is measured — one source edit plus the incremental
rebuild it triggered rewrote 550 files in `target/`.

`typescript` is pinned to 6.x on purpose: `astro check` uses the programmatic
compiler API, which TypeScript 7's native compiler does not expose yet.

## The install script

`public/install.sh` is served two ways:

- `https://lane.lukeed.com/install.sh`, a plain static asset.
- `https://lane.lukeed.com` itself, when the request comes from curl or wget.

The second one is why this deploys as a Worker rather than as static hosting:
`worker.ts` reads the user agent on `/` and answers with the script or the page.
`run_worker_first` in `wrangler.jsonc` scopes that to `/` alone, so every other
asset is served without invoking the Worker.

```bash
bun run serve     # astro build && wrangler dev — the real runtime, locally
bun run deploy    # astro build && wrangler deploy
```

`wrangler.jsonc` claims `lane.lukeed.com` as a custom domain. That needs the zone
on the same Cloudflare account; drop the `routes` block to deploy to a
`workers.dev` subdomain instead.

Binaries come from GitHub release assets, built by `.github/workflows/release.yml`
on a `v*` tag. The tarball is flat — one `lane` at the root — so the script and
`cargo binstall` resolve the same asset. Linux is musl, so one artifact per
architecture runs on any glibc vintage.
