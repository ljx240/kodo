"use strict";

const assert = require("assert");
const { renderPacket } = require("../src/documents");

const packet = renderPacket(
  { name: "acme industrial" },
  { number: "7", issuedAt: "2026-10-03", total: "1234.50" },
  { number: "12", validUntil: "2026-11-30", total: "98.00" }
);

assert.deepStrictEqual(packet.split("\n"), [
  "Acme Industrial",
  "INV-7 | 03 Oct 2026 | total    1234.50",
  "QTE-12 | valid until 30 Nov 2026 |      98.00",
]);

console.log("documents.test ok");
