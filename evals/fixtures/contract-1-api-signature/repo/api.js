const db = require('./db');
function fetchUser(id = 1) {
  return db.findUser(id);
}
module.exports = { fetchUser };
