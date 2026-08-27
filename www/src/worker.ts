import { handle } from '@astrojs/cloudflare/handler';

type CloudflareEnv = Parameters<typeof handle>[1];
type CloudflareContext = Parameters<typeof handle>[2];

// Anchored, so a browser UA that merely contains "curl" still gets the page.
let CLI = /^(curl|wget|libfetch|httpie|python-requests|powershell)/i;

export default {
	async fetch(request: Request, env: CloudflareEnv, ctx: CloudflareContext) {
		let url = new URL(request.url);
		let agent = request.headers.get('user-agent') ?? '';

		if (url.pathname === '/' && CLI.test(agent)) {
			let script = await handle(
				new Request(new URL('/install.sh', url), request),
				env,
				ctx,
			);
			if (script.ok) {
				return new Response(script.body, {
					status: 200,
					headers: {
						'content-type': 'text/x-shellscript; charset=utf-8',
						'cache-control': 'public, max-age=300',
					},
				});
			}
		}

		return handle(request, env, ctx);
	},
};
