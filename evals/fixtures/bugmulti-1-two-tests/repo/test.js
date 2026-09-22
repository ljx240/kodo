const assert = require('assert');
const { isEven } = require('./even');
assert.strictEqual(isEven(2), true, 'test_a');
assert.strictEqual(isEven(3), false, 'test_b');
console.log('ok');
