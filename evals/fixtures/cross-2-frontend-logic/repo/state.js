function reducer(state, action) {
  if (action.type === 'inc') return { count: state.count }; // BUG: not incrementing
  return state;
}
module.exports = { reducer };
