const assert = require('assert');
const { listen, port } = require('./app');
assert.strictEqual(listen(), port);
assert.strictEqual(port, 3000);
console.log('ok');
