const assert = require('assert');
const { sumAll, totalAll } = require('./math');
assert.strictEqual(sumAll([1,2,3]), 6);
assert.strictEqual(totalAll(), 6);
assert.strictEqual(require('./math').total, undefined);
assert.strictEqual(require('./app').run(), 6);
console.log('ok');
