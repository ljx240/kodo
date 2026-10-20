"use strict";

// Green today, green tomorrow: out of scope for ADR-004.
const assert = require("assert");
const { summarize } = require("../src/legacy/ledger");

assert.strictEqual(
  summarize([
    { account: "fees", amountCents: 1200 },
    { account: "tax", amountCents: 300 },
    { account: "fees", amountCents: 800 },
  ]),
  "fees: 2000\ntax: 300"
);
assert.strictEqual(summarize([]), "");

console.log("ledger.test ok");
