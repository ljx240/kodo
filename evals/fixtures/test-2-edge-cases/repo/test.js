const assert = require('assert');
const { pad } = require('./pad');
assert.strictEqual(pad(7, 3), '007');
console.log('ok');
