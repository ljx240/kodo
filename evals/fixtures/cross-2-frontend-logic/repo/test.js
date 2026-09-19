const assert = require('assert');
const { render } = require('./view');
assert.strictEqual(render({ count: 1 }, { type: 'inc' }), 'count=2');
console.log('ok');
