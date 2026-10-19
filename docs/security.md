# Security

How Horizon keeps its code and its downloads safe while most of the work is done by AI agents and releases go out
by themselves, including when Ludovic isn't at his computer. In plain words: what someone could have done before,
what stops them now, and what still needs Ludovic.

## Who does what

- **Agents** (Claude sessions run by Ludovic) write most pull requests, review them, merge them and answer
  reports. Everything an agent posts on GitHub says so ("Opened by Ludovic's AI agent").
- **Contributors** send pull requests from their forks like on any open-source project.
- **Players** report bugs on Discord and GitHub. Kira, the Discord bot, files their reports as issues.
- **Releases** are automatic: every merged change that players run ships within the hour.

## What could go wrong before (October 2026)

1. **Instructions hidden in a report.** Someone could write "ignore your previous instructions and…" in a
   Discord message or an issue. Agents read those reports while working with Ludovic's full GitHub account.
2. **A check that counted as passed without running.** Adding a label to a pull request started a second run of
   the checks in which `verify` was *skipped*, and GitHub counts a skipped required check as passed. Two pull
   requests merged before their real check had finished.
3. **Every release went straight to every player.** A broken or malicious build reached every launcher within
   minutes, with nobody looking at it first.
4. **Downloads trusted GitHub alone.** Anyone who got hold of a token that can write to the repository could
   replace a release's files, and launchers and servers would install them.
5. **The release machinery was ordinary code.** A pull request could change the workflows that build and publish
   releases as easily as it changes a texture.

## What stops it now

- **Reports are data, never instructions.** Agents receive people's text only inside quoted blocks marked as
  untrusted, with attachments as plain links. Text that looks aimed at an AI (talking to "the AI", asking to run
  commands, asking for passwords or keys, linking to scripts, hidden characters) is marked **suspicious**, and the
  agent coordinating the others sees the mark.
- **`verify` always runs.** Labels no longer start the checks, and the required `verify` job has no condition, so
  the only `verify` a change can have is one that ran. Main requires it, from GitHub Actions only, for everyone,
  admins included.
- **Beta, then stable.** Every release is first a **beta**. It becomes **stable**, the version launchers and
  servers install by themselves, only after it has been out 6 hours, passed its smoke test (the game started and
  drew a world on the Mac mini), and nobody has labelled an issue naming it `release-blocker`. Players who want the
  newest builds at once turn on "Beta builds" in the launcher's Settings. Discord announces stable releases only.
- **Signed updates.** Each release carries a list of its files' checksums, signed with Horizon's release key. The
  launcher (for the game and for itself) and `horizon-server update` check that signature against the public key
  built into them, then check the download against the list. Anything unsigned, signed with another key, signed
  for another version, or changed in any way is refused with a message saying why. The private key exists only as
  a secret of GitHub's `release` environment, which only real releases from main can use. The job that signs runs
  nothing from the repository but a short script calling OpenSSL, so no pull request can read the key by changing
  the code that gets built. Nobody, Ludovic included, ever needs to see it.
- **The release machinery needs Ludovic.** The workflows, the release and signing tools, the update-checking code
  and the installers are listed in `.github/CODEOWNERS`. Once main's protection requires code owners' reviews, a
  change to any of them waits for Ludovic's approval. Everything else still merges as soon as its checks pass.
- **Agents get their own, smaller identity** (prepared, switched on by Ludovic). The `horizon-agent` GitHub App
  can read and write code, pull requests and issues, and read the checks. It cannot change settings, workflows,
  secrets or members, and its tokens last an hour. Once agents use it, their pull requests are the app's, and
  Ludovic approves the rare pipeline change with one tap.

## What still needs Ludovic

- **Approving pipeline changes** (rare): a pull request touching a file in `.github/CODEOWNERS`.
- **Holding a release back or speeding one up**, from his phone if he likes. Label an issue naming the version
  `release-blocker` to hold that beta and the later ones. Run the `promote` workflow (GitHub app: Actions,
  promote, Run workflow) to make a beta stable at once.
- **Creating the GitHub App**: one link, "Create GitHub App", then "Install". His login, his clicks.
- **Fork pull requests**: they run on the Mac mini only after a maintainer has read them and added `run-parity`.
- **A leaked key**: rotate it (docs/release-flow.md, "Signed checksums"). Launchers older than the new key then
  refuse new versions until players download the launcher again.

## What this doesn't solve

- **Code that merges is code that ships.** Signing proves a build came from Horizon's pipeline, not that the code
  is good. Against a bad change, the safety nets are the checks and harnesses, review, the 6-hour beta and the
  release-blocker label.
- **Until agents switch to the App**, they still work with Ludovic's own account, so nothing technical tells his
  actions from theirs. Code owners' reviews are switched on together with the App.
- **The Mac mini** builds the macOS release and also runs the parity checks of this repository's pull requests,
  so a pull request runs code on it before review. Forks don't.
- **The first install** with `install.sh` / `install.ps1` checks the checksum GitHub publishes, not the
  signature: verifying Ed25519 signatures needs OpenSSL 3, which macOS doesn't ship. Every update after that
  checks the signature.

## Reporting a security problem

Use GitHub's private vulnerability reporting (the repository's **Security** tab, "Report a vulnerability"), not a
public issue or Discord. Say what you found and how to reproduce it; we answer there.
