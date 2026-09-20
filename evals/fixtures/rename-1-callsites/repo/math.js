function total(nums) {
  return nums.reduce((a, b) => a + b, 0);
}
function totalAll() {
  return total([1, 2, 3]);
}
module.exports = { total, totalAll };
