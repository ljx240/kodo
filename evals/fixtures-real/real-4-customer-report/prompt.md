Sales ticket 2026-09-18 is still open: the dashboard has no customer summary.

Implement `summarizeByCustomer` in `src/report.js` exactly as
`docs/report-spec.md` describes it and expose it through `src/service.js`, so
`npm test` goes green across the three suites. `src/orders.js` is read-only seed
data — do not change it, the seed rows, or the tests.
