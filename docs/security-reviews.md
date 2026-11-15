# Records of scoped source reviews

A signed release checksum establishes which bytes were published. It does not
say which code a reviewer read, what they checked, or whether the code is safe.
Keep those statements separate when recording a source review.

Small JSON records in `docs/security-reviews/` identify a full source commit,
exact file bytes (SHA-256 and Git mode), review date, reviewer, method, explicit
line ranges and questions, conclusions, and remaining work. See the
[release-signing example](security-reviews/2026-11-12-release-signing.json).
Its source inspection was assisted by AI, with no independent human sign-off.
It documents the selected operations, not approval of the entire signing script.

## Check a record

From a checkout containing the recorded Git objects:

```sh
python3 tools/check_security_review.py docs/security-reviews/2026-11-12-release-signing.json
python3 tools/check_security_review.py docs/security-reviews/2026-11-12-release-signing.json --against HEAD
```

The first command checks metadata and hashes against the recorded commit. The
second also rejects source changes at `HEAD`, including changes outside the
named lines in that file. It compares committed objects, not uncommitted working
tree edits. The tool reads Git objects and uses Python's standard library; it
does not run the reviewed scripts or contact game services.
If objects are missing in a shallow checkout, fetch the recorded commit first.
Neither command authenticates the reviewer or validates their conclusions.

## Record format

- `schema_version`: `1`.
- `source_commit`: full lowercase Git commit ID, never a moving branch/tag.
- `reviewed_at`: ISO date, `YYYY-MM-DD`.
- `reviewer`: nonempty `identity` and `method`, plus the boolean
  `independent_human_review`. Identify agent assistance rather than inferring a
  human endorsement from the account that submits a record.
- `files`: a nonempty list of unique repository-relative paths. Each entry has
  `path`, `git_mode` (`100644` or `100755`), lowercase `sha256`, `scope`, `result`,
  `conclusion`, `whole_file_status`, and `remaining`.
- `scope`: nonempty list of objects containing inclusive `lines: [first, last]`
  and a nonempty `question`. These are positions at `source_commit`.
- `result`: `no_findings_in_reviewed_scope` or `observations`. Neither means
  that the file, its dependencies or the resulting executable is safe.
- `whole_file_status`: always `pending` in this scoped format. `remaining`
  explains what the review did not cover. Unlisted files are also pending.

Unknown fields, invalid ranges, symlinks and missing or mismatched objects fail
verification. SHA-256 is over the raw file bytes, not Git's blob header. The
record intentionally does not contain a `safe` flag or a coverage percentage.
Repository path counts would also include docs, tests and assets, so they are
not a measure of runtime security coverage.

## Updating and contributing

Do not replace a digest simply to make a failed check pass. Inspect the changed
code and relevant callers, then add a record for the new commit with the actual
scope, date, reviewer and observations. Retain old records as historical
evidence; conclusions never automatically transfer to a new revision.

Records are review assertions under the repository's normal contribution and
review process, not signed auditor attestations or independent certifications.
A person able to change both a record and its source can create another matching
pair. Source/binary equivalence, dependency auditing and runtime observation
need their own evidence. A suspected exploitable vulnerability belongs in the
[private reporting channel](security.md#reporting-a-security-problem), not a
public record containing exploit details, real sessions or private logs.
