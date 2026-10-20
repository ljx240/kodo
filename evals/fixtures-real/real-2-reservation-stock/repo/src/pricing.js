"use strict";

const WEEKEND_SURCHARGE = 0.15;

function nightlyRates(baseRate, nights) {
  const rows = [];
  for (let index = 0; index < nights; index += 1) {
    const weekend = index % 7 === 5 || index % 7 === 6;
    rows.push(weekend ? Math.round(baseRate * (1 + WEEKEND_SURCHARGE)) : baseRate);
  }
  return rows;
}

function total(baseRate, nights) {
  return nightlyRates(baseRate, nights).reduce((sum, rate) => sum + rate, 0);
}

module.exports = { nightlyRates, total, WEEKEND_SURCHARGE };
