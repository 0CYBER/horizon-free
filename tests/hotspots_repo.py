"""Throwaway git repositories for the tests of tools/hotspots/."""
import os
from pathlib import Path
import subprocess
import tempfile

START = 1_790_000_000


class Repo:
    def __init__(self):
        self._dir = tempfile.TemporaryDirectory()
        self.path = Path(self._dir.name)
        self.time = START
        self.git("init", "-q", "-b", "main")

    def close(self):
        self._dir.cleanup()

    def git(self, *args, ok=(0,)):
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.com", GIT_COMMITTER_NAME="t",
                   GIT_COMMITTER_EMAIL="t@example.com", GIT_AUTHOR_DATE=f"@{self.time} +0000",
                   GIT_COMMITTER_DATE=f"@{self.time} +0000", GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        p = subprocess.run(["git", "-C", str(self.path), *args], capture_output=True, text=True, env=env)
        if p.returncode not in ok:
            raise AssertionError(f"git {' '.join(args)}: {p.stderr}")
        return p.stdout.strip()

    def tick(self, seconds=3600):
        self.time += seconds

    def write(self, files):
        for name, text in files.items():
            path = self.path / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
            self.git("add", name)

    def commit(self, branch, files, message="change", base=None):
        """A commit with `files` on `branch` (created from `base` when given); returns its sha."""
        self.tick()
        if base:
            self.git("checkout", "-q", "-B", branch, base)
        elif self.git("symbolic-ref", "--short", "HEAD") != branch:
            self.git("checkout", "-q", branch)
        self.write(files)
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD")

    def merge(self, branch, other, message, resolve=None):
        """Merges `other` into `branch` (always a merge commit); `resolve` gives the files when it conflicts."""
        self.tick()
        self.git("checkout", "-q", branch)
        rc = subprocess.run(["git", "-C", str(self.path), "merge", "--no-ff", "-q", "-m", message, other],
                            capture_output=True, env=dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull,
                                                          GIT_COMMITTER_DATE=f"@{self.time} +0000",
                                                          GIT_AUTHOR_DATE=f"@{self.time} +0000",
                                                          GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.com",
                                                          GIT_COMMITTER_NAME="t",
                                                          GIT_COMMITTER_EMAIL="t@example.com")).returncode
        if rc:
            if resolve is None:
                raise AssertionError(f"merging {other} into {branch} conflicted")
            self.write(resolve)
            self.git("commit", "-q", "--no-edit")
        elif resolve is not None:
            raise AssertionError(f"merging {other} into {branch} was expected to conflict")
        return self.git("rev-parse", "HEAD")

    def pull(self, number, sha):
        self.git("update-ref", f"refs/pull/{number}/head", sha)
