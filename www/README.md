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
