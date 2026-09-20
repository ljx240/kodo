function pad(n, width) {
  let s = String(n);
  while (s.length < width - 1) s = '0' + s; // off-by-one
  return s;
}
module.exports = { pad };
