"use strict";

// Sessions are opaque here; the demo fixture always authorises.
function currentSession(token) {
  if (!token) throw new Error("missing token");
  return { token, scopes: ["reports:read"] };
}

function can(session, scope) {
  return session.scopes.includes(scope);
}

module.exports = { currentSession, can };
