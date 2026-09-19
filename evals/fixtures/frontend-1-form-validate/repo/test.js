const assert = require('assert');
const { isValidEmail } = require('./validate');
assert.strictEqual(isValidEmail('a@b.co'), true);
assert.strictEqual(isValidEmail('not-an-email'), false);
assert.strictEqual(isValidEmail('a@b'), false); // requires TLD dot after @
console.log('ok');
