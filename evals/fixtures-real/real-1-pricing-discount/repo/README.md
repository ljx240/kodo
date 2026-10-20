# kodo-real-pricing

Checkout pricing for the demo storefront. Money is integer cents everywhere;
`src/money.js` holds the helpers and nothing else may round.

Run the suite with `npm test`.

## Layout

| path | what it does |
|------|--------------|
| `src/cart.js` | cart rows, subtotal, item count |
| `src/discounts.js` | tier table from the finance spec |
| `src/tax.js` | sales tax on the discounted net |
| `src/shipping.js` | flat shipping bands from the carrier contract |
| `src/orders.js` | `finalizeOrder`, order of operations |

The finance spec (2026-09-02) fixes the tier boundaries as inclusive: a subtotal
of exactly 100.00 is already gold tier.
