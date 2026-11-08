"""Opt-in native tests; temporarily blanks the current X11 display.
Run only intentionally: VEYNDESK_TEST_X11_BLANKING=1 python3 -m unittest discover -s crates/host/tests -v
"""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import time
import unittest

HELPER = Path(__file__).parents[1] / 'src' / 'privacy_helper.py'
spec = importlib.util.spec_from_file_location('privacy_native', HELPER)
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)


@unittest.skipUnless(os.environ.get('VEYNDESK_TEST_X11_BLANKING') == '1',
                     'requires explicit permission to blank a live X11 display')
class NativeTests(unittest.TestCase):
    def exercise(self, heartbeat_timeout):
        display = helper.X11()
        saved = display.snapshot()
        child = None
        try:
            child = subprocess.Popen([sys.executable, '-I', '-u', str(HELPER)],
                                     stdin=subprocess.PIPE)
            child.stdin.write(b'.\n')
            child.stdin.flush()
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                actual = display.snapshot()
                if all(v == 0 for channels in actual.values() for c in channels for v in c):
                    break
                time.sleep(0.05)
            else:
                self.fail('display gamma ramps did not become zero')
            if not heartbeat_timeout:
                child.stdin.close()
            # An open pipe with no heartbeat models a frozen host process.
            self.assertEqual(child.wait(timeout=8), 0)
            self.assertEqual(display.snapshot(), saved)
        finally:
            if child:
                if child.stdin and not child.stdin.closed:
                    child.stdin.close()
                if child.poll() is None:
                    child.terminate()
                    child.wait(timeout=8)
            for crtc, channels in saved.items():
                display.apply(crtc, channels)
            display.close()

    def test_disconnect(self):
        self.exercise(False)

    def test_frozen_host_heartbeat_timeout(self):
        self.exercise(True)
