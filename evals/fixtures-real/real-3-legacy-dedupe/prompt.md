The print shop rejected this week's paperwork: invoices come out as
`October 03, 2026` and quotes as `03 Nov 2026` for the same date, and totals are
zero-padded on invoices but space-padded on quotes.

`docs/adr-004.md` records the decision we already made about this. Carry it out:
`npm test` must go green and the legacy renderers must stop keeping private
copies of the date and amount helpers. Do not edit the tests, and leave
`src/legacy/ledger.js` alone.
