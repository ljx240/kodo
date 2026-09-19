const { displayName } = require('./name');
const assert = require('assert');
assert.strictEqual(displayName(null), 'anonymous');
console.log('ok');
