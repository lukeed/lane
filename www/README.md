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
