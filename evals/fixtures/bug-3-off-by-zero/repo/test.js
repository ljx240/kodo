const { clamp } = require('./clamp');
const assert = require('assert');
assert.strictEqual(clamp(10, 0, 5), 5);
assert.strictEqual(clamp(-1, 0, 5), 0);
console.log('ok');
