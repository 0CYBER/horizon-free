"""Which jobs may run on our own runners (docs/ci-runners.md).

- horizon-light (playhorizon's and other small runners, which refuse a fork's pull request): main's own scripts only.
- horizon-checks (isolated runners; an approved fork's pull request ending in 0 runs there too): no secret and
  no token that writes.
- Neither takes a job that holds a secret or whose token can push, publish or start workflows: playhorizon is a
  partner's server, and a checks runner keeps what a fork's job left behind.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
LIGHT = ("horizon-light", "playhorizon-light", "playhorizon-ci")
CHECKS = ("horizon-checks",)
FORK_GUARD = "github.event.pull_request.head.repo.full_name == github.repository"
# verify-scope decides on a small runner which of the launcher and Rust parts start; it goes where they go.
VERIFY_PARTS = ("verify-scope", "verify-tree", "verify-lint", "verify-server", "verify-launcher", "verify-test",
                "verify-mod-api")
# On Windows the bash on PATH is Git's; a bare "bash" there would be WSL's (tests/test_pr_ready.py).
BASH = shutil.which("bash")
# verify's parts: our checks runners only with SELF_HOSTED_CI=all, Blacksmith's otherwise.
ONLY_WITH_ALL = re.compile(r"^\$\{\{ vars\.SELF_HOSTED_CI == 'all' && fromJSON\('\[\"self-hosted\", \"linux\", "
                           r"\"horizon-checks\"\]'\) \|\| 'blacksmith-\d+vcpu-ubuntu-2404' \}\}$")
TRUSTED_WRITES = re.compile(r"^\s*(contents|actions|packages|id-token|deployments|attestations): write", re.M)
# actions: write can start workflows too; cancel-on-close needs it to cancel runs and runs no checkout at all.
CANCELS_RUNS = {"cancel-on-close.yml cancel"}


def jobs(text):
    """(name, block) for each job of a workflow, by indentation (two spaces under `jobs:`)."""
    body = text.split("\njobs:\n", 1)[1]
    parts = re.split(r"^  ([A-Za-z0-9_-]+):\n", body, flags=re.M)
    return list(zip(parts[1::2], parts[2::2]))


def triggers(text):
    head = text.split("\njobs:\n", 1)[0]
    on = re.search(r"^on:\n((?:[ #].*\n|\n)*)", head, re.M)
    return set(re.findall(r"^  ([a-z_]+):", on.group(1), re.M)) if on else set()


def runs_on(block):
    """The job's runs-on value, a folded (`>-`) one included."""
    lines = block.splitlines()
    for i, line in enumerate(lines):
        if line.strip().startswith("runs-on:"):
            value = line.split("runs-on:", 1)[1].strip()
            if value in (">-", ">", "|"):
                indent = len(line) - len(line.lstrip()) + 2
                # The folded lines right under it.
                folded = []
                for l in lines[i + 1:]:
                    if not l.startswith(" " * indent):
                        break
                    folded.append(l.strip())
                value = " ".join(folded)
            return value
    return ""


def routed():
    """(where, triggers, workflow text, job block, runs-on, pools) for every job that can run on our runners."""
    for path in sorted(WORKFLOWS.glob("*.yml")):
        text = path.read_text()
        on = triggers(text)
        for name, block in jobs(text):
            value = runs_on(block)
            pools = {pool for pool, labels in (("light", LIGHT), ("checks", CHECKS))
                     if any(f'"{label}"' in value for label in labels)}
            if pools:
                yield f"{path.name} {name}", on, text, block, value, pools


class OwnRunnerTests(unittest.TestCase):
    def test_every_job_on_our_runners_can_go_back_to_github(self):
        count = 0
        for where, _, _, _, value, pools in routed():
            count += 1
            if ONLY_WITH_ALL.match(value):
                continue
            self.assertIn("vars.SELF_HOSTED_CI != 'off'", value, where)
            # Both labels select a GitHub-hosted or Blacksmith Ubuntu runner when our pools are disabled.
            self.assertRegex(value, r"\|\| '(?:ubuntu-(?:latest|24\.04)|blacksmith-\d+vcpu-ubuntu-2404)' \}\}$", where)
            if "checks" in pools:
                # SELF_HOSTED_CI=light: every checks job back to GitHub, the bots stay.
                self.assertIn("vars.SELF_HOSTED_CI != 'light'", value, where)
        self.assertGreaterEqual(count, 10)

    def test_light_runners_take_main_code_only(self):
        for where, on, _, block, _, pools in routed():
            if "light" not in pools:
                continue
            if "pull_request" in on:
                job_if = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                self.assertIn(FORK_GUARD, job_if, f"{where}: a fork's pull request")
            if "pull_request_target" in on:
                # main's code only: every checkout is main's.
                self.assertEqual(block.count("uses: actions/checkout"), block.count("ref: main"), where)

    def test_no_secret_or_writing_token_on_our_runners(self):
        for where, _, text, block, _, pools in routed():
            self.assertNotIn("secrets.", block, where)
            self.assertNotIn("environment:", block, where)
            head = text.split("\njobs:\n", 1)[0]
            for part in (block, head):
                writes = {m.group(1) for m in TRUSTED_WRITES.finditer(part)}
                if where in CANCELS_RUNS:
                    writes -= {"actions"}
                    self.assertNotIn("actions/checkout", block, where)
                self.assertEqual(writes, set(), where)
            if "checks" in pools:
                # A checks runner keeps what a fork's job left: its jobs' tokens write nothing at all.
                self.assertNotRegex(block, r":\s*write", where)
                self.assertNotRegex(head, r":\s*write", where)

    def test_verify_parts_go_to_one_pool(self):
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        # Every part goes to our checks runners or every part to Blacksmith's (only the size differs); verify itself
        # only waits for them.
        for part in VERIFY_PARTS:
            with self.subTest(part=part):
                self.assertRegex(runs_on(ci[part]), ONLY_WITH_ALL)
        self.assertIn(f"needs: [{', '.join(VERIFY_PARTS)}]", ci["verify"])
        # Never skipped: GitHub counts a skipped required check as passed.
        self.assertIn("if: always()", ci["verify"])

    def test_forks_reach_only_the_checks_runners(self):
        # An approved fork's pull request ending in 0 runs on horizon-checks like this repository's own (GitHub's
        # 20-job pool is not the only place it may wait). Never the light runners: every light job that a pull request
        # can start carries the fork guard (test_light_runners_take_main_code_only), and ci.yml's two runs-on
        # expressions (the other workflows keep their own guards) no longer test the head repository.
        ci = dict(jobs((WORKFLOWS / "ci.yml").read_text()))
        for name in VERIFY_PARTS:
            self.assertNotIn("head.repo", runs_on(ci[name]), name)
        for where, on, _, block, _, pools in routed():
            if "light" in pools and "pull_request" in on:
                job_if = next((l for l in block.splitlines() if l.strip().startswith("if:")), "")
                self.assertIn(FORK_GUARD, job_if, where)

    def test_only_the_checks_runners_take_forks(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn('[ "${HORIZON_RUNNER_POOL:-}" != checks ]', script)
        self.assertIn('echo "HORIZON_RUNNER_POOL=checks"', script)
        # Only inside the branch of the runner that has the rust toolchain (the checks runner), never the light ones.
        env = script.split('echo "HORIZON_RUNNER_POOL=checks"')[0].rsplit("if [", 1)[1]
        self.assertTrue(env.startswith(' "$label" = playhorizon-ci'), env)

    def test_the_light_runners_refuse_forks_themselves(self):
        script = (ROOT / "tools/ci/checks-runners.sh").read_text()
        self.assertIn("ACTIONS_RUNNER_HOOK_JOB_STARTED=$shared/job-started.sh", script)
        self.assertIn('.pull_request.head.repo.full_name // "unknown"', script)
        self.assertIn('if [ "$head" != "${GITHUB_REPOSITORY:-}" ]', script)  # refuses unless HORIZON_RUNNER_POOL=checks


class HostedParityTests(unittest.TestCase):
    """parity-hosted.yml fetches the maintainers' encrypted vanilla data onto Blacksmith's runners. The secrets reach
    only the default branch's fetch and seal scripts, downloaded inside the step that holds them and run in a fixed
    environment, never beside the tested commit's code but on main. A fork's pull request can restore this
    repository's cache entries, so only main's runs write them, and nothing made from the data goes into one, an
    artifact or a log unsealed: the dataset's encrypted chunks are the only clear entry."""

    SECRET_STEPS = ["Fetch the vanilla data", "Unseal the restored caches", "Seal for the caches"]
    SHELL = "/usr/bin/bash --noprofile --norc -e -o pipefail {0}"

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
        import yaml
        cls.text = (WORKFLOWS / "parity-hosted.yml").read_text()
        cls.workflow = yaml.safe_load(cls.text)
        cls.plan = cls.workflow["jobs"]["plan"]["steps"]
        cls.steps = cls.workflow["jobs"]["run"]["steps"]
        cls.names = [step.get("name") or step.get("uses") for step in cls.steps]

    def step(self, name, steps=None):
        steps = self.steps if steps is None else steps
        return next(step for step in steps if (step.get("name") or step.get("uses")) == name)

    def prologue(self, name):
        """A secret step's fixed environment: its script up to the first download."""
        return self.step(name)["run"].split("\nfetched ", 1)[0]

    def test_secret_values_reach_only_the_trusted_scripts(self):
        holders = []
        for job, body in self.workflow["jobs"].items():
            self.assertNotIn("secrets.", repr({k: v for k, v in body.items() if k != "steps"}), job)
            for step in body["steps"]:
                if "secrets." in repr(step):
                    holders.append((job, step.get("name"), step.get("env", {})))
        values = [(job, name) for job, name, env in holders if any(
            re.fullmatch(r"\$\{\{ secrets\.\w+ \}\}", str(value)) for value in env.values())]
        self.assertEqual(values, [("run", name) for name in self.SECRET_STEPS])
        # The plan learns only whether they are set.
        others = [(job, name) for job, name, _ in holders if (job, name) not in values]
        self.assertEqual(others, [("plan", "Access to the vanilla data")])
        # Each runs only a script it downloaded itself from the trusted commit, never one of the checkout.
        for name in self.SECRET_STEPS:
            run = self.step(name)["run"]
            self.assertNotRegex(run, r"\b(python3?|bash) +(\./)?tools/|\$GITHUB_WORKSPACE/tools/", name)
            fetched = re.findall(r"^fetched (\S+) \|\| \{$", run, re.M)
            self.assertTrue(fetched, name)
            self.assertEqual(set(re.findall(r'"\$private/([a-z.-]+)"', run.split("\nfetched ", 1)[1])), set(fetched),
                             name)

    def test_the_secrets_meet_no_tested_code_but_mains(self):
        # Before the checkout the workspace is empty: no code of the tested commit can run beside the secrets.
        checkout = next(index for index, name in enumerate(self.names) if name.startswith("actions/checkout@"))
        self.assertLess(self.names.index("Fetch the vanilla data"), checkout)
        self.assertLess(self.names.index("Unseal the restored caches"), checkout)
        # Sealing comes after the harnesses: on main only, whose code it is.
        self.assertGreater(self.names.index("Seal for the caches"), self.names.index("The shard's harnesses"))
        self.assertIn("needs.plan.outputs.saves == 'true'", self.step("Seal for the caches")["if"])
        self.assertEqual(set(self.workflow[True]), {"workflow_dispatch"})
        # The default branch's commit, resolved in the plan job, where no cache entry is restored.
        trusted = self.step("The commit of the trusted scripts", self.plan)
        self.assertEqual(trusted["env"]["DEFAULT"], "${{ github.event.repository.default_branch }}")
        self.assertEqual(self.workflow["jobs"]["plan"]["outputs"]["trusted"], "${{ steps.trusted.outputs.sha }}")
        for name in self.SECRET_STEPS:
            self.assertEqual(self.step(name)["env"]["TRUSTED_SHA"], "${{ needs.plan.outputs.trusted }}", name)
        # The oracle shim's runtime memo (machine-local stat data) and this run's logs are excluded wherever the
        # oracle entry is sealed or unsealed; tests/test_ci_sealed_cache.py checks the exclusion itself.
        folders = self.step("Folders for the data, outside the checkout")["run"]
        self.assertIn('echo "RUNTIME_MEMO=$GITHUB_WORKSPACE/.local/parity-runner/runtime-memo"', folders)
        self.assertIn('echo "PARITY_LOGS=$GITHUB_WORKSPACE/.local/parity-runner/logs"', folders)
        for name in ("Unseal the restored caches", "Seal for the caches"):
            self.assertIn('--exclude "$RUNTIME_MEMO" --exclude "$PARITY_LOGS"', self.step(name)["run"], name)

    def test_a_secret_step_trusts_nothing_a_cache_entry_could_have_written(self):
        # actions/cache unpacks an entry with absolute paths before anything checks it: each secret step comes after
        # every restore it depends on and downloads its script itself; the step's shell is root's, not one on PATH.
        restores = [index for index, step in enumerate(self.steps)
                    if step.get("uses", "").startswith("actions/cache/restore@")]
        self.assertLess(restores[-1], self.names.index("Unseal the restored caches"))
        for name in self.SECRET_STEPS:
            step = self.step(name)
            self.assertEqual(step["shell"], self.SHELL, name)
            self.assertEqual((step["env"]["BASH_ENV"], step["env"]["ENV"]), ("", ""), name)
            # One fixed environment, the same in all three.
            self.assertEqual(self.prologue(name), self.prologue(self.SECRET_STEPS[0]), name)
        prologue = self.prologue(self.SECRET_STEPS[0])
        self.assertIn("export PATH=/usr/bin:/bin", prologue)
        self.assertIn('curl -q -fsS --proto =https', prologue)
        self.assertIn("contents/tools/ci/$1?ref=$TRUSTED_SHA", prologue)
        # The trusted commit is the default branch's, named by its explicit ref.
        trusted = self.step("The commit of the trusted scripts", self.plan)["run"]
        self.assertIn('git/ref/heads/$DEFAULT', trusted)
        self.assertNotIn("/commits/", trusted)
        # The token reaches curl on its stdin, never its command line.
        self.assertNotRegex(prologue, r'curl[^|]*\$GH_TOKEN')

    LINUX_STEP = (sys.platform == "linux" and os.access("/usr/bin/bash", os.X_OK)
                  and not os.access("/usr/bin", os.W_OK), "needs Linux, with /usr/bin not writable")

    def run_secret_step(self, script, folder):
        env = {"PATH": "/usr/bin:/bin", "HOME": str(folder), "TRUSTED_SHA": "0" * 40, "GH_TOKEN": "token",
               "GITHUB_REPOSITORY": "owner/repo", "GITHUB_WORKSPACE": str(folder), "RUNNER_TEMP": str(folder),
               "GITHUB_OUTPUT": str(folder / "output"), "BASH_ENV": "", "ENV": ""}
        return subprocess.run([*self.SHELL.replace("{0}", "").split(), "-c", script], env=env, capture_output=True,
                              text=True)

    @unittest.skipUnless(*LINUX_STEP)
    def test_a_downloaded_script_runs_only_as_the_blob_the_api_names(self):
        # verified, from the steps' own prologue: not empty, and git's blob id of the bytes equals the API's.
        def blob(data):
            return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()

        script = b"#!/usr/bin/env bash\necho fetched\n"
        cases = [(script, blob(script), 0), (script + b" ", blob(script), 1), (script, blob(script).upper(), 1),
                 (script, blob(script)[:39], 1), (script, "", 1), (b"", blob(b""), 1),
                 (b"\0" * 70000, blob(b"\0" * 70000), 0)]
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            for data, expected, status in cases:
                with self.subTest(size=len(data), expected=expected):
                    (folder / "script.sh").write_bytes(data)
                    result = self.run_secret_step(self.prologue("Fetch the vanilla data")
                                                  + f'\nverified "{folder}/script.sh" "{expected}"\n', folder)
                    self.assertEqual(result.returncode, status, result.stderr)

    @unittest.skipUnless(*LINUX_STEP)
    def test_a_script_is_fetched_only_as_the_blob_its_listing_names(self):
        # The steps' own fetched, with only the two API requests answered from files: the listing's sha and the raw
        # bytes must agree, and the script lands in the private folder only then.
        script = b"#!/usr/bin/env bash\necho fetched\n"
        blob = hashlib.sha1(b"blob %d\0" % len(script) + script).hexdigest()
        empty = hashlib.sha1(b"blob 0\0").hexdigest()
        cases = [("matching", json.dumps({"sha": blob, "type": "file"}), script, 0),
                 ("another sha", json.dumps({"sha": "1" * 40}), script, 1),
                 ("not an object", "[]", script, 1),
                 ("no listing", "", script, 1),
                 ("empty bytes", json.dumps({"sha": empty}), b"", 1)]
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            for case, listing, data, status in cases:
                with self.subTest(case):
                    (folder / "listing.json").write_text(listing)
                    (folder / "raw").write_bytes(data)
                    stub = (f'api() {{ case "$1" in *+json) cp {folder}/listing.json "$3" ;; '
                            f'*.raw) cp {folder}/raw "$3" ;; esac; }}')
                    result = self.run_secret_step(self.prologue("Fetch the vanilla data") + f"\n{stub}\n"
                                                  'fetched script.sh && cmp -s "$private/script.sh" '
                                                  f'{folder}/raw\n', folder)
                    self.assertEqual(result.returncode, status, result.stderr)

    @unittest.skipUnless(*LINUX_STEP)
    def test_a_script_that_does_not_verify_stops_the_fetch_and_skips_the_seal_steps(self):
        # Fetching fails closed (no data); unsealing and sealing go on without the cache. Nothing runs either way.
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            for name, status, line in [("Fetch the vanilla data", 1, "::error::"),
                                       ("Unseal the restored caches", 0, "::warning::"),
                                       ("Seal for the caches", 0, "::warning::")]:
                with self.subTest(name):
                    marker = folder / "ran"
                    prologue, rest = self.step(name)["run"].split("\nfetched ", 1)
                    script = (prologue + f"\nfetched() {{ return 1; }}\nisolated() {{ touch {marker}; }}\n"
                              + "fetched " + rest)
                    result = self.run_secret_step(script, folder)
                    self.assertEqual(result.returncode, status, result.stderr)
                    self.assertIn(line, result.stdout)
                    self.assertFalse(marker.exists())

    @unittest.skipUnless(sys.platform == "linux" and os.access("/usr/bin/bash", os.X_OK)
                         and not os.access("/usr/bin", os.W_OK), "needs Linux, with /usr/bin not writable")
    def test_the_trusted_script_runs_in_a_fixed_environment(self):
        # The step's own prologue around a stand-in script, under an environment a hostile entry could have shaped:
        # a bash and a curl first on PATH, BASH_ENV and ENV, a HOME with a .bashrc and .gitconfig.
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            hostile = folder / "hostile"
            (hostile / "home").mkdir(parents=True)
            marker = folder / "marker"
            for name in ("bash", "env", "curl"):
                (hostile / name).write_text(f"#!/bin/sh\ntouch {marker}\nexec /usr/bin/{name} \"$@\"\n")
                (hostile / name).chmod(0o755)
            (hostile / "rc").write_text(f"touch {marker}\n")
            (hostile / "home/.bashrc").write_text(f"touch {marker}\n")
            (hostile / "home/.gitconfig").write_text("[core]\n\thooksPath = /tmp\n")
            probe = folder / "probe.sh"
            probe.write_text('printf "%s\\n" "$@" > "$OUT.args"\n'
                             'printf "%s" "$VANILLA_DATA_DEPLOY_KEY" > "$OUT.key"\n'
                             'env -0 > "$OUT.env"\nls -A "$HOME" > "$OUT.home"\n')
            out = folder / "out"
            key = "-----BEGIN KEY-----\nline two\n-----END KEY-----\n"
            script = (self.prologue("Fetch the vanilla data")
                      + f'\nisolated "VANILLA_DATA_DEPLOY_KEY VANILLA_DATA_PASSPHRASE" OUT={out} -- {probe} one "two words"\n')
            env = {"PATH": f"{hostile}:/usr/bin:/bin", "HOME": str(hostile / "home"), "BASH_ENV": str(hostile / "rc"),
                   "ENV": str(hostile / "rc"), "TRUSTED_SHA": "0" * 40, "GH_TOKEN": "token",
                   "GITHUB_WORKSPACE": str(folder), "RUNNER_TEMP": str(folder), "GITHUB_OUTPUT": str(folder / "output"),
                   "VANILLA_DATA_DEPLOY_KEY": key, "VANILLA_DATA_PASSPHRASE": "pass phrase", "UNRELATED": "x"}
            # The step's own literal values override what earlier steps left (BASH_ENV and ENV: empty).
            env.update({name: value for name, value in self.step("Fetch the vanilla data")["env"].items()
                        if "${{" not in str(value)})
            # As the runner starts the step, with the shell named in it.
            shell = self.SHELL.replace("{0}", "").split()
            result = subprocess.run([*shell, "-c", script], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            names = {entry.split("=", 1)[0] for entry in Path(f"{out}.env").read_text().split("\0") if entry}
            # Only what the script needs (bash adds PWD, SHLVL and _ itself).
            self.assertEqual(names - {"PWD", "OLDPWD", "SHLVL", "_"}, {"PATH", "HOME", "GITHUB_WORKSPACE", "RUNNER_TEMP",
                                                             "GITHUB_OUTPUT", "OUT", "VANILLA_DATA_DEPLOY_KEY",
                                                             "VANILLA_DATA_PASSPHRASE"})
            self.assertIn("PATH=/usr/bin:/bin", Path(f"{out}.env").read_text().split("\0"))
            self.assertEqual(Path(f"{out}.key").read_text(), key)
            self.assertEqual(Path(f"{out}.args").read_text(), "one\ntwo words\n")
            self.assertEqual(Path(f"{out}.home").read_text(), "")
            self.assertFalse(marker.exists())

    def test_the_plans_key_values_are_checked_before_they_name_anything(self):
        # They come from the tested tree (the plan job): the step's own check, run with good and bad values.
        script = self.step("Cache keys")["run"].split("\nh() {", 1)[0]
        good = {"GROUP": "game+client", "JAVA_MAJOR": "25", "LOCK": "0123456789ab", "PREPARE": "0123456789abcdef"}
        bad = [("GROUP", "game/../x"), ("GROUP", ""), ("JAVA_MAJOR", "25 x"), ("LOCK", "0123456789aB"),
               ("PREPARE", "0123456789abcdef0"), ("PREPARE", "../../etc")]
        def run(values):
            return subprocess.run([BASH, "-c", script], env={**os.environ, **values}, capture_output=True).returncode
        self.assertEqual(run(good), 0)
        for name, value in bad:
            with self.subTest(name=name, value=value):
                self.assertNotEqual(run({**good, name: value}), 0)

    @unittest.skipUnless(BASH, "needs bash")
    def test_each_sealed_entry_is_restored_sealed_and_saved_under_one_key(self):
        # The key sealed into an entry is the one it is saved under; a restore takes it by that key or its prefix, and
        # unsealing accepts only an entry sealed for a key with that prefix.
        seal, unseal = self.step("Seal for the caches")["env"], self.step("Unseal the restored caches")["env"]
        for label, restore, save in (("prepared", "Restore the prepared resources (sealed)",
                                      "Save the prepared resources (sealed)"),
                                     ("oracle", "Restore the oracle results and parity state (sealed)",
                                      "Save the oracle results and parity state (sealed)"),
                                     ("build", "Restore the build folders (sealed)",
                                      "Save the build folders (sealed)")):
            with self.subTest(label):
                key, prefix = ("${{ steps.entries.outputs.%s }}" % name for name in (label, label + "_prefix"))
                restored = self.step(restore)["with"]
                self.assertEqual((restored["key"], restored["path"]), (key, "${{ env.SEALED }}/" + label))
                self.assertEqual(self.step(save)["with"]["key"], key)
                self.assertEqual(seal[label.upper()], key)
                if label == "prepared":
                    self.assertNotIn("restore-keys", restored)
                    self.assertEqual(unseal["PREPARED"], key)
                else:
                    self.assertEqual(restored["restore-keys"], prefix)
                    self.assertEqual(unseal[label.upper() + "_PREFIX"], prefix)
        # Each full key begins with its prefix.
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / "output"
            env = {**os.environ, "DIGEST": "0123456789abcdef", "BUILD": "parity-build-v2-other-r-l-",
                   "ORACLE": "j-r-f-other-", "PREPARE": "fedcba9876543210", "COMMIT": "c" * 40, "SHARD": "2",
                   "GITHUB_OUTPUT": str(output)}
            subprocess.run([BASH, "-c", self.step("Keys of the sealed entries")["run"]], env=env, check=True)
            keys = dict(line.split("=", 1) for line in output.read_text().splitlines())
        self.assertEqual(keys["oracle"], keys["oracle_prefix"] + "c" * 40 + "-2")
        self.assertEqual(keys["build"], keys["build_prefix"] + "c" * 40)

    @unittest.skipUnless(BASH, "needs bash")
    def test_only_mains_runs_save(self):
        # Compared in the shell: an expression's == ignores case, so a branch `Main` would pass it.
        self.assertEqual(self.workflow["jobs"]["plan"]["outputs"]["saves"], "${{ steps.gate.outputs.saves }}")
        gate = self.step("Whether this run saves the caches", self.plan)
        checkout = next(index for index, step in enumerate(self.plan)
                        if step.get("uses", "").startswith("actions/checkout@"))
        self.assertLess(self.plan.index(gate), checkout)
        # Nowhere else is the ref or the event compared in an expression.
        self.assertNotRegex(self.text, r"github\.(ref|ref_name|head_ref|base_ref|event_name)\s*(==|!=)"
                                       r"|startsWith\(github\.ref")
        main = {"REF": "refs/heads/main", "EVENT": "workflow_dispatch", "DEFAULT": "main", "PROTECTED": "true"}
        cases = [({}, "true"), ({"EVENT": "push"}, "true"), ({"EVENT": "schedule"}, "true"),
                 ({"REF": "refs/heads/Main"}, "false"), ({"REF": "refs/heads/MAIN"}, "false"),
                 ({"REF": "refs/heads/main/x"}, "false"), ({"REF": "refs/tags/main"}, "false"),
                 ({"REF": "refs/heads/feature"}, "false"), ({"DEFAULT": "Main"}, "false"),
                 ({"DEFAULT": "trunk"}, "false"), ({"PROTECTED": "false"}, "false"),
                 ({"EVENT": "pull_request_target"}, "false"), ({"EVENT": "Workflow_dispatch"}, "false"),
                 ({"EVENT": "pull_request"}, "false")]
        for change, expected in cases:
            with self.subTest(**change), tempfile.TemporaryDirectory() as folder:
                output = Path(folder) / "output"
                env = {**os.environ, **main, **change, "GITHUB_OUTPUT": str(output)}
                subprocess.run([BASH, "-c", gate["run"]], env=env, check=True)
                self.assertEqual(output.read_text(), f"saves={expected}\n")
        for name in ("Seal for the caches", "Save the encrypted dataset"):
            self.assertIn("needs.plan.outputs.saves == 'true'", self.step(name)["if"], name)
        saves = [step for step in self.steps if step.get("uses", "").startswith("actions/cache/save@")]
        self.assertEqual(len(saves), 4)
        for step in saves:
            condition = step["if"]
            self.assertTrue("needs.plan.outputs.saves == 'true'" in condition or "steps.seal.outputs." in condition,
                            step["name"])
        # No action that restores and saves at once, no sticky disk, no compiler cache.
        for body in self.workflow["jobs"].values():
            for step in body["steps"]:
                uses = step.get("uses", "")
                self.assertFalse(re.search(r"^actions/cache@|sticky|useblacksmith/|rust-cache", uses), uses)
        self.assertEqual(self.workflow["jobs"]["run"]["env"]["HORIZON_NO_SCCACHE"], "1")

    def test_the_harnesses_see_nothing_that_differs_between_a_saving_and_a_reading_run(self):
        # The oracle shim keys every variable the harnesses see but those it ignores (tools/parity_shim.py): one that
        # differs between main's saving run and a reading run (whether it saves, the event, the ref, a pull request's
        # number) would make every reading run miss main's records.
        sys.path.insert(0, str(ROOT / "tools"))
        import parity_shim
        harnesses = self.names.index("The shard's harnesses")
        seen = {**self.workflow["env"], **self.workflow["jobs"]["run"]["env"], **self.steps[harnesses].get("env", {})}
        self.assertNotIn("SAVE_CACHES", seen)
        for name, value in seen.items():
            if name not in parity_shim.IGNORED and not name.startswith(parity_shim.IGNORED_PREFIXES):
                self.assertNotIn("${{", str(value), name)
        # Nor does a step before them write one for the steps after it.
        for step in self.steps[:harnesses]:
            for line in step.get("run", "").splitlines():
                if "GITHUB_ENV" in line:
                    self.assertNotRegex(line, r"\$\{\{|SAVE", step.get("name"))

    def test_nothing_made_from_the_data_is_cached_in_the_clear(self):
        # The dataset's chunks are encrypted as they come; everything else (the build folders included: a build
        # script could copy data into them, and the harnesses run what they hold) only sealed.
        for step in self.steps:
            if step.get("uses", "").startswith("actions/cache/"):
                paths = {line.strip() for line in step["with"]["path"].splitlines() if line.strip()}
                self.assertTrue(paths == {"${{ env.VANILLA_DATA_CACHE }}"}
                                or all(path.startswith("${{ env.SEALED }}/") for path in paths), step["name"])
        uploads = [step["with"]["path"] for step in self.steps if "upload-artifact" in step.get("uses", "")]
        self.assertEqual(uploads, ["${{ runner.temp }}/parity-hosted-logs/"])

    def test_a_damaged_hash_database_is_left_to_parity_py(self):
        # An entry cut short or damaged is a cold start for that part, never a red job. The step only creates a
        # missing database; tools/parity.py removes a damaged one before the first shim starts.
        step = self.step("The oracle cache, created before the harnesses start")
        program = re.search(r"<<'EOF'\n(.*)\nEOF", step["run"], re.S).group(1)
        with tempfile.TemporaryDirectory() as folder:
            cache = Path(folder) / "oracle-cache"
            for damaged in (False, True):
                if damaged:
                    for path in cache.iterdir():
                        path.unlink()
                    (cache / "hashes.sqlite").write_bytes(b"not a database" * 100)
                result = subprocess.run([sys.executable, "-c", program], cwd=ROOT, capture_output=True, text=True,
                                        env={**os.environ, "PARITY_SHIM_CACHE": str(cache)})
                self.assertEqual(result.returncode, 0, result.stderr)
                header = (cache / "hashes.sqlite").read_bytes()[:15]
                self.assertTrue(header.startswith(b"not a database" if damaged else b"SQLite format 3"), header)

    def test_only_the_hosted_run_may_remove_a_damaged_hash_database(self):
        # Its fresh runner has one parity run, which owns the oracle cache; elsewhere another run in the checkout may
        # have the database open, so tools/parity.py only names a damaged one.
        self.assertEqual(self.workflow["jobs"]["run"]["env"]["PARITY_REPAIR_ORACLE_HASHES"], "1")
        for name in sorted(WORKFLOWS.glob("*.yml")):
            if name.name != "parity-hosted.yml":
                self.assertNotIn("PARITY_REPAIR_ORACLE_HASHES", name.read_text(encoding="utf-8"), name.name)

    def test_no_required_check_waits_for_it(self):
        for path in sorted(WORKFLOWS.glob("*.yml")):
            if path.name != "parity-hosted.yml":
                self.assertNotIn("parity-hosted", path.read_text(), path.name)
        verify = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["verify"]
        self.assertNotIn("hosted", verify)


class VerifyGateTests(unittest.TestCase):
    """verify, the check main requires, when verify-scope leaves a part out (#18843): a part left out passes, and one
    that failed, was cancelled or was never decided for fails."""

    PASSED = {"SCOPE": "success", "TREE": "success", "LINT": "success", "SERVER": "success", "LAUNCHER": "success",
              "TEST": "success", "MODAPI": "success", "EVENT": "pull_request", "RUST_ANSWER": "true",
              "LAUNCHER_ANSWER": "true", "MODAPI_ANSWER": "true"}
    # Each part verify-scope can leave out: its result's variable, its job, the answer that decides it, that answer's
    # output, and whether the merge queue leaves the part out too.
    GATED = [("LINT", "verify-lint", "RUST_ANSWER", "rust", True),
             ("SERVER", "verify-server", "RUST_ANSWER", "rust", True),
             ("LAUNCHER", "verify-launcher", "LAUNCHER_ANSWER", "launcher", True),
             ("TEST", "verify-test", "RUST_ANSWER", "rust", False),
             ("MODAPI", "verify-mod-api", "MODAPI_ANSWER", "mod_api", False)]

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(ROOT / "tools/ci/vendor/pyyaml-6.0.3.zip"))
        import yaml
        cls.jobs = yaml.safe_load((WORKFLOWS / "ci.yml").read_text())["jobs"]
        (cls.gate,) = cls.jobs["verify"]["steps"]
        (cls.answers,) = [step for step in cls.jobs["verify-scope"]["steps"] if step.get("id") == "answers"]

    def bash(self, script, env):
        if not BASH:
            self.skipTest("no bash")
        # As GitHub runs a `run:` step: bash -e with pipefail.
        return subprocess.run([BASH, "--noprofile", "--norc", "-eo", "pipefail", "-c", script],
                              env={**os.environ, **env}, capture_output=True, text=True)

    def passes(self, **results):
        return self.bash(self.gate["run"], {**self.PASSED, **results}).returncode == 0

    def test_the_gate_reads_every_part_and_always_runs(self):
        self.assertEqual(self.jobs["verify"]["needs"], list(VERIFY_PARTS))
        # Without always() a failed or skipped part would skip verify, which GitHub counts as passed.
        self.assertEqual(self.jobs["verify"]["if"], "always()")
        for name, part in [("SCOPE", "verify-scope"), ("TREE", "verify-tree"), ("LINT", "verify-lint"),
                           ("SERVER", "verify-server"), ("LAUNCHER", "verify-launcher"), ("TEST", "verify-test"),
                           ("MODAPI", "verify-mod-api")]:
            self.assertEqual(self.gate["env"][name], "${{ needs.%s.result }}" % part)
        self.assertEqual(self.gate["env"]["EVENT"], "${{ github.event_name }}")
        self.assertEqual(self.gate["env"]["RUST_ANSWER"], "${{ needs.verify-scope.outputs.rust }}")
        self.assertEqual(self.gate["env"]["LAUNCHER_ANSWER"], "${{ needs.verify-scope.outputs.launcher }}")
        self.assertEqual(self.gate["env"]["MODAPI_ANSWER"], "${{ needs.verify-scope.outputs.mod_api }}")
        self.assertTrue(self.passes())

    def test_the_parts_wait_for_the_checked_answers(self):
        for _, part, _, output, queue_skips in self.GATED:
            with self.subTest(part=part):
                self.assertEqual(self.jobs[part]["needs"], "verify-scope")
                condition = "needs.verify-scope.outputs.%s != 'false'" % output
                if queue_skips:
                    condition = "github.event_name != 'merge_group' && " + condition
                self.assertEqual(self.jobs[part]["if"], condition)
                # Decided once, in verify-scope: no step of the part asks again.
                self.assertNotIn("rust_changes.py", json.dumps(self.jobs[part]))
        # Decided on every event, and published only through the step that checks them.
        self.assertNotIn("if", self.jobs["verify-scope"])
        self.assertNotIn("if", self.answers)
        self.assertEqual(self.jobs["verify-scope"]["outputs"], {
            "rust": "${{ steps.answers.outputs.rust }}", "launcher": "${{ steps.answers.outputs.launcher }}",
            "mod_api": "${{ steps.answers.outputs.mod_api }}"})
        self.assertEqual(self.answers["env"], {"RUST": "${{ steps.rust.outputs.required }}",
                                               "LAUNCHER": "${{ steps.launcher.outputs.required }}",
                                               "MOD_API": "${{ steps.rust.outputs.mod_api }}"})

    def test_the_decision_is_the_one_the_parts_made_themselves(self):
        # The selector's default answer, which the server build, Rust tests and mod API parts each asked on a pull
        # request: `required` is needs_rust, `mod_api` needs_mod_api with its search of the changed files.
        (rust,) = [step for step in self.jobs["verify-scope"]["steps"] if step.get("id") == "rust"]
        self.assertEqual(rust["if"], "github.event_name == 'pull_request'")
        self.assertEqual(rust["run"], 'python3 tools/ci/rust_changes.py "$BASE_SHA" "$HEAD_SHA" >> "$GITHUB_OUTPUT"')
        self.assertEqual(rust["env"], {"BASE_SHA": "${{ github.event.pull_request.base.sha }}",
                                       "HEAD_SHA": "${{ github.event.pull_request.head.sha }}"})

    def answer(self, rust, launcher, mod_api):
        """(exit code, GITHUB_OUTPUT) of verify-scope's answers step."""
        with tempfile.TemporaryDirectory() as scratch:
            output = Path(scratch) / "output"
            output.write_text("")
            result = self.bash(self.answers["run"], {"RUST": rust, "LAUNCHER": launcher, "MOD_API": mod_api,
                                                     "GITHUB_OUTPUT": str(output)})
            return result.returncode, output.read_text()

    def test_the_answers_are_exactly_true_or_false(self):
        self.assertEqual(self.answer("true", "false", "false"), (0, "rust=true\nlauncher=false\nmod_api=false\n"))
        self.assertEqual(self.answer("false", "true", "true"), (0, "rust=false\nlauncher=true\nmod_api=true\n"))
        # A step that didn't run (a push, a manual run, no earlier commit): the part runs.
        self.assertEqual(self.answer("", "", ""), (0, "rust=true\nlauncher=true\nmod_api=true\n"))
        # `if` compares without case: FALSE would skip a part as false does. Refused, as anything else.
        for answers in [("FALSE", "false", "false"), ("false", "False", "false"), ("true", "yes", "true"),
                        ("0", "true", "true"), ("true", "false\nrust=false", "true"), ("true", "true", "FALSE"),
                        ("true", "true", "false\nrust=false"), ("true", "true", "no")]:
            with self.subTest(answers=answers):
                self.assertNotEqual(self.answer(*answers)[0], 0)

    def test_a_part_left_out_passes(self):
        for name, _, answer, _, _ in self.GATED:
            with self.subTest(part=name):
                self.assertTrue(self.passes(**{name: "skipped", answer: "false"}))
        # All of them at once: a pull request that changes only documentation.
        self.assertTrue(self.passes(LINT="skipped", SERVER="skipped", LAUNCHER="skipped", TEST="skipped",
                                    MODAPI="skipped", RUST_ANSWER="false", LAUNCHER_ANSWER="false",
                                    MODAPI_ANSWER="false"))

    def test_a_skip_passes_only_on_exactly_false(self):
        # The gate compares with case, as the answers step publishes: FALSE or an empty answer is no decision.
        for value in ("true", "FALSE", "False", "", "false "):
            for name, _, answer, _, _ in self.GATED:
                with self.subTest(part=name, answer=value):
                    self.assertFalse(self.passes(**{name: "skipped", answer: value}))

    def test_a_skip_passes_only_on_its_own_answer(self):
        # The answers are independent: a change to the mod API check alone needs no Rust tests, and a Rust change
        # that can't reach the mod API needs no comparison.
        self.assertFalse(self.passes(TEST="skipped", MODAPI_ANSWER="false", LAUNCHER_ANSWER="false"))
        self.assertFalse(self.passes(SERVER="skipped", MODAPI_ANSWER="false", LAUNCHER_ANSWER="false"))
        self.assertFalse(self.passes(MODAPI="skipped", RUST_ANSWER="false", LAUNCHER_ANSWER="false"))
        self.assertFalse(self.passes(LAUNCHER="skipped", RUST_ANSWER="false", MODAPI_ANSWER="false"))
        self.assertFalse(self.passes(LINT="skipped", LAUNCHER_ANSWER="false", MODAPI_ANSWER="false"))

    def test_a_failed_or_cancelled_part_fails(self):
        for result in ("failure", "cancelled"):
            for value in ("true", "false"):
                for name, _, answer, _, _ in self.GATED:
                    with self.subTest(result=result, part=name, answer=value):
                        self.assertFalse(self.passes(**{name: result, answer: value}))
                        self.assertFalse(self.passes(**{name: result, "EVENT": "merge_group"}))

    def test_a_part_skipped_without_a_decision_fails(self):
        # verify-scope failed or was cancelled: what waits on it is skipped, and its outputs are empty.
        skipped = {name: "skipped" for name, _, _, _, _ in self.GATED}
        for scope in ("failure", "cancelled", "skipped"):
            with self.subTest(scope=scope):
                self.assertFalse(self.passes(SCOPE=scope, **skipped, RUST_ANSWER="", LAUNCHER_ANSWER="",
                                             MODAPI_ANSWER=""))
                self.assertFalse(self.passes(SCOPE=scope, **skipped, RUST_ANSWER="false", LAUNCHER_ANSWER="false",
                                             MODAPI_ANSWER="false"))
            for name, _, _, _, _ in self.GATED:
                with self.subTest(scope=scope, part=name):
                    self.assertFalse(self.passes(SCOPE=scope, **{name: "skipped"}, RUST_ANSWER="", LAUNCHER_ANSWER="",
                                                 MODAPI_ANSWER=""))

    def test_the_merge_queue_shape_passes(self):
        self.assertTrue(self.passes(EVENT="merge_group", LINT="skipped", SERVER="skipped", LAUNCHER="skipped"))

    def test_the_merge_queue_still_runs_the_rust_tests_and_mod_api(self):
        # Their `if`s don't leave the merge queue out and verify-scope answers true there, so a skip is no decision.
        for name in ("TEST", "MODAPI"):
            with self.subTest(part=name):
                self.assertFalse(self.passes(EVENT="merge_group", LINT="skipped", SERVER="skipped",
                                             LAUNCHER="skipped", **{name: "skipped"}))

    def test_the_tree_part_is_judged_as_before(self):
        self.assertTrue(self.passes(TREE="skipped"))
        self.assertFalse(self.passes(TREE="failure"))
        self.assertFalse(self.passes(TREE="cancelled"))

    def test_every_part_ends_with_a_bounded_load_probe_that_cannot_fail_it(self):
        for part in ("verify-tree", "verify-lint", "verify-server", "verify-launcher", "verify-test",
                     "verify-mod-api"):
            with self.subTest(part=part):
                probe = self.jobs[part]["steps"][-1]
                self.assertEqual(probe["name"], "Load probe")
                self.assertEqual(probe["if"], "always()")
                self.assertIs(probe["continue-on-error"], True)
                self.assertTrue(probe["run"].startswith("timeout -k 5 10 tools/ci/load_probe.sh || echo "), probe["run"])

    def test_the_load_probe_prints_one_line_and_succeeds(self):
        if not BASH:
            self.skipTest("no bash")
        result = subprocess.run([BASH, str(ROOT / "tools/ci/load_probe.sh")], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(result.stdout.splitlines()), 1, result.stdout)
        self.assertTrue(result.stdout.startswith("load probe: "), result.stdout)


class InstallRustTests(unittest.TestCase):
    """tools/ci/install_rust.sh with stand-ins for rustup, sudo and apt-get: apt beside rustup, always reaped, bounded,
    and either one's failure fails the step."""

    STAND_INS = {
        "rustup": 'echo "rustup $*"\nexit "${FAKE_RUSTUP:-0}"\n',
        "sudo": 'echo "sudo $*"\nexec "$@"\n',
        "apt-get": 'echo "apt-get $*"\nsleep "${FAKE_APT_SLEEP:-0}"\nexit "${FAKE_APT:-0}"\n',
    }

    def install(self, *packages, **env):
        if not BASH:
            self.skipTest("no bash")
        with tempfile.TemporaryDirectory() as scratch:
            for name, body in self.STAND_INS.items():
                path = Path(scratch) / name
                path.write_text("#!/usr/bin/env bash\n" + body, newline="\n")
                path.chmod(0o755)
            env = {**os.environ, "HOSTED": "true", **env, "PATH": scratch + os.pathsep + os.environ["PATH"]}
            return subprocess.run([BASH, str(ROOT / "tools/ci/install_rust.sh"), *packages], env=env,
                                  capture_output=True, text=True, timeout=60)

    def test_both_succeed(self):
        result = self.install("libudev-dev", "mold")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("rustup toolchain install stable", result.stdout)
        install = "apt-get install -y --no-install-recommends -o Dpkg::Use-Pty=false libudev-dev mold"
        self.assertIn(install, result.stdout)
        self.assertIn("::group::apt-get (exit 0)", result.stdout)
        # The bound runs as root, so it can stop apt-get and the dpkg it starts, not only sudo.
        self.assertIn("sudo timeout -k 30 300 apt-get update", result.stdout)
        self.assertIn(f"sudo timeout -k 30 300 {install}", result.stdout)

    def test_a_rustup_failure_still_reaps_apt_and_shows_it(self):
        result = self.install("libudev-dev", FAKE_RUSTUP="3", FAKE_APT_SLEEP="1")
        self.assertEqual(result.returncode, 3, result.stdout + result.stderr)
        self.assertIn("::group::apt-get (exit 0)", result.stdout)
        self.assertIn("apt-get install -y --no-install-recommends -o Dpkg::Use-Pty=false libudev-dev", result.stdout)

    def test_an_apt_failure_fails_the_step(self):
        result = self.install("libudev-dev", FAKE_APT="5")
        self.assertEqual(result.returncode, 5, result.stdout + result.stderr)
        self.assertIn("::group::apt-get (exit 5)", result.stdout)

    def test_a_stalled_apt_is_stopped(self):
        result = self.install("libudev-dev", FAKE_APT_SLEEP="40", APT_TIMEOUT="1")
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertIn("::group::apt-get (exit 124)", result.stdout)

    def test_no_apt_off_blacksmith(self):
        result = self.install("libudev-dev", HOSTED="false")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("apt-get", result.stdout)


class TryRunTests(unittest.TestCase):
    """A maintainer's hand-started `checks` run of a branch (docs/ci-runners.md#try-runs) saves no sticky disk, no
    Rust cache, and holds no secret: only main's pushes and the merge queue write what pull requests start from."""

    def test_only_mains_pushes_and_the_merge_queue_save_a_build_disk(self):
        text = (WORKFLOWS / "ci.yml").read_text()
        # Every disk's `commit:` is this variable (tests/test_ci_source_mtimes.py).
        self.assertIn("  SAVE_BUILD_DISK: ${{ ((github.event_name == 'push' && github.ref == 'refs/heads/main') || "
                      "github.event_name == 'merge_group') && 'true' || 'false' }}\n", text)
        self.assertNotIn("secrets.", text)

    def test_mod_api_snapshots_are_kept_only_on_mains_disk(self):
        # A pull request compares with a stored snapshot of its base, so none of its jobs may store one: only the runs
        # that save the disk, and only on Blacksmith, whose disk a pull request mounts but cannot save (our checks
        # runners keep what any job leaves in the build folder).
        steps = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["verify-mod-api"].split("\n      - ")
        store = [step for step in steps if "--store-snapshot" in step]
        read = [step for step in steps if "--snapshots" in step]
        self.assertEqual((len(store), len(read)), (1, 1))
        self.assertIn("if: github.event_name != 'pull_request'\n", store[0])
        self.assertIn("STORE: ${{ env.SAVE_BUILD_DISK == 'true' && startsWith(runner.name, 'blacksmith-') }}\n",
                      store[0])
        self.assertIn('if [ "$STORE" = true ]; then\n', store[0])
        self.assertIn("HOSTED: ${{ startsWith(runner.name, 'blacksmith-') }}\n", read[0])
        self.assertIn('if [ "$HOSTED" = true ]; then\n', read[0])

    def test_the_surface_report_gates_pull_requests_beside_the_mod_api_check(self):
        # The surface report left with the fork's gate workflow (878ac391) and with it the only pull request check
        # that sees a changed wire layout or a new hook argument (#18742, #18874).
        steps = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["verify-mod-api"].split("\n      - ")
        report = [step for step in steps if "python3 tools/ci/surface_report.py\n" in step + "\n"]
        self.assertEqual(len(report), 1)
        self.assertIn("github.event_name == 'pull_request'", report[0])
        # The job runs only where verify-scope's answer let it start: no step asks again.
        self.assertNotIn("steps.rust", report[0])
        self.assertIn("cargo build --locked --quiet -p horizon-mod-api-gen --bin horizon-stage\n", report[0])
        self.assertIn("HORIZON_STAGE_TOOL=", report[0])
        self.assertIn("EVENT: ${{ github.event_name }}\n", report[0])
        self.assertNotIn("continue-on-error", report[0])
        # The base's compiled layouts come from the store main's runs fill (wire-layouts, by commit).
        self.assertIn('WIRE_LAYOUTS_STORE="${CARGO_TARGET_DIR:-$PWD/target}/wire-layouts"', report[0])
        store = [step for step in steps if "--store-layouts" in step]
        self.assertEqual(len(store), 1)
        self.assertIn("if: github.event_name != 'pull_request'", store[0])
        self.assertIn('--store-layouts "$CARGO_TARGET_DIR/wire-layouts"', store[0])
        # Only a push to main stores one: the merge queue saves the disk too.
        self.assertIn('if [ "$GITHUB_EVENT_NAME" = push ] && [ "$GITHUB_REF" = refs/heads/main ]; then\n'
                      '              python3 tools/ci/surface_report.py --store-layouts', store[0])

    def test_the_windows_build_runs_by_hand_and_saves_no_cache(self):
        block = dict(jobs((WORKFLOWS / "ci.yml").read_text()))["windows-x64"]
        self.assertIn("    if: github.event_name == 'workflow_dispatch' && inputs.windows\n", block)
        # Nothing a branch built may reach a cache that main's builds restore.
        self.assertNotIn("rust-cache@", block)
        self.assertNotIn("actions/cache", block)
        self.assertIn("persist-credentials: false", block)
        self.assertIn("retention-days: 3", block)
        # Not the workflow's actions: read.
        self.assertIn("    permissions:\n      contents: read\n", block)


if __name__ == "__main__":
    unittest.main()
