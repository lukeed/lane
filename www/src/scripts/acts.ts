/**
 * The four beats of one lane, in order. Every output line here is the shape the
 * CLI actually prints; only the numbers are from an example repo.
 *
 * @example
 * import * as acts from '../scripts/acts';
 * acts.list[0].session // -> the `lane new` transcript
 */

export type Kind = 'cmd' | 'con' | 'out' | 'hi' | 'warn' | 'gone' | 'gap';

export type Line = {
	kind: Kind;
	text: string;
};

export type Act = {
	/** what the rail calls this beat */
	label: string;
	/** the transcript; `cmd` and `con` lines are typed one character at a time */
	session: Line[];
	/** milliseconds to hold on the finished frame before advancing */
	dwell: number;
};

export let list: Act[] = [
	{
		label: 'lane new',
		dwell: 5200,
		session: [
			{ kind: 'cmd', text: 'lane new fix-login' },
			{ kind: 'out', text: '  reflink: yes (reflink available)' },
			{ kind: 'hi', text: '  12283 files cloned (1358.1 MiB shared, 0 copied)' },
			{ kind: 'out', text: '  ~/repo/.lane/trees/fix-login' },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: 'ls node_modules | wc -l' },
			{ kind: 'out', text: '4612' },
			{ kind: 'cmd', text: 'cargo build' },
			{ kind: 'out', text: '   Finished `dev` profile in 0.21s' },
		],
	},
	{
		label: 'lane note',
		dwell: 5600,
		session: [
			{ kind: 'cmd', text: 'lane note -p src/auth.rs -a "fn verify" \\' },
			{ kind: 'con', text: '"early return leaks token length"' },
			{ kind: 'out', text: 'noted -> src/auth.rs#fn verify' },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: 'git commit -am "make verify constant-time"' },
			{ kind: 'out', text: '[fix-login 4f1a92c] 1 file changed' },
		],
	},
	{
		label: 'the code moves',
		dwell: 6400,
		session: [
			{ kind: 'cmd', text: '$EDITOR src/auth.rs   # rewrite the compare' },
			{ kind: 'cmd', text: 'cargo build' },
			{ kind: 'out', text: '   Compiling myapp v0.1.0' },
			{ kind: 'out', text: '    Finished `dev` profile in 10.2s' },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: 'lane check' },
			{ kind: 'out', text: 'fresh              7' },
			{ kind: 'warn', text: 'body-drift         1' },
			{ kind: 'out', text: 'signature-changed  0' },
			{ kind: 'out', text: 'anchor-missing     0' },
		],
	},
	{
		label: 'lane done',
		dwell: 7000,
		session: [
			{ kind: 'cmd', text: 'lane done' },
			{ kind: 'out', text: 'rebased onto main' },
			{ kind: 'out', text: 'memory: +1 new; checked 8: 7 fresh, 1 body-drift,' },
			{ kind: 'out', text: '        0 signature-changed, 0 missing' },
			{ kind: 'out', text: '  reviewed 1 drifted note(s) via anthropic(haiku)' },
			{ kind: 'hi', text: '  superseded    src/auth.rs#fn verify -> 01M0B9MFVB' },
			{ kind: 'out', text: 'committed memory update' },
			{ kind: 'out', text: 'fast-forwarded main' },
			{ kind: 'gone', text: 'removed lane fix-login' },
		],
	},
];
