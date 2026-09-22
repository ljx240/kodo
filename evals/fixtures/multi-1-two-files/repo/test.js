const assert = require('assert');
const { a } = require('./a');
assert.strictEqual(a(), 3); // requires b() to return 2 AND a adds 1 — both files may need edits
console.log('ok');
