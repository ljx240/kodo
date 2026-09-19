const users = { u1: { id: 'u1', name: 'Ada' } };
module.exports = { findUser: (id) => users[id] || null };
