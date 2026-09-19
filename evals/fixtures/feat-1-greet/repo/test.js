const assert = require('assert');
const { greet } = require('./greet');
assert.strictEqual(greet('Ada'), 'Hello, Ada!');
console.log('ok');
