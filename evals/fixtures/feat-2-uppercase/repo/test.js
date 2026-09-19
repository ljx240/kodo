const assert = require('assert');
const { shout } = require('./str');
assert.strictEqual(shout('hi'), 'HI');
console.log('ok');
