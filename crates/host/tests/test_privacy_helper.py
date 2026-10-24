import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "privacy_helper", Path(__file__).parents[1] / "src" / "privacy_helper.py")
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


class FakeDisplay:
    def __init__(self):
        # Nonlinear, distinct ramps must be restored exactly, on every display.
        self.saved = {7: ((10, 90), (20, 80), (30, 70)),
                      8: ((1, 99), (2, 98), (3, 97))}
        self.calls = []
        self.fail_once = False

    def snapshot(self):
        return self.saved

    def apply(self, crtc, channels):
        self.calls.append((crtc, channels))
        if self.fail_once:
            self.fail_once = False
            raise RuntimeError("driver failure")


class RestorationTests(unittest.TestCase):
    def test_disconnect_restores_exact_ramps_on_all_displays(self):
        d = FakeDisplay()
        helper.blank_until_disconnect(d, lambda: False)
        self.assertEqual(d.calls[:2], [(7, ((0, 0),) * 3), (8, ((0, 0),) * 3)])
        self.assertEqual(d.calls[-2:], list(d.saved.items()))

    def test_heartbeat_failure_restores(self):
        d = FakeDisplay()
        def failed_heartbeat():
            raise OSError("parent gone")
        with self.assertRaises(OSError):
            helper.blank_until_disconnect(d, failed_heartbeat)
        self.assertEqual(d.calls[-2:], list(d.saved.items()))

    def test_partial_blanking_failure_restores_all(self):
        d = FakeDisplay()
        d.fail_once = True
        with self.assertRaises(RuntimeError):
            helper.blank_until_disconnect(d, lambda: True)
        self.assertEqual(d.calls[-2:], list(d.saved.items()))

    def test_reapplies_while_connected(self):
        d = FakeDisplay()
        heartbeats = iter([True, True, False])
        helper.blank_until_disconnect(d, lambda: next(heartbeats))
        self.assertEqual(len(d.calls), 8)
        self.assertEqual(d.calls[-2:], list(d.saved.items()))

    def test_termination_restores(self):
        d = FakeDisplay()
        def terminate():
            raise SystemExit(0)
        with self.assertRaises(SystemExit):
            helper.blank_until_disconnect(d, terminate)
        self.assertEqual(d.calls[-2:], list(d.saved.items()))


if __name__ == "__main__":
    unittest.main()
