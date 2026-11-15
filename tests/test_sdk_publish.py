"""The mod SDK is published in one file, sdk-publish.yml, which every publisher calls (#4022, #4108)."""
import unittest

from tests.test_ci_concurrency import ROOT, yaml

WORKFLOWS = ROOT / ".github/workflows"


def load(name):
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


class SdkPublishTests(unittest.TestCase):
    def test_only_sdk_publish_releases_an_sdk_and_every_caller_hands_it_the_key(self):
        publishers = sorted(path.name for path in WORKFLOWS.glob("*.yml") if "Release sdk-<sha>" in path.read_text())
        self.assertEqual(publishers, ["sdk-publish.yml"])
        callers = {}
        for path in WORKFLOWS.glob("*.yml"):
            for name, job in load(path.name)["jobs"].items():
                if job.get("uses") == "./.github/workflows/sdk-publish.yml":
                    callers[path.name] = job
        self.assertEqual(sorted(callers), ["nightly-build.yml", "prebuilt-game.yml"])
        for name, job in callers.items():
            with self.subTest(caller=name):
                self.assertEqual(job["secrets"], "inherit")
                self.assertEqual(job["permissions"]["contents"], "write")
                self.assertEqual(set(job["with"]), {"commit", "publish"})

    def test_the_nightly_publishes_its_sdk_after_the_build_and_never_on_a_dry_run(self):
        jobs = load("nightly-build.yml")["jobs"]
        self.assertEqual(jobs["sdk"]["uses"], "./.github/workflows/prebuilt-game-build.yml")
        self.assertEqual(jobs["sdk"]["with"], {"commit": "${{ needs.pick.outputs.sha }}"})
        publish = jobs["sdk-publish"]
        self.assertEqual(publish["needs"], ["pick", "build", "sdk"])
        self.assertEqual(publish["if"], "needs.sdk.outputs.needed == 'true'")
        self.assertEqual(publish["with"], {"commit": "${{ needs.pick.outputs.sha }}",
                                           "publish": "${{ needs.pick.outputs.dry_run != 'true' }}"})
        self.assertIn("sdk-index", jobs["sign"]["needs"])

    def test_the_sdk_build_packs_exactly_the_archives_a_release_needs(self):
        from tools.ci import nightly
        build = yaml.load((ROOT / ".github/workflows/prebuilt-game-build.yml").read_text(),
                          Loader=yaml.BaseLoader)["jobs"]["build"]
        # Each matrix leg packs its three slices; the build data and surface are gathered once from them all.
        names = {f"{kind}-{entry['target']}{'-server' if entry.get('server') == 'true' else ''}.tar.xz"
                 for entry in build["strategy"]["matrix"]["include"] for kind in ("runtime", "api", "engine")}
        self.assertEqual(names | {"api-source.tar.xz", "build-data.tar.xz", "surface.json"}, set(nightly.SDK_ARCHIVES))
        self.assertEqual(len(nightly.SDK_ARCHIVES), len(set(nightly.SDK_ARCHIVES)))
        steps = {step.get("name") or step.get("uses"): step for step in build["steps"]}
        self.assertEqual(steps["Pack the SDK's assets"]["run"], "python tools/prebuilt_game.py pack prebuilt dist")
        self.assertEqual(steps["actions/upload-artifact@v4"]["with"]["name"], "sdk-leg-${{ matrix.name }}")
        self.assertNotIn("env", build, "no hand-set archive name")

        # The launcher and the mod tool ask for horizon_mod_package::sdk::Asset::name(kind, target, side):
        # `{kind}-{target}{-server}.tar.xz`, the kinds named runtime, api and engine; and the release's other files
        # are its BESIDE_ASSETS. Producer (the legs), publish check (SDK_ARCHIVES, SDK_ASSETS) and launcher agree.
        sdk = (ROOT / "crates/horizon-mod-package/src/sdk.rs").read_text()
        self.assertIn('format!("{}-{target}{suffix}.tar.xz", kind.name())', sdk)
        self.assertIn('let suffix = if side == Side::Server { "-server" } else { "" };', sdk)
        for kind in ("runtime", "api", "engine", "build-data", "api-source"):
            self.assertIn(f'=> "{kind}",', sdk)
        self.assertIn('pub const INDEX: &str = "sdk.json";', sdk)
        self.assertIn('pub const BESIDE_ASSETS: [&str; 3] = [INDEX, "SHA256SUMS", "SHA256SUMS.sig"];', sdk)
        self.assertEqual(set(nightly.SDK_ASSETS) - set(nightly.SDK_ARCHIVES), {"sdk.json", "SHA256SUMS", "SHA256SUMS.sig"})

    def test_the_sdk_is_signed_and_published_only_on_main(self):
        workflow = yaml.load((ROOT / ".github/workflows/sdk-publish.yml").read_text(), Loader=yaml.BaseLoader)
        jobs = workflow["jobs"]
        on_main = "inputs.publish && github.ref == 'refs/heads/main'"
        self.assertEqual(jobs["sign"]["environment"], f"${{{{ {on_main} && 'release' || '' }}}}")
        sign = jobs["sign"]["steps"][-3]
        self.assertEqual(sign["run"], 'bash tools/ci/sign_release.sh "$TAG" dist/*')
        self.assertEqual(sign["env"]["EPHEMERAL"], f"${{{{ !({on_main}) && 'yes' || '' }}}}")
        for key in ("RELEASE_SIGNING_KEY", "RELEASE_SIGNING_KEY_NEXT"):
            self.assertEqual(sign["env"][key], f"${{{{ {on_main} && secrets.{key} || '' }}}}")
        self.assertEqual(workflow["env"]["TAG"], "sdk-${{ inputs.commit }}")
        # Only the sign job sees the key, and it checks out nothing but the signing script; the key signs only a
        # commit of main, whatever commit the caller names.
        self.assertEqual(jobs["sign"]["steps"][0]["with"]["sparse-checkout"], "tools/ci/sign_release.sh")
        on_main_check = jobs["sign"]["steps"][1]
        self.assertEqual(on_main_check["if"], f"${{{{ {on_main} }}}}")
        self.assertIn('git merge-base --is-ancestor "$COMMIT" refs/remotes/origin/main', on_main_check["run"])
        # The jobs that read the legs' output cannot write; the legs' artifacts only, not an earlier attempt's own.
        for job in ("index", "sign"):
            self.assertEqual(jobs[job]["permissions"], {"contents": "read"}, job)
        self.assertEqual(jobs["publish"]["permissions"], {"contents": "write"})
        self.assertEqual(jobs["index"]["steps"][4]["with"]["pattern"], "sdk-leg-*")
        self.assertNotIn("environment", jobs["index"])
        self.assertNotIn("environment", jobs["publish"])
        publish = {step.get("name"): step for step in jobs["publish"]["steps"]}
        self.assertEqual(jobs["publish"]["env"]["PUBLISH"], f"${{{{ {on_main} }}}}")
        self.assertEqual(publish["Release sdk-<sha>"]["if"], "env.PUBLISH == 'true'")
        order = [step.get("name") for step in jobs["publish"]["steps"]]
        self.assertLess(order.index("The signature, checked as the launcher checks it"), order.index("Release sdk-<sha>"))
        self.assertLess(order.index("Exactly the assets a Nightly needs"), order.index("Release sdk-<sha>"))
        hand = yaml.load((ROOT / ".github/workflows/prebuilt-game.yml").read_text(), Loader=yaml.BaseLoader)
        self.assertEqual(hand["jobs"]["publish"]["with"]["publish"], "${{ github.ref == 'refs/heads/main' }}")


if __name__ == "__main__":
    unittest.main()
