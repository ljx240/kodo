"use strict";

const DAY_MS = 86400000;

function parseDate(iso) {
  const parsed = new Date(`${iso}T00:00:00Z`);
  if (Number.isNaN(parsed.getTime())) throw new Error(`bad date ${iso}`);
  return parsed;
}

function nights(checkIn, checkOut) {
  return Math.round((parseDate(checkOut) - parseDate(checkIn)) / DAY_MS);
}

function isWeekend(iso) {
  const day = parseDate(iso).getUTCDay();
  return day === 0 || day === 6;
}

module.exports = { nights, isWeekend };
