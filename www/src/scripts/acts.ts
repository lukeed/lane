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
		claim: 'A second worktree in seconds, at no cost on disk.',
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
		why: 'Every path git ignores, at any depth, arrives as a window onto blocks main/ already holds. Nothing was copied, so the build cache is warm on the first command.',
	},
	{
		label: 'lane note',
		claim: 'Write down what must stay true, beside the code it constrains.',
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
		why: 'The edit un-shares that one file, and only inside the lane — main/ still reads the block it always read. Copy costs nothing until you write, and then it costs one file.',
	},
	{
		label: 'lane check',
		claim: 'Anchored to the symbol, flagged the moment the symbol moves.',
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
		why: 'The rebuild rewrote 550 files. Those blocks belong to the lane and leave with it. The note noticed too: fn verify kept its signature and changed its body, so lane flags it rather than guess whether it is still true.',
	},
	{
		label: 'lane ls',
		claim: 'One repo, three agents, nothing to lock and nothing to merge.',
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
		why: 'Three trees, three warm caches, one set of extents. Each agent writes only the blocks it touches, and every note is a new file — so two of them can annotate the same function in the same second.',
	},
	{
		label: 'lane merge',
		claim: 'The worktree goes. What it learned lands on main.',
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
		why: 'The commits land, the worktrees close, and every block they allocated is freed. The note does not close with them: the drifted one was rewritten against the code that shipped, the old one moved to .lane/attic/, and the next lane opens with both in reach.',
	},
];
