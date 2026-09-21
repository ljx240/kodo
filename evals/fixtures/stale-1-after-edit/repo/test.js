const assert = require('assert');
const { version } = require('./package_info');
assert.strictEqual(version, '1.0.0'); // stale expectation after product change
console.log('ok');
