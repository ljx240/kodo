const { reducer } = require('./state');
function render(state, action) {
  const next = reducer(state, action);
  return `count=${next.count}`;
}
module.exports = { render };
