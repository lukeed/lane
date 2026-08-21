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

- `https://lane.lukeed.com/install.sh` on any static host.
- `https://lane.lukeed.com` itself, when the request comes from curl or wget.
  That second one needs an edge function. `functions/_middleware.ts` implements
  it for Cloudflare Pages; point the Pages project at this directory and it is
  picked up with no configuration. On a host without edge functions the root
  serves the page as usual and only the `/install.sh` form works.

Binaries come from GitHub release assets, built by `.github/workflows/release.yml`
on a `v*` tag. The tarball is flat — one `lane` at the root — so the script and
`cargo binstall` resolve the same asset. Linux is musl, so one artifact per
architecture runs on any glibc vintage.
