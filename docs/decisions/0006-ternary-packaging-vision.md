# ADR 0006 — Ternary Packaging and Versioning Vision

**Status:** Accepted (Long-term vision for v0.4 / v0.5)

**Date:** 2026-05-10

## Context

During the development of the module system (v0.2.x) and in preparation for Pillar 3.1 (CAS Packaging), a philosophical argument was proposed regarding the application of **Balanced Ternary (-1, 0, +1) logic** to the Packaging and Versioning architecture. The philosophy of the Triet language extends beyond using ternary logic at the mathematical or boolean level; it is embedded within the software architecture itself.

This document records that breakthrough idea, directing how Triet will manage versions and dependencies in the future, while noting technical concerns to be addressed when the Package Manager system is implemented.

## Arguments & Design Decisions

### 1. Radix Economy
**Initial Argument:** Mathematically, base 3 (which is close to $e$) provides the most efficient information representation. Therefore, a ternary system can manage a larger and more compact identifier space than a binary system.
**Evaluation & Direction:** This is theoretically sound according to information theory. This strongly supports Triet's **CAS (Content-Addressable Storage)** architecture. When hashing library packages, an address space represented (or compressed) in ternary will yield optimal representation efficiency.

### 2. Versioning via "Stability State"
**Initial Argument:** Instead of traditional SemVer (1.2.3), Triet versions represent the nature of change through three states:
- `0 (Neutral)`: A stable baseline version.
- `+1 (Positive)`: An expansion, adding new features.
- `-1 (Negative)`: A refactor, cleanup, or optimization (API-preserving).

**Evaluation & Direction:** This is a **breakthrough** idea. It transforms versioning from a quantitative metric to a **qualitative (Semantic Intent)** one.
- **Decision:** The Triet Package Manager (in v0.5) will utilize a versioning model based on Ternary Vectors combined with CAS. A version will be a sequence of decisions: `[Hash of original, +1, +1, -1, 0]`. The package manager and AI will automatically understand whether a library is expanding or being pruned.

### 3. Ternary Tree Hierarchy for Modules
**Initial Argument:** Instead of an arbitrary directory tree, namespaces are divided into three natural logical branches:
- `Middle branch (0)`: Core logic.
- `Right branch (+1)`: High-level API, extensions.
- `Left branch (-1)`: Low-level, hardware drivers.
Example: `sys.io.0`, `sys.io.+1`, `sys.io.-1`.

**Evaluation & Direction:** A perfectly symmetric model. It links directly to **Pillar 3.5 (Capability System)**.
- **Technical Note (Concern):** Naming directories or modules `sys.io.-1` will be difficult for developers (User ergonomics).
- **Solution:** This Ternary Tree structure will be applied as a **metadata constraint** rather than a physical name. We will introduce syntax to specify the "layer": `module sys.io (layer: -1)`. Consequently, the compiler will automatically enforce Capabilities (e.g., the `-1` layer will require an Explicit Grant to be invoked).

### 4. Resolving Dependency Conflicts via Ternary CMP
**Initial Argument:** A ternary comparison function `CMP(a, b)` that returns `-1, 0, 1` can enable the Package Manager to resolve dependency nodes extremely quickly without complex tree traversal, as the version itself is inherently comparative.

**Technical Note (Core Concern):**
Dependency Resolution is, in graph theory terms, a SAT (satisfiability) problem. Simply having a 3-way `<=>` operator does not break the mathematical limit required to transform SAT into an $O(1)$ problem.
*However*, the combination of **Ternary Vector Versioning** (point 2) and searching for the **Stability State (0)** may allow us to replace traditional graph traversal algorithms with a **Ternary Search Tree**. Instead of searching for the "highest compatible version" (as seen in Cargo/NPM), Triet will search for the "version with the state 0 closest to the requested Hash." This has the potential to significantly reduce computational complexity.

## Consequences

This document does not directly impact the current Phase v0.2 source code. It serves as a "North Star" to guide AI and future developers when the Package Manager (v0.5) and Capability (v0.6) architectures are designed. Traditional binary SemVer must not be implemented for the Triet language.
