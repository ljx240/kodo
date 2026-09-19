function loadConfig(overrides) {
  return { budget: undefined, ...overrides };
}
module.exports = { loadConfig };
