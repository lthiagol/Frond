"""Private scratch root shared by the smoke scripts.

Fixtures used to live at fixed, guessable `/tmp/frond-*` paths: on a shared
machine two users collide there, and another user could pre-create the
directory. The root is now `$FROND_SMOKE_TMP`, else `<tmpdir>/frond-smoke-<uid>`
created mode 0700 and refused unless it is a real directory owned by us.
"""
import os
import shutil
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


TESTVAULT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "testvault")


def copy_testvault(name):
    """Copies the sample vault (testvault/, the repository's) to path(name),
    replacing an earlier copy, and returns that path. Mutate only the copy:
    the checked-in vault is never written. What the vault holds:
    docs/TESTVAULT.md.
    """
    dest = path(name)
    shutil.rmtree(dest, ignore_errors=True)
    # symlinks=True keeps the vault's file symlink a relative link inside the copy.
    shutil.copytree(TESTVAULT, dest, symlinks=True)
    return dest
