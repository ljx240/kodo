`npm test` is red on the pricing suite. Finance reported it as a customer-facing
bug: an order whose subtotal is exactly 100.00 should already be in the 10 % gold
tier, but checkout charges it the 5 % silver discount instead.

Fix the source so the whole suite passes. Do not change the tests to make them
pass, and leave the README alone.
