import { defineConfig } from 'astro/config';
import { satteri } from '@astrojs/markdown-satteri';
import cloudflare from '@astrojs/cloudflare';
import { defineHastPlugin } from 'satteri';
import { split } from './src/scripts/tty';


/**
 * A fence gets the same treatment as the transcripts written by hand: the
 * prompt in the accent colour, the line you typed in the foreground one, and
 * the output left dim. `.block` is what the hand-written ones already use.
 */
let terminal = defineHastPlugin({
	name: 'terminal',
	element: {
		filter: ['pre'],
		visit(node) {
			let code = node.children.find((c: any) => c.tagName === 'code') as any;
			let text = code?.children?.[0];
			if (!code || text?.type !== 'text') return;

			return {
				...node,
				properties: { ...node.properties, className: ['block'] },
				children: [{ ...code, children: paint(text.value) }],
			};
		},
	},
});

/** one node per part, and the newline the split ate back between the lines */
function paint(source: string) {
	let out: any[] = [];
	for (let [i, line] of split(source).entries()) {
		if (i > 0) out.push({ type: 'text', value: '\n' });
		for (let part of line) {
			if (!part.cls) {
				out.push({ type: 'text', value: part.text });
			} else if (part.cls === 'b') {
				out.push({ type: 'element', tagName: 'b', properties: {}, children: [{ type: 'text', value: part.text }] });
			} else {
				out.push({
					type: 'element',
					tagName: 'span',
					properties: { className: [part.cls] },
					children: [{ type: 'text', value: part.text }],
				});
			}
		}
	}
	return out;
}

export default defineConfig({
	adapter: cloudflare({
		prerenderEnvironment: 'node',
	}),
	build: {
		inlineStylesheets: 'always'
	},
	markdown: {
		// The page has two colours and a terminal for a body font; a highlighter
		// would put six more in and none of them would mean anything here.
		syntaxHighlight: false,
		processor: satteri({
			hastPlugins: [terminal]
		}),
	},
});
