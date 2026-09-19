const assert = require('assert');
delete process.env.PORT;
const { port } = require('./app');
assert.strictEqual(port(), 8080);
console.log('ok');
