# Target validation evidence

This directory contains reviewed Windows/macOS physical-target evidence. CI and cross-compilation are not target evidence.

Create one JSON file per `ASSURANCE-TARGET-*` using this schema:

```json
{
  "targetId": "ASSURANCE-TARGET-005",
  "scope": "windows",
  "commit": "40-character-git-commit",
  "version": "2.1.0",
  "executedOn": "2026-11-15",
  "tester": "qualified tester identity",
  "reviewer": "different qualified reviewer identity",
  "result": "passed",
  "cases": ["WASAPI default render endpoint loopback", "mixed-source underflow"],
  "artifacts": [{"name": "target-log.zip", "sha256": "64 lowercase hex characters"}]
}
```

Allowed scopes are fixed by `scripts/check-target-evidence.mjs`. Evidence must not contain API keys, proxy credentials, PCM, transcripts, or unredacted server payloads. Store large artifacts outside Git and record their immutable name and SHA-256 digest here.

Use a workflow `prerelease` with an `alpha`, `beta`, or `rc` SemVer tag to distribute an explicitly unvalidated candidate to testers. That path builds the selected platforms from the current immutable commit and must never be cited as acceptance evidence by itself.

After testing one immutable candidate commit with the intended final numeric version, commit only these reviewed evidence records on the control branch. A production `release` preflight runs `node scripts/check-target-evidence.mjs --require-complete --version <final-version> --print-commit`, requires every record to identify the same candidate and final version, checks that commit exists, forces an all-platform build, and rebuilds that exact candidate. Absent, mixed, malformed, self-reviewed, or unrelated evidence prevents v2.1.0 production publication.
