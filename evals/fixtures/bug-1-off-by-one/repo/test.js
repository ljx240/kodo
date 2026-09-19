const { sum } = require('./sum');
const assert = require('assert');
assert.strictEqual(sum([1,2,3]), 6, 'sum must be 6');
console.log('ok');
