/**
 * The workflow, beat by beat. Every output line is the shape the CLI actually
 * prints; only the numbers come from an example repo.
 *
 * @example
 * import * as acts from '../scripts/acts';
 * acts.list[0].session // -> the `lane new` transcript
 */

/** what the disk panel is showing, in the order the workflow reaches it */
export let steps = [
	'seed',
	'lane',
	'write',
	'build',
	'drift',
	'fanout',
	'land',
	'gone',
] as const;

export type Step = (typeof steps)[number];

/** where the panel rests for a reader without JavaScript */
export let rest: Step = 'fanout';

export type Kind = 'cmd' | 'con' | 'out' | 'hi' | 'warn' | 'gone' | 'gap';

export type Line = {
	kind: Kind;
	text: string;
	/** the panel moves here once this line has finished printing */
	step?: Step;
	/** milliseconds to hold afterwards, so the change has time to be read */
	hold?: number;
};

export type Act = {
	/** the command this beat is about */
	label: string;
	/** the one thing the beat is claiming */
	claim: string;
	/** the directory the prompt is sitting in */
	cwd: string;
	/** where the panel sits when the beat starts */
	from: Step;
	session: Line[];
	/** what the panel is showing, in words */
	why: string;
};

export let list: Act[] = [
	{
		label: 'lane new',
		claim: 'Ignored files arrive by reference. The cache is already warm.',
		cwd: '~/repo',
		from: 'seed',
		session: [
			{ kind: 'cmd', text: 'lane new fix-login' },
			{ kind: 'out', text: '  reflink: yes (clonefile)' },
			{ kind: 'hi', text: '  12283 files cloned (1358.1 MiB, 0 copied)', step: 'lane', hold: 1500 },
			{ kind: 'out', text: '  ~/repo/.lane/trees/fix-login' },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: 'ls node_modules | wc -l' },
			{ kind: 'out', text: '4612' },
			{ kind: 'cmd', text: 'cargo build' },
			{ kind: 'out', text: '   Finished `dev` profile in 0.21s' },
		],
		why: '12,283 files, 1358.1 MiB shared, 0 copied. node_modules is there. cargo build finishes in 0.21s because target/ was never missing.',
	},
	{
		label: 'lane note',
		claim: 'Record what must stay true. Then edit.',
		cwd: '~/…/fix-login',
		from: 'lane',
		session: [
			{ kind: 'cmd', text: 'lane note add src/auth.rs -a "fn verify" \\' },
			{ kind: 'con', text: '"early return leaks token length"' },
			{ kind: 'out', text: 'noted -> src/auth.rs#fn verify' },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: '$EDITOR src/auth.rs', step: 'write', hold: 1600 },
			{ kind: 'cmd', text: 'git commit -am "make verify constant-time"' },
			{ kind: 'out', text: '[fix-login 4f1a92c] 1 file changed' },
		],
		why: 'The note is a new file. Editing src/auth.rs un-shares only that file. main still reads the original block.',
	},
	{
		label: 'lane check',
		claim: 'The rebuild stays in the lane. lane check flags the body.',
		cwd: '~/…/fix-login',
		from: 'write',
		session: [
			{ kind: 'cmd', text: 'cargo build' },
			{ kind: 'out', text: '   Compiling myapp v0.1.0' },
			{ kind: 'out', text: '    Finished `dev` profile in 10.2s', step: 'build', hold: 1600 },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: 'lane check' },
			{ kind: 'out', text: 'fresh              7' },
			{ kind: 'warn', text: 'content-changed    1', step: 'drift', hold: 1400 },
			{ kind: 'out', text: 'contract-changed  0' },
			{ kind: 'out', text: 'anchor-missing     0' },
		],
		why: '550 files rewritten — 106.5 MiB now owned by the lane. fn verify kept its signature and changed its body, so the note is content-changed.',
	},
	{
		label: 'lane ls',
		claim: 'Three worktrees. One set of extents.',
		cwd: '~/repo',
		from: 'drift',
		session: [
			{ kind: 'cmd', text: 'lane new agent-b && lane new agent-c' },
			{ kind: 'hi', text: '  2 lanes cloned (0 copied)', step: 'fanout', hold: 1600 },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: 'lane ls' },
			{ kind: 'out', text: 'fix-login  open    dirty  1 pending note(s)' },
			{ kind: 'out', text: 'agent-b    open    dirty  3 pending note(s)' },
			{ kind: 'out', text: 'agent-c    open    clean  0 pending note(s)' },
		],
		why: 'Each lane writes only the blocks it touches. Each note is its own file, so two agents can annotate the same function at once.',
	},
	{
		label: 'lane merge',
		claim: 'The worktree is removed. Commits and notes stay.',
		cwd: '~/…/fix-login',
		from: 'fanout',
		session: [
			{ kind: 'cmd', text: 'lane note replace 01M0B4KQTX7H3EZ8FE7S6BJ91N \\' },
			{ kind: 'con', text: '"constant-time; no early return"' },
			{ kind: 'out', text: 'replacement queued -> 01M0B4KQTX7H3EZ8FE7S6BJ91N src/auth.rs#fn verify' },
			{ kind: 'gap', text: '' },
			{ kind: 'cmd', text: 'lane merge   # in each lane, in any order' },
			{ kind: 'out', text: 'rebased onto main' },
			{ kind: 'out', text: 'memory: +1 new; checked 8' },
			{ kind: 'out', text: '  7 fresh, 1 content-changed' },
			{ kind: 'out', text: '  0 contract-changed, 0 anchor-missing' },
			{ kind: 'out', text: 'committed memory update' },
			{ kind: 'out', text: 'fast-forwarded main', step: 'land', hold: 1300 },
			{ kind: 'gone', text: 'removed lane fix-login', step: 'gone', hold: 900 },
		],
		why: 'The 114.9 MiB the lanes allocated is freed. The drifted note was rewritten; the old one is in .lane/attic/.',
	},
];
