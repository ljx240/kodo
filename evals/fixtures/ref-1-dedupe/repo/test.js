const assert = require('assert');
const { unique, uniqueAgain } = require('./list');
assert.deepStrictEqual(unique([1,1,2]), [1,2]);
assert.deepStrictEqual(uniqueAgain([1,1,2]), [1,2]);
console.log('ok');
