/**
 * Every subcommand lane's parser accepts, in the order it defines them, with
 * the flags that belong to each. Read off crates/lane/src, not off --help.
 *
 * @example
 * import { commands } from '../data/commands';
 * commands[0].name // -> 'init'
 */

export type Option = {
	/** as you type it, short form first when there is one */
	flag: string;
	/** the value it takes, or null when it is a switch */
	arg: string | null;
	about: string;
};

export type Command = {
	/** `install hooks` for a leaf of a parent command */
	name: string;
	/** the whole line, flags included */
	usage: string;
	summary: string;
	options: Option[];
	/** a real transcript: what you type, then what it prints */
	example: string;
};

export let commands: Command[] = [
	{
		name: "init",
		usage: "lane init",
		summary: "scaffolds .lane/, adds the union merge rule, writes the AGENTS.md protocol, and probes reflink support.",
		options: [],
		example: "$ lane init\nwrote /w/proj/AGENTS.md protocol\ninitialized .lane/, union merge rules, AGENTS.md protocol\nreflink on this filesystem: yes (reflink available)",
	},
	{
		name: "new",
		usage: "lane new <name> [--base <ref>] [--dirty] [--cd]",
		summary: "creates a branch and a copy-on-write worktree under .lane/trees/, cloning everything git ignores by reference.",
		options: [
			{ flag: "--base", arg: "<ref>", about: "ref the lane branches from; defaults to the repo's trunk." },
			{ flag: "--dirty", arg: null, about: "carry uncommitted work into the lane." },
			{ flag: "--cd", arg: null, about: "print the path last on stdout, for the shell function." },
		],
		example: "$ lane new fix-login\n  reflink: yes (reflink available)\n  1284 files cloned (612.4 MiB shared, 0 copied)\n/w/proj/.lane/trees/fix-login",
	},
	{
		name: "ls",
		usage: "lane ls",
		summary: "lists every lane with its branch, dirty state, and pending note count.",
		options: [],
		example: "$ lane ls\nagent-a    agent-a    clean   3 pending note(s)\nagent-b    agent-b    dirty   1 pending note(s)",
	},
	{
		name: "path",
		usage: "lane path <name>",
		summary: "prints the absolute path of a lane's worktree.",
		options: [],
		example: "$ lane path fix-login\n/w/proj/.lane/trees/fix-login",
	},
	{
		name: "note",
		usage: "lane note -p <path> [-a <anchor>] [--supersedes <id>] <text>",
		summary: "records a pending finding against a file and an anchor.",
		options: [
			{ flag: "-p, --path", arg: "<path>", about: "file the note is about; required, resolved to a repo-relative path." },
			{ flag: "-a, --anchor", arg: "<anchor>", about: "what in the file the note is about; defaults to @file." },
			{ flag: "--supersedes", arg: "<id>", about: "retire this live note when the pending replacement is promoted." },
		],
		example: "$ lane note -p src/auth.rs -a \"fn verify\" \\\n    \"must stay constant-time; early return leaks length\"\nnoted -> src/auth.rs#fn verify",
	},
	{
		name: "why",
		usage: "lane why [<path>] [-a <anchor>]",
		summary: "prints the notes for a path, each with a freshness mark. A pure read; it changes nothing.",
		options: [
			{ flag: "-a, --anchor", arg: "<anchor>", about: "show only the notes on this anchor." },
		],
		example: "$ lane why src/auth.rs\n\nsrc/auth.rs#fn verify\n    must stay constant-time; early return leaks length\n      01M0B9MBYB · fix-login · 2026-08-14\n  ~ callers rely on false-on-expiry   [body-drift]\n      01M0B4KQTX · rate-limit · 2026-07-30",
	},
	{
		name: "holds",
		usage: "lane holds <id>",
		summary: "re-vouches for a drifted note and refreshes its fingerprint. Takes any unambiguous prefix of an id, and refuses one that matches two notes.",
		options: [],
		example: "$ lane check\n...\n~ 01M0B4KQTX  src/auth.rs#fn verify\n\n$ lane holds 01M0B4KQTX\nholds -> 01M0B4KQTX7H3EZ8FE7S6BJ91N",
	},
	{
		name: "check",
		usage: "lane check [--json]",
		summary: "counts the notes in each staleness tier, lists the ones that are not fresh, and exits 1 if any anchor is missing.",
		options: [
			{ flag: "--json", arg: null, about: "print each note as JSON, with current spans on non-fresh work items." },
		],
		example: "$ lane check\nfresh              7\nbody-drift         1\nsignature-changed  0\nanchor-missing     0\nunverifiable       0\n\n~ 01M0B4KQTX  src/auth.rs#fn verify",
	},
	{
		name: "audit",
		usage: "lane audit [--base <ref>] [--json]",
		summary: "promotes pending notes, re-anchors across renames, reports drift, ranks what is left, and evicts whatever is over budget.",
		options: [
			{ flag: "--base", arg: "<ref>", about: "ref to diff against for touched paths and rename detection; empty by default." },
			{ flag: "--max-notes", arg: "<n>", about: "keep at most this many notes per path and anchor; default 5." },
			{ flag: "--max-chars", arg: "<n>", about: "keep at most this many characters per path and anchor; default 1200." },
			{ flag: "--json", arg: null, about: "print the outcome as JSON instead of the report." },
		],
		example: "$ lane audit\nmemory: +2 new; checked 8: 7 fresh, 1 body-drift,\n        0 signature-changed, 0 missing\n  drift   src/sync.rs#fn reconnect",
	},
	{
		name: "done",
		usage: "lane done [--trunk <ref>] [--keep] [--squash]",
		summary: "rebases the lane onto trunk, audits memory, commits it, fast-forwards trunk, and removes the lane.",
		options: [
			{ flag: "--trunk", arg: "<ref>", about: "branch to rebase onto and advance; defaults to the repo's trunk." },
			{ flag: "--keep", arg: null, about: "keep the lane worktree and branch after landing." },
			{ flag: "--squash", arg: null, about: "squash the lane's commits into one landing commit." },
			{ flag: "--cd", arg: null, about: "print the main root path last on stdout, for the shell function." },
			{ flag: "--max-notes", arg: "<n>", about: "keep at most this many notes per path and anchor; default 5." },
			{ flag: "--max-chars", arg: "<n>", about: "keep at most this many characters per path and anchor; default 1200." },
		],
		example: "$ lane done\nrebased onto main\nmemory: +2 new; checked 8: 7 fresh, 1 body-drift,\n        0 signature-changed, 0 missing\ncommitted memory update\nfast-forwarded main\nremoved lane fix-login",
	},
	{
		name: "rm",
		usage: "lane rm <name> [--force]",
		summary: "discards a lane without landing it, keeping the branch if it holds commits trunk does not have.",
		options: [
			{ flag: "--force", arg: null, about: "discard commits trunk does not have." },
		],
		example: "$ lane rm keeper\nremoved lane keeper; kept branch keeper, it has\ncommits main does not\n  git worktree add <path> keeper   to get back\n  lane rm keeper --force           to discard",
	},
	{
		name: "install hooks",
		usage: "lane install hooks",
		summary: "installs the post-commit and prepare-commit-msg hooks that capture Why: trailers.",
		options: [],
		example: "$ lane install hooks\ninstalled .git/hooks/post-commit\ninstalled .git/hooks/prepare-commit-msg",
	},
	{
		name: "install skill",
		usage: "lane install skill",
		summary: "writes the lane skill, which teaches an agent the daily loop, to .agents/skills/lane/SKILL.md.",
		options: [],
		example: "$ lane install skill\ninstalled /w/proj/.agents/skills/lane/SKILL.md",
	},
	{
		name: "uninstall hooks",
		usage: "lane uninstall hooks",
		summary: "removes lane's block from the post-commit and prepare-commit-msg hooks.",
		options: [],
		example: "$ lane uninstall hooks\nremoved lane block from .git/hooks/post-commit\nremoved lane block from .git/hooks/prepare-commit-msg",
	},
	{
		name: "uninstall skill",
		usage: "lane uninstall skill",
		summary: "deletes the installed skill file at .agents/skills/lane/SKILL.md.",
		options: [],
		example: "$ lane uninstall skill\nremoved /w/proj/.agents/skills/lane/SKILL.md",
	},
	{
		name: "shellenv",
		usage: "lane shellenv",
		summary: "prints the shell function that makes new, cd and done leave you in the right directory.",
		options: [],
		example: "$ eval \"$(lane shellenv)\"\n$ lane shellenv\nlane() {\n  case \"$1\" in\n    new)  shift; p=$(command lane new --cd \"$@\") ...\n    cd)   shift; p=$(command lane path \"$@\") ...\n    done) shift; p=$(command lane done --cd \"$@\") ...\n    *)    command lane \"$@\" ;;\n  esac\n}",
	},
	{
		name: "capture",
		usage: "lane capture <rev>",
		summary: "reads Why: trailers from one commit and queues them as pending notes. Hidden, and run for you by the post-commit hook.",
		options: [],
		example: "$ lane capture HEAD\ncaptured -> src/auth.rs#fn verify",
	},
];

/** accepted everywhere, so they are listed once */
export let globals: Option[] = [
	{ flag: "-h, --help", arg: null, about: "print help; accepted by the root command and by every subcommand." },
	{ flag: "-V, --version", arg: null, about: "print the version; the root command only." },
];
