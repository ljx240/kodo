function unique(xs) {
  const out = [];
  for (const x of xs) {
    if (!out.includes(x)) out.push(x);
  }
  return out;
}
function uniqueAgain(xs) {
  const out = [];
  for (const x of xs) {
    if (!out.includes(x)) out.push(x);
  }
  return out;
}
module.exports = { unique, uniqueAgain };
