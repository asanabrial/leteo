#!/usr/bin/env node
// The npm face of Leteo: `npx leteo mcp` in an MCP client's config, with no
// Rust toolchain and no install script.
//
// This does not reimplement the product. It fetches the same release archive
// `install.sh` and `install.ps1` fetch, verifies it against the same published
// `SHA256SUMS`, and hands every argument to the same binary. What it saves is
// the step where somebody has to install something before editing a JSON file.
//
// Zero dependencies on purpose. This runs under `npx -y` inside an agent's
// process tree, where every extra package is another thing to resolve before
// the agent can speak to its memory, and another thing that can go wrong in a
// place nobody is watching.

"use strict";

const { createHash } = require("node:crypto");
const { spawnSync, execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const REPO = "asanabrial/leteo";

// Five builds, matching the release workflow's matrix. `npm/tests` in the Rust
// crate holds this table against `.github/workflows/release.yml`, because a
// target added there and not here is a platform that silently falls back to
// "no prebuilt Leteo" — the failure is a sentence, so nobody would look.
const TARGETS = {
  "linux-x64": { triple: "x86_64-unknown-linux-gnu", archive: "tar.gz", exe: "leteo" },
  "linux-arm64": { triple: "aarch64-unknown-linux-gnu", archive: "tar.gz", exe: "leteo" },
  "darwin-x64": { triple: "x86_64-apple-darwin", archive: "tar.gz", exe: "leteo" },
  "darwin-arm64": { triple: "aarch64-apple-darwin", archive: "tar.gz", exe: "leteo" },
  "win32-x64": { triple: "x86_64-pc-windows-msvc", archive: "zip", exe: "leteo.exe" },
};

// Everything this prints goes to stderr, without exception. `leteo mcp` speaks
// JSON-RPC over stdout, and one line of progress written there is a message the
// client cannot parse — an MCP server that fails at the handshake for a reason
// that looks like a protocol bug rather than a download.
function say(message) {
  process.stderr.write(`${message}\n`);
}

// Thrown rather than exited on, because `process.exit()` here crashes.
//
// Every failure below happens while `fetch` still holds handles open, and
// calling `process.exit()` from inside that on Windows aborts the process:
//
//   Assertion failed: !(handle->flags & UV_HANDLE_CLOSING), src\win\async.c:76
//
// which leaves exit code 127 and a C assertion where the message about the
// checksum should have been. Measured on the tampered-archive check below: the
// mismatch was found and reported correctly, and then the report was buried.
// Setting `exitCode` and letting the loop unwind gives 1 and the sentence.
class LeteoError extends Error {}

function fail(message) {
  throw new LeteoError(message);
}

function platformKey() {
  return `${process.platform}-${process.arch}`;
}

function resolveTarget() {
  const key = platformKey();
  const target = TARGETS[key];
  if (!target) {
    fail(
      `no prebuilt Leteo for ${key}. Build from source instead: ` +
        `cargo install leteo`,
    );
  }
  return target;
}

async function download(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText} for ${url}`);
  }
  return Buffer.from(await response.arrayBuffer());
}

// The checksum is not optional and there is no flag to skip it. This runs
// unattended inside an agent, which is exactly the situation where nobody would
// notice a substituted archive, so an unverifiable download is a failure rather
// than a warning — the same choice `install.sh` makes in its own words.
function verify(archive, sums, name) {
  const line = sums
    .split("\n")
    .map((entry) => entry.trim())
    .find((entry) => entry.endsWith(name) || entry.endsWith(`*${name}`));
  if (!line) {
    fail(`no checksum published for ${name}; refusing to run unverified`);
  }
  const expected = line.split(/\s+/)[0];
  const actual = createHash("sha256").update(archive).digest("hex");
  if (actual !== expected) {
    fail(
      `checksum mismatch for ${name}\n  expected ${expected}\n  got      ${actual}`,
    );
  }
}

// `tar` rather than a dependency, and an absolute path on Windows because the
// name is taken there. Git Bash puts GNU tar first on PATH and GNU tar cannot
// read a zip: it answers "This does not look like a tar archive". The bsdtar
// Windows has shipped in System32 since Windows 10 1803 reads both, and that
// path is not on anybody's PATH to shadow.
function extract(archivePath, into) {
  const tar =
    process.platform === "win32"
      ? path.join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe")
      : "tar";
  execFileSync(tar, ["-xf", archivePath, "-C", into], { stdio: "inherit" });
}

async function fetchBinary(version, target, destination) {
  const packageName = `leteo-${version}-${target.triple}`;
  const archiveName = `${packageName}.${target.archive}`;
  // Overridable for the same reason the shell scripts allow it: an internal
  // mirror serving the same layout, and a way to exercise this path without
  // publishing anything.
  const base =
    process.env.LETEO_BASE_URL ||
    `https://github.com/${REPO}/releases/download/${version}`;

  say(`leteo: fetching ${version} for ${target.triple}`);

  let archive;
  let sums;
  try {
    [archive, sums] = await Promise.all([
      download(`${base}/${archiveName}`),
      download(`${base}/SHA256SUMS`),
    ]);
  } catch (error) {
    fail(`could not download ${version}: ${error.message}`);
  }

  verify(archive, sums.toString("utf8"), archiveName);

  // Unpack beside the destination and rename into place, so two `npx leteo`
  // processes racing on a cold cache cannot leave a half-written binary that
  // the loser then executes.
  const staging = fs.mkdtempSync(path.join(path.dirname(destination), "staging-"));
  try {
    const archivePath = path.join(staging, archiveName);
    fs.writeFileSync(archivePath, archive);
    extract(archivePath, staging);

    const unpacked = path.join(staging, packageName, target.exe);
    if (!fs.existsSync(unpacked)) {
      fail(`${archiveName} did not contain ${packageName}/${target.exe}`);
    }
    fs.chmodSync(unpacked, 0o755);
    fs.renameSync(unpacked, destination);
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}

async function main() {
  const target = resolveTarget();
  const { version } = require("../package.json");
  // The npm version and the release tag are the same number by construction:
  // this package exists to deliver that release and nothing else. LETEO_VERSION
  // overrides it for the same reason the scripts allow it.
  const tag = process.env.LETEO_VERSION || `v${version}`;

  const home = path.join(__dirname, "..", "vendor");
  fs.mkdirSync(home, { recursive: true });
  const binary = path.join(home, `${tag}-${target.triple}-${target.exe}`);

  if (!fs.existsSync(binary)) {
    await fetchBinary(tag, target, binary);
  }

  const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) {
    fail(`could not run ${binary}: ${result.error.message}`);
  }
  // A binary killed by a signal has no exit code, and reporting 0 there would
  // tell a supervising agent that a process somebody killed finished its work.
  process.exitCode = result.signal || result.status === null ? 1 : result.status;
}

main().catch((error) => {
  const message =
    error instanceof LeteoError ? error.message : error.stack || error.message;
  process.stderr.write(`leteo: ${message}\n`);
  process.exitCode = 1;
});
