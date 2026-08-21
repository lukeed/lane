/**
 * A transcript is two things at once: what you typed and what came back. The
 * prompt and the line you typed are lit, the output is not, which is the only
 * way to read one at a glance.
 *
 * @example
 * import { split } from '../scripts/tty';
 * split('$ lane init\nok')
 * // -> [[{ cls: 'p', text: '$ ' }, { cls: 'b', text: 'lane init' }], [{ cls: '', text: 'ok' }]]
 */

export type Part = {
	/** `p` the prompt, `c` its continuation, `b` what you typed, empty for output */
	cls: 'p' | 'c' | 'b' | '';
	text: string;
};

// a prompt sits at the start of a line, with one space after it. `#` is left
// out on purpose: in these transcripts it opens a comment, not a root shell.
let PROMPT = /^([$>])(?: (.*))?$/;

export function split(text: string): Part[][] {
	return text.split('\n').map((line) => {
		let hit = PROMPT.exec(line);
		if (!hit) return [{ cls: '', text: line }];

		let parts: Part[] = [{ cls: hit[1] === '>' ? 'c' : 'p', text: `${hit[1]} ` }];
		if (hit[2]) parts.push({ cls: 'b', text: hit[2] });
		return parts;
	});
}
