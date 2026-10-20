# Booking rules (house policy, 2026-08-15)

Availability for a room type is `total - confirmed holds`.

A `pending` hold is a courtesy reservation the front desk can still release, so
**pending records never reduce availability**. Only `confirmed` records do.

`create(db, roomId, guest)` books the *last* remaining unit too: it succeeds
while availability is at least 1, and it throws `sold out` only when
availability has reached 0. A successful booking writes a `confirmed` record.

Rates and night counts live in `src/pricing.js` and `src/dates.js`; this file
does not describe them.
