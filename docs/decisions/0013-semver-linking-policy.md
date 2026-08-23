# ADR 0'013 — Semver linking policy

**Status:** Decision. Applicable to the v0.4 linker/loader and all tools reading cross-package dependency relationships (linker, future package manager). Directly references ABI metadata (ADR-0011) and witness dispatch (ADR-0012).

**Issue:** VISION §3.3 commits: *"compiler refuse-to-link with clear diff"* during cross-package ABI mismatch. However, "mismatch" exists at multiple levels:

- Patch version change (`1.2.3 → 1.2.4`): bug fix, ABI remains unchanged → link OK.
- Minor version change (`1.2.x → 1.3.x`): additive (new exports), backwards-compatible → link
