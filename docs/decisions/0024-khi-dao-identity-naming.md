# ADR 0024 — Khí + Đạo Identity Naming (Đạo Đức Kinh)

**Trạng thái:** Quyết định. Áp dụng cho v0.7.x.identity sub-task series (5 commit + 1 ADR commit này, ship trước v0.7.10 mở).

**Origin:** Author 2026-05-24 (sau khi v0.7.9.5 đóng và byte-identical gate xanh). Trước khi mở v0.7.10 CLI wiring, author đặt câu hỏi: *"Hiện tại chúng ta đang dùng `crate` và `cargo` theo Rust. Tôi muốn chúng có tên riêng phù hợp với các đặt tên của ngôn ngữ là Triết."*

## §1 — Vấn đề: Rust-inherited surface naming

Hiện trạng v0.7.9.5 còn 5 thuật ngữ kế thừa từ Rust nằm trên **user-facing surface** của Triết:

| Element | Hiện tại | Visibility |
|---|---|---|
| Path keyword | `crate.foo.bar` | User-facing — ~114 lượt dùng trong `.tri` source (std/examples/demos/compiler) |
| Compiled artifact | `.khi` | User-facing — `dao build` output |
| CLI tool binary | `triet` | User-facing — mọi user command |
| Manifest filename | `triet.package` | User-facing — root của project |
| Lockfile | `triet.lock` | User-facing — committed vào VCS |

(Note: `Cargo.toml` / `Cargo.lock` ở root workspace là Rust dev artifact để build Rust-impl compiler, KHÔNG trên Triết user surface — nằm ngoài phạm vi ADR này.)

**Vấn đề bản sắc:**

1. Triết VISION (§3) emphasizes Vietnamese-rooted philosophical depth. Surface terms kế thừa English logistics metaphor (`crate`, `cargo`, `pack`) làm yếu identity.
2. Half-renamed state hiện tại không nhất quán: manifest đã có prefix `triet.`, wire format đã có `.khi`, nhưng path keyword vẫn là `crate` — gây cognitive split.
3. SPEC §"Path keywords" + CLAUDE.md "Reserved namespace roots" liệt kê `crate` chung với `std/sys/dev/usr/core` — duy nhất từ này không phải ASCII transliteration của Vietnamese concept.

## §2 — Quyết định: Khí (器) + Đạo (道)

Áp dụng cặp khái niệm cốt lõi từ **Đạo Đức Kinh** (Lão Tử) làm framework đặt tên:

- **Khí (器)** — vessel, utensil, instrument. Theo §28: *"Phác tán tắc vi khí"* (樸散則為器) — khi phác (uncarved block, chất gốc) tán ra, nó trở thành các khí (vessel). Mapping compilation: source `.tri` (phác) → compile (tán) → artifact `.khi` (vessel chứa philosophical content).
- **Đạo (道)** — the Way, principle, process. Theo §42: *"Đạo sinh nhất, nhất sinh nhị, nhị sinh tam, tam sinh vạn vật"* (道生一,一生二,二生三,三生萬物) — Đạo sinh nhị nguyên rồi tam phân. Direct alignment với balanced ternary identity của Triết (Trit::Negative / Zero / Positive — tam phân là consequence of Đạo).

**Tại sao 2 khái niệm này, không phải khác:**

| Cặp đã xét | Lý do từ chối |
|---|---|
| `package` + `triet` (giữ Rust naming, chỉ thay path keyword) | Pragmatic nhưng KHÔNG philosophical, không thể hiện Vietnamese identity |
| `treatise/corpus/volume/opus` + giữ `triet` CLI | English Latin/academic — vẫn là Western framework, không Việt |
| `niệm` (念) + `hành` (行) — Wang Yangming "tri hành hợp nhất" | Tốt nhưng epistemological (knowing/acting) hơn là ontological (becoming); ít ternary tie; Neo-Confucian scholar-level hơn là foundational |
| `khí` + `pháp` (法 — method/dharma) | `pháp build` ASCII clear hơn `dao build`; nhưng pháp Buddhist Sanskrit-rooted, không native Đạo Đức Kinh; mất direct Lão Tử reference |
| `phác` (樸) + `đạo` (source extension `.phac`) | Metaphor cực mạnh (source = phác) nhưng mất chữ `tri` báo hiệu ngôn ngữ tam phân; rename 1000+ `.tri` files churn lớn |

Lý do chọn **`khí + đạo`**:

1. **Đạo Đức Kinh là foundational philosophy ở Vietnam** — universally known từ giáo dục phổ thông. Lão Tử không yêu cầu "scholar tier" để recognize.
2. **Đạo §42 trực tiếp justify balanced ternary** — không metaphor nào khác mạnh hơn câu *"nhị sinh tam, tam sinh vạn vật"* cho 1 ngôn ngữ tam phân. ADR-0010 (ternary-native IR) có thể quote câu này làm epigraph.
3. **Khí §28 mapping compilation hoàn hảo** — phác (raw source) → khí (compiled vessel). Compiler là splitter; output là vessel chứa nội dung. Không metaphor English-rooted nào (`pack`, `bundle`, `archive`) đạt được depth này.
4. **CLI `dao build` SIGNALS bản sắc** — `dao` 3 chars, đọc gọn. Lo ngại nhầm với English "dao" (knife) được giải quyết bằng docs đầu trang. So với `cargo build` (English logistics, depth = 0) → `dao build` (Vietnamese philosophical core) là feature, không phải bug.

## §3 — Naming matrix (9 ô)

| # | Element | Trước | Sau | Note |
|---|---|---|---|---|
| 1 | Language name | Triết | Triết | KHÔNG đổi — danh tính ngôn ngữ là ổn định invariant |
| 2 | Source file extension | `.tri` | `.tri` | KHÔNG đổi — chữ `tri` báo hiệu (a) liên kết với "Triết", (b) ngôn ngữ tam phân |
| 3 | IR bytecode | `.triv` | `.triv` | KHÔNG đổi — internal artifact, không trên user surface |
| 4 | Compiled package artifact | `.khi` | **`.khi`** | Per §28 phác tán tắc vi khí |
| 5 | CLI tool binary | `triet` | **`dao`** | Đạo (the Way) — tool that performs phác→khí transformation |
| 6 | Manifest filename | `triet.package` | **`dao.package`** | Cohesive với CLI tool name |
| 7 | Lockfile | `triet.lock` | **`dao.lock`** | Cohesive với CLI tool name |
| 8 | Path keyword | `crate.foo.bar` | **`khi.foo.bar`** | Reference "this khí" — file đang nằm trong khí thì path bắt đầu từ khí |
| 9 | Reserved namespace roots | `std/sys/dev/usr/core/crate/self/super` | `std/sys/dev/usr/core/`**`khi`**`/self/super` | Per CLAUDE.md reserved-root list |

**CLI subcommands** — mixed primary + Vietnamese aliases:

| Primary (English) | Vietnamese alias | Origin |
|---|---|---|
| `dao build` | `dao tao` | tạo (create) |
| `dao check` | `dao kiem` | kiểm (verify) |
| `dao run` | `dao chay` | chạy (execute) |
| `dao store ...` | `dao kho ...` | kho (warehouse) |
| `dao fmt` | (không alias — `fmt` đã là 3 chars) | — |

Aliases ASCII không-diacritic per CLI usability (`dao tao` typeable on any keyboard layout). Implementation: `dao` arg parser accept cả 2, dispatch về cùng handler. Documentation list primary + alias side-by-side.

## §4 — Implementation: 5-stage commit series (per-step cadence)

Hard cutover (no transition period — v0.7 chưa có external users). Stage độc lập, mỗi stage tests xanh.

| Stage | Scope | Files đụng (estimate) |
|---|---|---|
| **A** | Path keyword `crate` → `khi` (lexer token + parser dispatch + SPEC §"Path keywords" + ~114 user-source + ~30 snapshots) | ~150 |
| **B** | Wire format `.khi` → `.khi` (pack serde + store paths + CLI args + docs) | ~50 |
| **C** | CLI binary `triet` → `dao` (Cargo.toml `[[bin]]` + README + tất cả snapshots match `triet …`) | ~80 |
| **D** | Manifest `triet.package` → `dao.package` + lock `triet.lock` → `dao.lock` (loader filename matcher + tất cả demo manifests) | ~30 |
| **E** | Vietnamese subcommand aliases (`dao tao/kiem/chay/kho`) — additive feature on top of stage C | ~10 |

Tổng = ~320 files modified across 5 commits + 1 ADR commit = 6 commits ship trước v0.7.10 mở.

**Stage ordering rationale:**

- **A trước** vì path keyword là user-facing nhất + ảnh hưởng Triết source code; nếu rollback phải thì rollback A một mình rẻ.
- **B trước C** vì `.khi` wire format không phụ thuộc CLI binary name; có thể test `dao build -o foo.khi` ở giai đoạn middle.
- **C + D** thường ship cùng nhau (CLI rename + manifest rename liên quan chặt) nhưng tách để diff dễ review.
- **E sau cùng** vì aliases là additive (không thay primary commands).

**Test invariant**: Sau mỗi stage, full `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` xanh. Per CLAUDE.md "Tests must be green before any commit".

## §5 — Backward compatibility: hard cutover

V0.7 chưa có external user / package registry / installed toolchain ngoài author's machine. Hard cutover hợp lý:

- Không support legacy `crate.foo.bar` import — typecheck reject với E2207-equivalent.
- Không support legacy `dao build` command — `dao build` exclusive.
- Không support legacy `triet.package` manifest — loader chỉ tìm `dao.package`.
- Không support legacy `.khi` reader — `read_tripack` xóa hoặc rename `read_khi`.

Migration tool (`dao fmt --migrate-khi`) cho user-facing source: optional, defer post-v0.7. Hiện chỉ author đụng, search/replace bằng tay hoặc 1 sed-based script là đủ.

## §6 — Liên quan

- [ADR-0005](0005-module-system.md) — Module system + path keywords (sẽ bị supersede phần `crate` keyword)
- [ADR-0010](0010-ternary-native-ir.md) — Ternary-native IR — quote Đạo §42 làm epigraph (defer, optional cleanup)
- [ADR-0011](0011-crate-pack-format.md) — Crate-pack format (rename "crate-pack" → "khí" trong description, no semantic change)
- [ADR-0014](0014-cas-packaging.md) — CAS packaging (terminology sweep "crate-pack" → "khí" pkg level)
- [SPEC.md §"Path keywords"](../../SPEC.md) — update reserved-roots list
- [VISION.md §3](../../VISION.md) — có thể add 1 paragraph về Đạo Đức Kinh framework grounding the naming

## §7 — Notes

- Lo ngại CLI confusion `dao build` ≠ English "dao" (knife): đã đánh giá, lean INTO Vietnamese identity. Docs đầu README + `dao --help` ghi rõ "`dao` (đạo, the Way) — Triết's build and package manager". Adoption proven by `cargo` / `gem` / `pip` precedent.
- Aliases ASCII (`tao` not `tạo`): không phải mọi user gõ được diacritic. Latin-only command alias là pragmatic choice. Documentation HIỂN THỊ diacritic version cho recognition.
- Future: nếu Triết có package registry, registry URL có thể là `dao.<TLD>` (e.g. `dao.triet.dev`). Cohesive branding.

---

*Quyết định này lock identity-level naming cho phase v0.7 onward. Breaking change ở bất kỳ §3 ô nào cần ADR mới supersede. Implementation v0.7.x.identity (5 stages) bắt đầu ngay sau ADR commit.*
