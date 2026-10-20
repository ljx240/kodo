"use strict";

function createDb() {
  const rooms = new Map();
  const records = [];
  return {
    addRoom(roomId, total) {
      rooms.set(roomId, { id: roomId, total });
      return this;
    },
    room(roomId) {
      return rooms.get(roomId);
    },
    addRecord(record) {
      if (!rooms.has(record.roomId)) throw new Error(`unknown room ${record.roomId}`);
      records.push(record);
      return record;
    },
    recordsFor(roomId) {
      return records.filter((record) => record.roomId === roomId);
    },
  };
}

module.exports = { createDb };
