const assert = require('assert');
const { getUser } = require('./server');
const { renderUser } = require('./client');
const u = getUser(1);
assert.strictEqual(u.name, 'ada');
assert.strictEqual(renderUser(u), 'ada');
console.log('ok');
