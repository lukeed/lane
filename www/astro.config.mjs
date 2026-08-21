// @ts-check
import { defineConfig } from 'astro/config';

export default defineConfig({
	build: { inlineStylesheets: 'always' },
	// The page has two colours and a terminal for a body font; a highlighter
	// would put six more in and none of them would mean anything here.
	markdown: { syntaxHighlight: false },
});
