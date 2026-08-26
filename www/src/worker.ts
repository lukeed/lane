import { astro, FetchState } from 'astro/fetch';
import { cf } from '@astrojs/cloudflare/fetch';

type CloudflareEnv = Parameters<typeof cf>[1];
type CloudflareContext = Parameters<typeof cf>[2];

// Anchored, so a browser UA that merely contains "curl" still gets the page.
let CLI = /^(curl|wget|libfetch|httpie|python-requests|powershell)/i;

/** Run a request through Cloudflare's asset handling, then Astro's router. */
async function route(request: Request, env: CloudflareEnv, ctx: CloudflareContext) {
	let state = new FetchState(request);
	let asset = await cf(state, env, ctx);
	return asset ?? astro(state);
}

export default {
	async fetch(request: Request, env: CloudflareEnv, ctx: CloudflareContext) {
		let url = new URL(request.url);
		let agent = request.headers.get('user-agent') ?? '';

		if (url.pathname !== '/' || !CLI.test(agent)) {
			return route(request, env, ctx);
		}

		let script = await route(
			new Request(new URL('/install.sh', url), request),
			env,
			ctx,
		);

		return new Response(script.body, {
			status: script.status,
			headers: {
				'content-type': 'text/x-shellscript; charset=utf-8',
				'cache-control': 'public, max-age=300',
			},
		});
	},
};
