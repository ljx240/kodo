"use strict";

// Suites for the modules this bug report does not touch. They are green today
// and must stay green.
const assert = require("assert");
const { nights, isWeekend } = require("../src/dates");
const { nightlyRates, total } = require("../src/pricing");

assert.strictEqual(nights("2026-10-01", "2026-10-04"), 3);
assert.strictEqual(nights("2026-10-01", "2026-10-01"), 0);
assert.strictEqual(isWeekend("2026-10-03"), true, "Saturday");
assert.strictEqual(isWeekend("2026-10-01"), false, "Thursday");
assert.throws(() => nights("nope", "2026-10-04"), /bad date/);

assert.deepStrictEqual(nightlyRates(100, 3), [100, 100, 100]);
assert.strictEqual(total(100, 7), 730, "five weekdays plus two weekend nights");

console.log("support.test ok");
