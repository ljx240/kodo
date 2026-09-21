const assert = require('assert');
const { sum } = require('./sum');
const { label } = require('./other_helper');
assert.strictEqual(sum(1, 2), 3);
assert.strictEqual(label(), 'y'); // unrelated broken expectation
console.log('ok');
