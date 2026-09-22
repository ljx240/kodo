function area(w, h) { return w * h; }
function vol(w, h, d) {
  let t = 0;
  for (let i = 0; i < d; i++) t += w * h;
  return t;
}
module.exports = { area, vol };
