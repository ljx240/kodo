const assert = require('assert');
const { clamp } = require('./math');
assert.strictEqual(clamp(10, 5), 5);
console.log('ok');
