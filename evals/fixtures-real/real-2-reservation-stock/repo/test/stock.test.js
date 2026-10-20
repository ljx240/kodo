"use strict";

const assert = require("assert");
const { createDb } = require("../src/db");
const { available } = require("../src/stock");

const db = createDb();
db.addRoom("std", 5);
db.addRoom("empty", 1);
db.addRecord({ roomId: "std", guest: "ann", status: "confirmed" });
db.addRecord({ roomId: "std", guest: "bo", status: "confirmed" });
db.addRecord({ roomId: "std", guest: "cy", status: "pending" });

assert.strictEqual(available(db, "std"), 3, "pending holds do not reduce stock");
assert.strictEqual(available(db, "empty"), 1, "an untouched room type keeps its total");
assert.throws(() => available(db, "attic"), /unknown room/);

console.log("stock.test ok");
