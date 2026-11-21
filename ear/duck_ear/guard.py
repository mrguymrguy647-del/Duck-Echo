"""Nothing leaves the PC while the Ear listens.

A Python audit hook refuses every network connection that is not to this
PC itself (127.0.0.1 or ::1), and logs it. It is a guard, not a sandbox:
code outside Python could still connect. The other walls are in
docs/phase4-design.md, section 5 ("Nothing leaves your PC while
listening"), and a Linux CI step runs the Ear's tests with no network at all.
"""

import os
import sys

LOCAL = {"127.0.0.1", "::1", "localhost"}


def _host_of(address):
    if isinstance(address, tuple) and address:
        return str(address[0])
    return None  # a Unix socket path: this PC


def _hook(event, args):
    if event in ("socket.connect", "socket.connect_ex", "socket.sendto"):
        address = args[1] if len(args) > 1 else None
        host = _host_of(address)
        if host is not None and host not in LOCAL:
            print("duck-ear: refused a network connection to %s" % host, file=sys.stderr, flush=True)
            raise PermissionError("the Duck Ear does not use the network (it tried %s)" % host)
    elif event == "socket.getaddrinfo":
        host = args[0] if args else None
        if host not in (None, "") and str(host) not in LOCAL:
            print("duck-ear: refused to look up %s" % host, file=sys.stderr, flush=True)
            raise PermissionError("the Duck Ear does not use the network (it tried %s)" % host)


def refuse_the_network():
    """Install the guard, and tell libraries that look online not to."""
    for name in ("HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE", "HF_HUB_DISABLE_TELEMETRY", "UV_OFFLINE"):
        os.environ[name] = "1"
    sys.addaudithook(_hook)
