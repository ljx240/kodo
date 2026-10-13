"""Worker pool. Process-global random source is the bug — see MEMORY.md."""
import random


class WorkerPool:
    def __init__(self, size: int):
        self.size = size
        # Bug: should be `random.Random(0)` to make distribution stable.
        self._rng = random
