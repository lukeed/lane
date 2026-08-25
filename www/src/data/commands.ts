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
		usage: "lane ls [--json]",
		summary: "lists every lane's landing state, dirty state, and pending note count.",
		options: [
			{ flag: "--json", arg: null, about: "print the lane inventory as machine-readable JSON." },
		],
		example: '$ lane ls --json\n[{"name":"agent-a","path":"/w/proj/.lane/trees/agent-a","branch":"agent-a","state":"open","dirty":false,"pending_notes":3}]',
	},
	{
		name: "path",
		usage: "lane path <name>",
		summary: "prints the absolute path of a lane's worktree.",
		options: [],
		example: "$ lane path fix-login\n/w/proj/.lane/trees/fix-login",
	},
	{
		name: "anchors",
		usage: "lane anchors [--json] <path>",
		summary: "lists every canonical anchor in source order with its inclusive line range; @file is always present.",
		options: [
			{ flag: "--json", arg: null, about: "print exact anchor, start, and end fields as machine-readable JSON." },
		],
		example: "$ lane anchors src/auth.rs\n@file\t1-8\nfn verify\t1-4\nfn refresh\t6-8",
	},
	{
		name: "note add",
		usage: "lane note add [-a <anchor>] <path> [text]",
		summary: "records a pending finding; supplied text never prompts, while omitted text opts into the anchor selector and one-line prompt.",
		options: [
			{ flag: "-a, --anchor", arg: "<anchor>", about: "what in the file the note is about; defaults to @file." },
		],
		example: "$ lane note add src/auth.rs -a \"fn verify\" \\\n    \"must stay constant-time; early return leaks length\"\nnoted -> src/auth.rs#fn verify",
	},
	{
		name: "note edit",
		usage: "lane note edit <id>",
		summary: "shows a live note and interactively chooses one lifecycle action: confirm, replace text, retire, or toggle pinning.",
		options: [],
		example: "$ lane note edit 01M0B4KQTX\nEditing 01M0B4KQTX7H3EZ8FE7S6BJ91N\n  src/auth.rs#fn verify\n  status: content-changed\n  must stay constant-time\nAction:\n  1. confirm — still true\n  2. replace — change the text\n  3. retire — no longer applies\n  4. pin — protect from eviction\nChoose [1-4]: 1\nconfirmed -> 01M0B4KQTX7H3EZ8FE7S6BJ91N",
	},
	{
		name: "note replace",
		usage: "lane note replace [-p <path>] [-a <anchor>] <id> [text]",
		summary: "queues a successor that inherits the live predecessor's path and anchor; promotion retires the predecessor.",
		options: [
			{ flag: "-p, --path", arg: "<path>", about: "override the predecessor's path." },
			{ flag: "-a, --anchor", arg: "<anchor>", about: "override the predecessor's anchor." },
		],
		example: "$ lane note replace 01M0B4KQTX \"constant-time; no early return\"\nreplacement queued -> 01M0B4KQTX7H3EZ8FE7S6BJ91N src/auth.rs#fn verify",
	},
	{
		name: "note confirm",
		usage: "lane note confirm <id>",
		summary: "re-vouches for a drifted live note and refreshes its fingerprint.",
		options: [],
		example: "$ lane note confirm 01M0B4KQTX\nconfirmed -> 01M0B4KQTX7H3EZ8FE7S6BJ91N",
	},
	{
		name: "note retire",
		usage: "lane note retire <id>",
		summary: "moves a live note to the attic without rewriting its bytes.",
		options: [],
		example: "$ lane note retire 01M0B4KQTX\nretired -> 01M0B4KQTX7H3EZ8FE7S6BJ91N",
	},
	{
		name: "note restore",
		usage: "lane note restore <id>",
		summary: "moves a retired note back to live memory without rewriting its bytes.",
		options: [],
		example: "$ lane note restore 01M0B4KQTX\nrestored -> 01M0B4KQTX7H3EZ8FE7S6BJ91N",
	},
	{
		name: "note pin",
		usage: "lane note pin <id>",
		summary: "protects a live note from missing-anchor and budget eviction.",
		options: [],
		example: "$ lane note pin 01M0B4KQTX\npinned -> 01M0B4KQTX7H3EZ8FE7S6BJ91N",
	},
	{
		name: "note unpin",
		usage: "lane note unpin <id>",
		summary: "removes eviction protection from a live note.",
		options: [],
		example: "$ lane note unpin 01M0B4KQTX\nunpinned -> 01M0B4KQTX7H3EZ8FE7S6BJ91N",
	},
	{
		name: "why",
		usage: "lane why [<path>] [-a <anchor>] [--json]",
		summary: "prints the notes for a path in compact form, or for every path beneath it when the path is a directory. A pure read; it changes nothing.",
		options: [
			{ flag: "-a, --anchor", arg: "<anchor>", about: "show only the notes on this anchor." },
			{ flag: "--json", arg: null, about: "print full note fields as machine-readable JSON." },
		],
		example: '$ lane why src/auth.rs --json\n[{"id":"01M0B4KQTX7H3EZ8FE7S6BJ91N","path":"src/auth.rs","anchor":"fn verify","created":"2026-08-19T00:00:00Z","note":"must stay constant-time"}]',
	},
	{
		name: "check",
		usage: "lane check [--json]",
		summary: "counts the notes in each staleness tier, lists the ones that are not fresh under their tier, and exits 1 if any anchor is missing.",
		options: [
			{ flag: "--json", arg: null, about: "print each note as JSON, with current spans on non-fresh work items." },
		],
		example: "$ lane check\nfresh              7\ncontent-changed         1\ncontract-changed  0\nanchor-missing     0\nunverifiable       0\n\n[content-changed]\n~ 01M0B4KQTX  src/auth.rs#fn verify",
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
		example: "$ lane audit\nmemory: +2 new; checked 8: 7 fresh, 1 content-changed,\n        0 contract-changed, 0 missing\n  drift   src/sync.rs#fn reconnect",
	},
	{
		name: "merge",
		usage: "lane merge [--base <ref>] [--keep] [--squash]",
		summary: "rebases the lane onto its base, audits memory, commits it, fast-forwards the base, and removes the lane.",
		options: [
			{ flag: "--base", arg: "<ref>", about: "ref to rebase onto and advance; defaults to the lane's recorded base." },
			{ flag: "--keep", arg: null, about: "keep the lane worktree and branch after landing." },
			{ flag: "--squash", arg: null, about: "squash the lane's commits into one landing commit." },
			{ flag: "--cd", arg: null, about: "print the main root path last on stdout, for the shell function." },
			{ flag: "--max-notes", arg: "<n>", about: "keep at most this many notes per path and anchor; default 5." },
			{ flag: "--max-chars", arg: "<n>", about: "keep at most this many characters per path and anchor; default 1200." },
		],
		example: "$ lane merge\nrebased onto main\nmemory: +2 new; checked 8: 7 fresh, 1 content-changed,\n        0 contract-changed, 0 missing\ncommitted memory update\nfast-forwarded main\nremoved lane fix-login",
	},
	{
		name: "push",
		usage: "lane push [--base <ref>]",
		summary: "rebases the lane onto its base, audits and commits memory, then pushes it for a pull request.",
		options: [
			{ flag: "--base", arg: "<ref>", about: "ref to rebase onto; defaults to the lane's recorded base." },
			{ flag: "--max-notes", arg: "<n>", about: "keep at most this many notes per path and anchor; default 5." },
			{ flag: "--max-chars", arg: "<n>", about: "keep at most this many characters per path and anchor; default 1200." },
		],
		example: "$ lane push\nrebased onto main\npushed fix-login to origin",
	},
	{
		name: "prune",
		usage: "lane prune [--dry-run]",
		summary: "removes every lane that has landed in trunk, seen through a squash or rebase merge, skipping any that is dirty, holds work trunk does not have, or is the directory you are in.",
		options: [
			{ flag: "--dry-run", arg: null, about: "list what would go, remove nothing." },
		],
		example: "$ lane prune\nremoved fix-login\nskipped rate-limit: uncommitted changes",
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
		summary: "prints the shell function that makes new, cd and merge leave you in the right directory.",
		options: [],
		example: "$ eval \"$(lane shellenv)\"\n$ lane shellenv\nlane() {\n  case \"$1\" in\n    new)   shift; p=$(command lane new --cd \"$@\") ...\n    cd)    shift; p=$(command lane path \"$@\") ...\n    merge) shift; p=$(command lane merge --cd \"$@\") ...\n    *)     command lane \"$@\" ;;\n  esac\n}",
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
