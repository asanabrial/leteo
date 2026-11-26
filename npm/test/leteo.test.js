"use strict";

const assert = require("node:assert");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const { libcFromReport } = require("../bin/libc.js");

test("a glibc runtime is named by the version Node resolved", () => {
  assert.equal(libcFromReport({ header: { glibcVersionRuntime: "2.36" } }), "glibc");
});

test("a musl runtime reports no glibc version", () => {
  // What Alpine's Node answers: the header is there and the field is not.
  assert.equal(libcFromReport({ header: {} }), "musl");
  assert.equal(libcFromReport({}), "musl");
});

test("the wrapper refuses a musl system before it downloads anything", () => {
  const wrapper = require.resolve("../bin/leteo.js");
  const musl = require.resolve("./musl-report.js");
  const result = spawnSync(process.execPath, ["--require", musl, wrapper, "--version"], {
    encoding: "utf8",
  });

  assert.equal(result.error, undefined);
  assert.equal(result.status, 1, `the wrapper exited ${result.status}`);
  assert.equal(result.stdout, "", "a failure must not write to the MCP channel");
  assert.match(result.stderr, /musl/);
  assert.match(result.stderr, /cargo install leteo/);
  assert.doesNotMatch(
    result.stderr,
    /fetching/,
    "the refusal has to come before the download, not after it",
  );
});
