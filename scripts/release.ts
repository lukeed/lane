#!/usr/bin/env bun
/**
 * Bump the workspace version, then commit, tag, and push the release.
 * The tag is what starts .github/workflows/release.yml.
 *
 * @example
 *   ./scripts/release.ts patch
 */

import { $, argv } from "bun";

const ROOT = import.meta.dir + "/..";
const CARGO_FILE = ROOT + "/Cargo.toml";
const VERSION = /(?<=\[workspace\.package\][^[]*?\nversion\s*=\s*")\d+\.\d+\.\d+(?=")/;

let [increment] = argv.slice(2);
if (!increment) throw new Error("usage: ./scripts/release.ts <major|minor|patch>");

let dirty = await $`git status --porcelain`.cwd(ROOT).text();
if (dirty.trim()) throw new Error("working tree is dirty");

let source = await Bun.file(CARGO_FILE).text();
let current = source.match(VERSION)?.[0];
if (!current) throw new Error("missing [workspace.package] version");

let [major, minor, patch] = current.split(".").map(Number);

if (increment === "major") var version = `${major + 1}.0.0`;
else if (increment === "minor") version = `${major}.${minor + 1}.0`;
else if (increment === "patch") version = `${major}.${minor}.${patch + 1}`;
else throw new Error(`unknown increment: "${increment}"`);

await Bun.write(CARGO_FILE, source.replace(VERSION, version));
// Resolution only: `cargo check` would compile the workspace to rewrite one string.
await $`cargo update --workspace --offline`.cwd(ROOT).quiet();
await $`git add Cargo.toml Cargo.lock`.cwd(ROOT);
await $`git commit -m ${`release: v${version}`}`.cwd(ROOT).quiet();
await $`git tag -m ${`v${version}`} ${`v${version}`}`.cwd(ROOT);
await $`git push origin HEAD --tags`.cwd(ROOT);

console.log(`\n~> pushed v${version} release`);
