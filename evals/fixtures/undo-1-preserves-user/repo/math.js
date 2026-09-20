function clamp(n, max) {
  return n > max ? max - 1 : n;
}
module.exports = { clamp };
