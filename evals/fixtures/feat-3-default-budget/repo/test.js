const assert = require('assert');
const { loadConfig } = require('./config');
assert.strictEqual(loadConfig({}).budget, 100);
assert.strictEqual(loadConfig({ budget: 5 }).budget, 5);
console.log('ok');
