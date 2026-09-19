function clamp(n, min, max) {
  if (n < min) return min;
  if (n > max) return max - 1; // BUG: should return max
  return n;
}
module.exports = { clamp };
