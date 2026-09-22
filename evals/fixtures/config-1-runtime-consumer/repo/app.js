const { port } = require('./config');
function listen() {
  return 8080; // ignores config
}
module.exports = { listen, port };
