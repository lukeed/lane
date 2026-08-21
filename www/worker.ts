/**
 * Serves the install script at the site root when a command line client asks
 * for it, so `curl -fsSL https://lane.lukeed.com | sh` works while a browser at
 * the same address still gets the page. Everything else is a static asset.
 *
 * Only `/` and paths with no matching asset reach this Worker; see
 * run_worker_first in wrangler.jsonc. Verified: /favicon.svg and /install.sh
 * are served from the asset store without invoking it.
 */

type Env = {
	ASSETS: { fetch(request: Request): Promise<Response> };
};

// Anchored, so a browser UA that merely contains "curl" still gets the page.
let CLI = /^(curl|wget|libfetch|httpie|python-requests|powershell)/i;

export default {
	async fetch(request: Request, env: Env): Promise<Response> {
		let url = new URL(request.url);
		let agent = request.headers.get('user-agent') ?? '';

		if (url.pathname !== '/' || !CLI.test(agent)) {
			return env.ASSETS.fetch(request);
		}

		let script = await env.ASSETS.fetch(
			new Request(new URL('/install.sh', url), request),
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
