#!/usr/bin/env python3
"""Register the real-client LAN suite with parity.py's traced harness selection.

The assertions live in lan_e2e; this is integration coverage, alongside the
fast vanilla differential probes. The host uses the normal server thread.
Linux runners need a display (for example xvfb-run -a).
"""
import lan_e2e
from integration_env import suite_lock


if __name__ == '__main__':
    with suite_lock():
        lan_e2e.main()
