#!/usr/bin/env node
// Keeps the Windows-only branch free of the paths in excluded-paths.txt.
//
//   node scripts/windows-only/prune-after-merge.mjs            during `git merge --no-commit upstream/dev`:
//                                                              removes every excluded path, which settles
//                                                              modify/delete conflicts and new upstream
//                                                              files in excluded folders alike
//   node scripts/windows-only/prune-after-merge.mjs --check    prints each tracked excluded path; exit 1 if any
//   node scripts/windows-only/prune-after-merge.mjs --fmt-check
//                                                              `cargo fmt --all -- --check` with temporary
//                                                              empty files for the cfg-gated modules this
//                                                              branch deleted (rustfmt follows every `mod`,
//                                                              whatever the target); the files are removed after
//
// Run it with node: the Python route crashes on Windows' cp1252 console.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, readdirSync, rmdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

function git(args, options = {}) {
  const run = spawnSync("git", args, { encoding: "utf8", maxBuffer: 1 << 28, ...options });
  if (run.error) throw run.error;
  return run;
}

const root = git(["rev-parse", "--show-toplevel"]).stdout.trim();
const specs = readFileSync(join(here, "excluded-paths.txt"), "utf8")
  .split(/\r?\n/)
  .map((line) => line.trim())
  .filter((line) => line && !line.startsWith("#"));

function trackedExcluded() {
  const run = git(["ls-files", "-z", "--", ...specs], { cwd: root });
  return [...new Set(run.stdout.split("\0").filter(Boolean))].sort();
}

function check() {
  const found = trackedExcluded();
  for (const path of found) console.log(path);
  if (found.length) {
    console.error(`${found.length} excluded path(s) are tracked.`);
    return 1;
  }
  console.log("No excluded path is tracked.");
  return 0;
}

function prune() {
  if (git(["rev-parse", "-q", "--verify", "MERGE_HEAD"], { cwd: root }).status !== 0) {
    console.error("No merge in progress. Run `git merge --no-commit upstream/dev` first.");
    return 2;
  }
  const found = trackedExcluded();
  const run = git(["rm", "-r", "-f", "-q", "--ignore-unmatch", "--", ...specs], { cwd: root });
  if (run.status !== 0) {
    console.error(run.stderr);
    return run.status ?? 1;
  }
  for (const path of found) console.log(`removed ${path}`);
  console.log(`Pruned ${found.length} excluded path(s).`);
  return 0;
}

// Paths the branch lacks that a cfg-gated `mod` still names. The first pass
// seeds every Rust file the excluded paths hold on the full-tree branch; the
// loop then adds whatever rustfmt still reports.
function fmtCheck() {
  const created = [];
  const dirs = [];
  const place = (relative) => {
    const file = resolve(root, relative);
    if (existsSync(file)) return false;
    let dir = dirname(file);
    const missing = [];
    while (!existsSync(dir)) {
      missing.push(dir);
      dir = dirname(dir);
    }
    mkdirSync(dirname(file), { recursive: true });
    dirs.push(...missing.reverse());
    // rustfmt rewrites an empty file to a newline, so start from one.
    writeFileSync(file, "\n");
    created.push(file);
    return true;
  };
  try {
    const seed = git(["ls-tree", "-r", "--name-only", "personal", "--", ...specs.filter((s) => !s.startsWith(":"))], { cwd: root });
    if (seed.status === 0) {
      for (const path of seed.stdout.split("\n").filter((p) => p.endsWith(".rs"))) place(path);
    }
    for (let pass = 0; pass < 200; pass++) {
      const run = spawnSync("cargo", ["fmt", "--all", "--", "--check"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
      if (run.status === 0) {
        console.log(`fmt ok (${created.length} temporary module file(s))`);
        return 0;
      }
      const output = `${run.stdout}\n${run.stderr}`;
      let progress = false;
      const base = root.replace(/\\/g, "/");
      for (const line of output.split(/\r?\n/)) {
        if (!/does not exist|couldn't find file|file not found/.test(line)) continue;
        const found = line.match(/([A-Za-z]:\\[^`'"]+?\.rs)/);
        if (!found) continue;
        const absolute = found[1].replace(/\\/g, "/");
        if (!absolute.toLowerCase().startsWith(base.toLowerCase() + "/")) continue;
        if (place(absolute.slice(base.length + 1))) progress = true;
      }
      if (!progress) {
        process.stdout.write(output);
        console.error("rustfmt reported a difference or an error this script cannot settle.");
        return 1;
      }
    }
    console.error("too many passes");
    return 1;
  } finally {
    for (const file of created) rmSync(file, { force: true });
    for (const dir of dirs.reverse()) {
      try {
        if (readdirSync(dir).length === 0) rmdirSync(dir);
      } catch {
        // already gone
      }
    }
  }
}

const mode = process.argv[2];
let code;
if (mode === "--check") code = check();
else if (mode === "--fmt-check") code = fmtCheck();
else if (mode === undefined) code = prune();
else {
  console.error("usage: prune-after-merge.mjs [--check | --fmt-check]");
  code = 2;
}
process.exit(code);
