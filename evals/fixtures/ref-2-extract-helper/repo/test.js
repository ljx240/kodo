const assert = require('assert');
const { area, vol } = require('./math');
assert.strictEqual(area(2,3), 6);
assert.strictEqual(vol(2,3,4), 24);
console.log('ok');
