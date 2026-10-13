"""Request cache. Unbounded — see MEMORY.md."""


class RequestCache:
    def __init__(self):
        # Bug: no cap, strong refs on the value side.
        self._bodies = {}

    def put(self, request_id, body):
        self._bodies[request_id] = body

    def get(self, request_id):
        return self._bodies.get(request_id)
