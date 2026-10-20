The front desk filed two bugs against the booking code: a room type shows as
taken while another guest's hold is still only pending, and the very last unit of
a room type can never be booked at all.

`npm test` is red on the stock and reservations suites. `docs/spec.md` is the
house policy and the tests are the contract — fix the source under `src/` so all
three suites pass without editing the tests or the spec.
