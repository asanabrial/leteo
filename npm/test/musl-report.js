// The report and platform an Alpine Node produces, for the wrapper's own tests.
//
// `node --require` this file before `bin/leteo.js` and the wrapper sees a musl
// Linux on a runner that is neither. `process.report` and `process.platform`
// are configurable accessors on Node, which is why a preload can stand in for
// the runtime rather than the wrapper growing a test-only switch.
"use strict";

process.report.getReport = () => ({ header: {} });
Object.defineProperty(process, "platform", { value: "linux" });
