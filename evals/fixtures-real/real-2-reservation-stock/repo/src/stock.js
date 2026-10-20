"use strict";

// House policy: see docs/spec.md — availability counts confirmed holds only.
function available(db, roomId) {
  const room = db.room(roomId);
  if (!room) throw new Error(`unknown room ${roomId}`);
  let held = 0;
  for (const record of db.recordsFor(roomId)) {
    if (record.status === "confirmed" || record.status === "pending") held += 1;
  }
  return room.total - held;
}

module.exports = { available };
