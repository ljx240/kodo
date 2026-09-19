const assert = require('assert');
const { twice } = require('./fix');
assert.strictEqual(twice(2), 4);
console.log('ok');
