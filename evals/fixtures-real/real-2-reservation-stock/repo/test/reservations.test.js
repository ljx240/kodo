"use strict";

const assert = require("assert");
const { createDb } = require("../src/db");
const { available } = require("../src/stock");
const { create } = require("../src/reservations");

const db = createDb();
db.addRoom("suite", 2);
db.addRecord({ roomId: "suite", guest: "ann", status: "confirmed" });
db.addRecord({ roomId: "suite", guest: "bo", status: "pending" });

assert.strictEqual(available(db, "suite"), 1, "one suite is still bookable");

// The last unit must still be bookable, and the walk-in after it must not be.
const held = create(db, "suite", "ada");
assert.strictEqual(held.status, "confirmed", "create() books instantly");
assert.strictEqual(available(db, "suite"), 0, "the suite is now full");
assert.throws(() => create(db, "suite", "bob"), /sold out/);

console.log("reservations.test ok");
