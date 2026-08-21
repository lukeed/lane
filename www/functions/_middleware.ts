/**
 * Serves the install script at the site root when a command line client asks
 * for it, so `curl -fsSL https://lane.lukeed.com | sh` works while a browser at
 * the same address still gets the page.
 *
 * Cloudflare Pages picks this up from `functions/`. On a host without edge
 * functions the script is still reachable at /install.sh.
 */

type Context = {
	request: Request;
	next: (request?: Request) => Promise<Response>;
};

let CLI = /^(curl|wget|libfetch|httpie|python-requests|powershell)/i;

export async function onRequest(context: Context): Promise<Response> {
	let url = new URL(context.request.url);
	let agent = context.request.headers.get('user-agent') ?? '';

	if (url.pathname !== '/' || !CLI.test(agent)) return context.next();

	let script = await context.next(
		new Request(new URL('/install.sh', url), context.request),
	);

	return new Response(script.body, {
		status: script.status,
		headers: {
			'content-type': 'text/x-shellscript; charset=utf-8',
			'cache-control': 'public, max-age=300',
		},
	});
}
