# Customer summary report (sales ticket 2026-09-18)

`summarizeByCustomer(orders)` in `src/report.js` returns one row per customer.

- `orders` is optional and defaults to `orders.listOrders()`.
- Only `status === "paid"` counts. Cancelled and refunded orders are ignored
  entirely: they contribute neither a total nor a count.
- Group by `customerId`, then emit rows of exactly
  `{ customerId, name, totalCents, count }`, where `name` comes from
  `customers.findName(customerId)` (an unknown id keeps its `unknown:<id>`
  placeholder) and `count` is the number of paid orders.
- Sort by `totalCents` descending. Break ties by `customerId` ascending.
- Customers with no paid order do not appear; an empty input gives an empty list.

`src/service.js` exposes it to the dashboard: add the kind `by-customer` to its
`KINDS` list and make `report("by-customer")` return the summary. `daily` stays
as it is.
