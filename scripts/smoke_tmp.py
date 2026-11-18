"""Private scratch root shared by the smoke scripts.

Fixtures used to live at fixed, guessable `/tmp/frond-*` paths: on a shared
machine two users collide there, and another user could pre-create the
directory. The root is now `$FROND_SMOKE_TMP`, else `<tmpdir>/frond-smoke-<uid>`
created mode 0700 and refused unless it is a real directory owned by us.
"""
import os
import stat
import tempfile


def smoke_root():
    root = os.environ.get("FROND_SMOKE_TMP") or os.path.join(
        tempfile.gettempdir(), f"frond-smoke-{os.getuid()}"
    )
    os.makedirs(root, mode=0o700, exist_ok=True)
    st = os.lstat(root)
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.getuid() or st.st_mode & 0o077:
        raise SystemExit(f"refusing smoke scratch dir {root}: not a private dir we own")
    return root


SMOKE_ROOT = smoke_root()


def path(*parts):
    return os.path.join(SMOKE_ROOT, *parts)
