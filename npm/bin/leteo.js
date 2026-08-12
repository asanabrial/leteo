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
const https = require("node:https");
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
// Every failure below happens while a download still holds handles open, and
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

// `node:https` rather than `fetch`, and redirects followed by hand.
//
// `fetch` fails on this download. Not intermittently and not because of the
// network: in a container where `curl` fetches the 7.9 MB archive with a 200
// every time, `fetch` answers `UND_ERR_SOCKET` — the socket closed mid-body —
// on three attempts out of three. It succeeded from Windows and failed from
// Linux against the same URL in the same minute, which is how it reached npm
// before anybody noticed. undici is doing something to a large body over
// GitHub's redirect that the classic stack does not.
//
// So this is not a retry around `fetch`. Retries are here too, because a
// download really can fail for ordinary reasons, but they would not have saved
// the case that motivated this: a bug that fails every time is not waited out.
const MAX_REDIRECTS = 5;

function get(url, redirectsLeft = MAX_REDIRECTS) {
  return new Promise((resolve, reject) => {
    const request = https.get(url, { headers: { "user-agent": "leteo-npm" } }, (response) => {
      const status = response.statusCode || 0;
      // GitHub answers a release asset with a redirect to its object store,
      // and `https.get` does not follow it on its own.
      if (status >= 300 && status < 400 && response.headers.location) {
        response.resume();
        if (redirectsLeft === 0) {
          reject(new Error(`too many redirects for ${url}`));
          return;
        }
        const next = new URL(response.headers.location, url).toString();
        resolve(get(next, redirectsLeft - 1));
        return;
      }
      if (status < 200 || status >= 300) {
        response.resume();
        reject(new Error(`${status} ${response.statusMessage || ""} for ${url}`.trim()));
        return;
      }
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => resolve(Buffer.concat(chunks)));
      response.on("error", reject);
    });
    request.on("error", reject);
    // A socket that goes quiet is a download that never ends, and this runs
    // inside an agent's start-up where nobody is watching a spinner.
    request.setTimeout(60_000, () => {
      request.destroy(new Error(`timed out after 60s for ${url}`));
    });
  });
}

async function download(url) {
  let last;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    try {
      return await get(url);
    } catch (error) {
      last = error;
      if (attempt < 3) {
        await new Promise((wait) => setTimeout(wait, attempt * 500));
      }
    }
  }
  throw last;
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
  // tar's stdout is discarded rather than inherited, for the reason every
  // message in this file goes to stderr: this runs once, on the first call,
  // which is the call carrying the MCP handshake. A tar that decides to say
  // something on stdout — a warning about an ownership it could not set, say —
  // would put it in the middle of the JSON-RPC stream. stderr is kept, because
  // a failure here throws and the reason should be readable.
  execFileSync(tar, ["-xf", archivePath, "-C", into], {
    stdio: ["ignore", "ignore", "inherit"],
  });
}

// Where the downloaded binary lives.
//
// Inside the package by preference, because then `npm uninstall` takes it away
// and an `npx` cache entry stays self-contained. But a global install puts the
// package somewhere the running user may not own — `npm i -g` under a root
// prefix is the ordinary case on Linux — and the first run would die with
// `EACCES: permission denied, mkdir '/usr/lib/node_modules/leteo/vendor'`,
// which names a path the user did not choose and cannot obviously fix.
//
// So it is attempted and then given up on, rather than predicted: whether the
// directory is writable is a question about this machine, and `access` answers
// it in a syscall. The fallback is the user's own cache, which they always own.
function cacheDirectory() {
  const beside = path.join(__dirname, "..", "vendor");
  try {
    fs.mkdirSync(beside, { recursive: true });
    fs.accessSync(beside, fs.constants.W_OK);
    return beside;
  } catch {
    // `%LOCALAPPDATA%\leteo\bin` on Windows because that is where `install.ps1`
    // already puts the binary, and `$XDG_CACHE_HOME`/`~/.cache` elsewhere by
    // the same convention everything else on those systems follows. An earlier
    // version joined `AppData` directly, which is writable and belongs to
    // nobody: a `leteo` folder sitting beside `Local` and `Roaming` where no
    // Windows program keeps anything.
    const home =
      process.platform === "win32"
        ? process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local")
        : process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
    const mine = path.join(home, "leteo", "bin");
    fs.mkdirSync(mine, { recursive: true });
    return mine;
  }
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
    // The cause, not just the message. Node's network errors arrive as the
    // word "fetch failed" or "socket hang up" with everything that identifies
    // them tucked into `cause` — the report that sent somebody hunting a
    // GitHub outage said exactly `could not download v0.1.1: fetch failed`,
    // while the code underneath was `UND_ERR_SOCKET` and named the bug.
    const cause = error.cause && (error.cause.code || error.cause.message);
    fail(
      `could not download ${version}: ${error.message}` +
        (cause ? ` (${cause})` : ""),
    );
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
    // Another process may have finished first — two MCP clients starting at
    // once on a cold cache is an ordinary Tuesday. Its copy came through the
    // same checksum, so it is the same bytes, and on Windows renaming over a
    // binary that process is already executing fails with EPERM. Leaving the
    // winner alone is both correct and the only thing that works.
    if (!fs.existsSync(destination)) {
      fs.renameSync(unpacked, destination);
    }
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

  const binary = path.join(
    cacheDirectory(),
    `${tag}-${target.triple}-${target.exe}`,
  );

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
