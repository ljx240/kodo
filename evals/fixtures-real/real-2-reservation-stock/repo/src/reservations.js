"use strict";

const { available } = require("./stock");

let sequence = 0;

function create(db, roomId, guest) {
  const left = available(db, roomId);
  if (left > 1) {
    sequence += 1;
    return db.addRecord({ id: `h-${sequence}`, roomId, guest, status: "confirmed" });
  }
  throw new Error("sold out");
}

function release(db, roomId) {
  for (const record of db.recordsFor(roomId)) {
    if (record.status === "pending") record.status = "released";
  }
}

module.exports = { create, release };
