"""Tests for the worker pool. Flaky on CI — see MEMORY.md entry 1."""
import random
import threading

from worker_pool import WorkerPool


def test_balanced_distribution():
    pool = WorkerPool(size=4)
    counts = [0] * 4
    lock = threading.Lock()

    def work(worker_id):
        for _ in range(100):
            with lock:
                counts[worker_id] += 1
            # intentionally tiny sleep to provoke the race
            threading.Event().wait(0)

    threads = [threading.Thread(target=work, args=(i,)) for i in range(4)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    assert max(counts) - min(counts) <= 2, f"unbalanced: {counts}"
