"""Own the mutable datafix root, independently of read-only vanilla inputs."""
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VARIABLE = 'HORIZON_DATAFIX_ROOT'
# The read-only shared copy of the vanilla data (tools/ci/vanilla-data.sh's HORIZON_SHARED_SOURCE).
READ_ONLY = Path(os.environ.get('HORIZON_SHARED_SOURCE') or Path.home() / 'code/horizon-parity/shared-local')


def writable_root():
    """Resolve and reject shared fixture destinations before creating anything."""
    configured = os.environ.get(VARIABLE)
    root = Path(configured).expanduser().resolve() if configured else (ROOT / '.local' / 'datafix').resolve()
    forbidden = READ_ONLY.resolve()
    problem = None
    if root.is_relative_to(forbidden) or forbidden.is_relative_to(root) or (not configured and not root.is_relative_to(ROOT.resolve())):
        problem = 'shared fixture inputs are read-only or the default escapes the checkout'
    elif root.exists() and not root.is_dir():
        problem = 'root is not a directory'
    else:
        ancestor = root
        while not ancestor.exists():
            ancestor = ancestor.parent
        if not ancestor.is_dir() or not os.access(ancestor, os.W_OK | os.X_OK):
            problem = f'ancestor {ancestor} is not writable'
        elif root.exists():
            # Validate existing descendants once at the owning boundary, before
            # consumers reuse the resolved root for downloads, locks and worlds.
            for path in root.rglob('*'):
                if path.is_symlink() and not path.resolve().is_relative_to(root):
                    problem = f'descendant {path} escapes the private root'
                    break
    if problem:
        raise ValueError(f'{VARIABLE} writable root resolves to {root}: {problem}. '
                         f'Set {VARIABLE} to a private writable directory without escaping symlinks.')
    return root
