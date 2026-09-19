const assert = require('assert');
const { fetchUser } = require('./api');
(async () => {
  const u = await fetchUser('u1');
  assert.strictEqual(u.name, 'Ada');
  console.log('ok');
})();
