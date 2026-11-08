"""X11 display gamma blanking watchdog, embedded by privacy.rs.

The parent sends a heartbeat each second. EOF or five seconds without a
heartbeat restores the original gamma ramps, including after a parent crash.
"""
import ctypes as C
import os
import select
import signal
import sys


class Resources(C.Structure):
    _fields_ = [("timestamp", C.c_ulong), ("configTimestamp", C.c_ulong),
                ("ncrtc", C.c_int), ("crtcs", C.POINTER(C.c_ulong)),
                ("noutput", C.c_int), ("outputs", C.POINTER(C.c_ulong)),
                ("nmode", C.c_int), ("modes", C.c_void_p)]


class CrtcInfo(C.Structure):
    _fields_ = [("timestamp", C.c_ulong), ("x", C.c_int), ("y", C.c_int),
                ("width", C.c_uint), ("height", C.c_uint), ("mode", C.c_ulong),
                ("rotation", C.c_ushort), ("noutput", C.c_int),
                ("outputs", C.POINTER(C.c_ulong)), ("rotations", C.c_ushort),
                ("npossible", C.c_int), ("possible", C.POINTER(C.c_ulong))]


class Gamma(C.Structure):
    _fields_ = [("size", C.c_int), ("red", C.POINTER(C.c_ushort)),
                ("green", C.POINTER(C.c_ushort)), ("blue", C.POINTER(C.c_ushort))]


class X11:
    def __init__(self):
        self.x = C.CDLL("libX11.so.6")
        self.r = C.CDLL("libXrandr.so.2")
        self.x.XOpenDisplay.argtypes = [C.c_char_p]
        self.x.XOpenDisplay.restype = C.c_void_p
        self.x.XDefaultRootWindow.argtypes = [C.c_void_p]
        self.x.XDefaultRootWindow.restype = C.c_ulong
        self.x.XSync.argtypes = [C.c_void_p, C.c_int]
        self.x.XCloseDisplay.argtypes = [C.c_void_p]
        self.r.XRRGetScreenResourcesCurrent.argtypes = [C.c_void_p, C.c_ulong]
        self.r.XRRGetScreenResourcesCurrent.restype = C.POINTER(Resources)
        self.r.XRRFreeScreenResources.argtypes = [C.POINTER(Resources)]
        self.r.XRRGetCrtcInfo.argtypes = [C.c_void_p, C.POINTER(Resources), C.c_ulong]
        self.r.XRRGetCrtcInfo.restype = C.POINTER(CrtcInfo)
        self.r.XRRFreeCrtcInfo.argtypes = [C.POINTER(CrtcInfo)]
        self.r.XRRGetCrtcGamma.argtypes = [C.c_void_p, C.c_ulong]
        self.r.XRRGetCrtcGamma.restype = C.POINTER(Gamma)
        self.r.XRRSetCrtcGamma.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(Gamma)]
        self.r.XRRAllocGamma.argtypes = [C.c_int]
        self.r.XRRAllocGamma.restype = C.POINTER(Gamma)
        self.r.XRRFreeGamma.argtypes = [C.POINTER(Gamma)]
        self.error = None
        callback_type = C.CFUNCTYPE(C.c_int, C.c_void_p, C.c_void_p)
        def record_error(_display, _event):
            self.error = "X11 rejected the gamma operation"
            return 0
        self.error_handler = callback_type(record_error)
        self.x.XSetErrorHandler.argtypes = [callback_type]
        self.x.XSetErrorHandler(self.error_handler)
        self.display = self.x.XOpenDisplay(None)
        if not self.display:
            raise RuntimeError("cannot open the X11 display")

    def snapshot(self):
        resources = self.r.XRRGetScreenResourcesCurrent(
            self.display, self.x.XDefaultRootWindow(self.display))
        if not resources:
            raise RuntimeError("RandR display resources unavailable")
        saved = {}
        try:
            for i in range(resources.contents.ncrtc):
                crtc = resources.contents.crtcs[i]
                info = self.r.XRRGetCrtcInfo(self.display, resources, crtc)
                if not info:
                    raise RuntimeError("cannot read display configuration")
                try:
                    active = bool(info.contents.mode and info.contents.noutput)
                finally:
                    self.r.XRRFreeCrtcInfo(info)
                if not active:
                    continue
                gamma = self.r.XRRGetCrtcGamma(self.display, crtc)
                if not gamma:
                    raise RuntimeError("cannot read display gamma")
                try:
                    g = gamma.contents
                    if g.size > 0:
                        saved[crtc] = tuple(tuple(channel[j] for j in range(g.size))
                                            for channel in (g.red, g.green, g.blue))
                finally:
                    self.r.XRRFreeGamma(gamma)
        finally:
            self.r.XRRFreeScreenResources(resources)
        if not saved:
            raise RuntimeError("no displays with gamma support")
        return saved

    def apply(self, crtc, channels):
        # XRRSetCrtcGamma reads one contiguous RGB buffer starting at red.
        # Use XRRAllocGamma rather than three independently allocated arrays.
        gamma = self.r.XRRAllocGamma(len(channels[0]))
        if not gamma:
            raise MemoryError("cannot allocate gamma ramps")
        try:
            for target, source in zip((gamma.contents.red, gamma.contents.green,
                                       gamma.contents.blue), channels):
                for i, value in enumerate(source):
                    target[i] = value
            self.r.XRRSetCrtcGamma(self.display, crtc, gamma)
            self.x.XSync(self.display, 0)
            if self.error:
                error, self.error = self.error, None
                raise RuntimeError(error)
        finally:
            self.r.XRRFreeGamma(gamma)

    def close(self):
        self.x.XCloseDisplay(self.display)


def blank_until_disconnect(display, alive):
    saved = display.snapshot()
    try:
        while True:
            for crtc, channels in saved.items():
                display.apply(crtc, tuple((0,) * len(c) for c in channels))
            if not alive():
                break
    finally:
        # Attempt all restorations even if one output has disappeared.
        for crtc, channels in saved.items():
            try:
                display.apply(crtc, channels)
            except Exception as error:
                print(f"screen restoration failed: {error}", file=sys.stderr)


def main():
    if os.environ.get("XDG_SESSION_TYPE", "").lower() != "x11":
        raise RuntimeError("screen blanking requires an X11 session (Ubuntu on Xorg)")
    def terminate(_sig, _frame):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    display = X11()
    try:
        def alive():
            ready, _, _ = select.select([sys.stdin.buffer], [], [], 5)
            return bool(ready) and bool(os.read(sys.stdin.fileno(), 4096))
        blank_until_disconnect(display, alive)
    finally:
        display.close()


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"Veyndesk screen blanking unavailable: {error}", file=sys.stderr)
        sys.exit(1)
