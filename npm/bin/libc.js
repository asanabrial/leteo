// Which C library the Node running this was built against.
//
// The release has no musl build, so a Linux Node on Alpine would fetch the
// glibc archive, verify it, extract it, and fail at `spawnSync` with `ENOENT`
// for a loader it does not have — an error that reads as a missing file, not as
// an unsupported platform.
//
// `process.report` is the answer Node itself gives: a glibc build reports the
// runtime version it resolved, and a musl build reports none. It is the field
// `detect-libc` reads, and the alternative — looking for `/lib/ld-musl-*.so.1`
// — is a guess about a layout that varies by distribution. No dependency is
// added for one field.
"use strict";

function libcFromReport(report) {
  const header = report && report.header;
  return header && header.glibcVersionRuntime ? "glibc" : "musl";
}

module.exports = { libcFromReport };
