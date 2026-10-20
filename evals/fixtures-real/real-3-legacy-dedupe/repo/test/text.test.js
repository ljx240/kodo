"use strict";

// ADR-004: the shared module owns both helpers.
const assert = require("assert");
const text = require("../src/text");

assert.strictEqual(typeof text.formatDate, "function", "text.js must export formatDate");
assert.strictEqual(typeof text.padAmount, "function", "text.js must export padAmount");
assert.strictEqual(text.formatDate("2026-10-03"), "03 Oct 2026");
assert.strictEqual(text.formatDate("2026-01-09"), "09 Jan 2026");
assert.strictEqual(text.padAmount("1234.50"), "   1234.50");
assert.strictEqual(text.padAmount("1"), "         1");
// Untouched helpers keep working.
assert.strictEqual(text.titleCase("acme industrial"), "Acme Industrial");

console.log("text.test ok");
