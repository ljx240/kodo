const assert = require('assert');
const { fetchUser } = require('./api');
assert.throws(() => fetchUser(), /id/i);
assert.strictEqual(fetchUser(7).id, 7);
console.log('ok');
