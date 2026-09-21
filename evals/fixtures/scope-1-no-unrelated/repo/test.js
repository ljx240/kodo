const assert = require('assert');
const { pad } = require('./pad');
assert.strictEqual(pad(1, 3), '001');
assert.strictEqual(pad(12, 3), '012');
console.log('ok');
