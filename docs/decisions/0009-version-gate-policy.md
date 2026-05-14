# ADR 0009 — Version gate policy: v0.4 entry requirements

**Trạng thái:** Quyết định. Áp dụng cho mọi version bump v0.x → v0.(x+1) kể từ v0.3 → v0.4. Lock thành nguyên tắc dự án.

**Issue:** Khi đóng phase v0.3 (commit `28e7da0`, ROADMAP § v0.3 "đã ship"), một số gate vẫn ở trạng thái partial:

- Differential VM ≡ interpreter: **3/11** (8 ignored với `#[ignore]`)
- VM bench: **1.26×** interpreter (gate đặt 3×)
- Cargo workspace version: vẫn `0.1.0` (không đồng bộ SPEC v0.3)
- Clippy warnings tích tụ: 109+ trong `triet-ir/lib` mặc dù `CLAUDE.md` yêu cầu *"fix every new warning"*
- TODO comments `TODO(v0.3.4)`, `TODO(v0.3.5)` còn sót trong code dù v0.3 đóng

Đây là drift giữa **"phase đóng"** (theo ROADMAP) và **"phase thực sự sạch"**. Nguyên tắc `stability over speed` (VISION § 6) yêu cầu ngược lại: phase N không thể đóng nếu gate chưa đạt 100%.

ADR này lock một **gate policy bất khả khoan nhượng** cho mọi version bump tương lai. Không phải để punish v0.3 retroactively — mà để định nghĩa rõ "đóng phase" có nghĩa là gì, tránh repeat cùng pattern khi đi v0.4 → v0.5.

## Quyết định

Một phase v0.N **chỉ có thể đóng** (và mở v0.(N+1)) khi **toàn bộ** điều kiện sau đạt **đồng thời**:

### Gate A — Functional completeness

| Tiêu chí | Đo bằng |
|---|---|
| Tất cả deliverables liệt kê trong ROADMAP § v0.N status = ✅ | Manual cross-check |
| Mọi `gate đặt N×` numerical trong ROADMAP đạt hoặc vượt | Reproducible benchmark hoặc test |
| Không còn `#[ignore]` hoặc `#[allow(...)]` mới được thêm trong phase này | `grep -r "#\[ignore\]\|#\[allow" crates/` |
| Không còn `TODO(vX.Y)` với version ≤ N trong source code | `grep -rn "TODO(v" crates/` |

### Gate B — Code hygiene

| Tiêu chí | Đo bằng |
|---|---|
| `cargo test --workspace` xanh, 0 ignored không có lý do ghi rõ | CI |
| `cargo clippy --workspace --all-targets -- -D warnings` sạch | CI |
| `cargo fmt --all --check` sạch | CI |
| Không có file source > 2000 dòng (signal để tách module) | `find crates -name '*.rs' \| xargs wc -l \| awk '$1>2000'` |

### Gate C — Documentation sync

| Tiêu chí | Đo bằng |
|---|---|
| `SPEC.md` tiêu đề ghi đúng version v0.N | Manual |
| `ROADMAP.md` § v0.N có đầy đủ sub-task changelog với commit hash | Manual |
| `README.md` status, test count, workspace structure khớp thực tế | Manual diff vs `cargo test --workspace 2>&1 \| grep "test result"` |
| `Cargo.toml workspace.package.version` = `0.N.0` | `grep version Cargo.toml` |
| `triet info` CLI subcommand in đúng version | `./target/release/triet info` |
| ADR cho mọi quyết định kiến trúc lớn của phase đã được merge | Manual cross-check |

### Gate D — Self-consistency

| Tiêu chí | Đo bằng |
|---|---|
| Tất cả `.tri` files trong `examples/` chạy được qua tree-walker | `for f in examples/*.tri; do triet run "$f"; done` |
| Tất cả `.tri` files trong `demos/` chạy được qua tree-walker | Idem |
| Mọi feature đã đặc tả trong SPEC được test ít nhất 1 lần | Manual cross-check SPEC chapters |

## Áp dụng cho v0.4 entry

Trước khi mở **bất kỳ** sub-task `v0.4.x`, các điều kiện sau **phải** đạt:

1. **Differential VM ≡ interpreter: 11/11 byte-identical** trong `crates/triet-cli/tests/differential_tests.rs`. Hiện tại 3/11 — 8 `#[ignore]` phải được resolved (không phải bằng cách xoá test, mà bằng cách hoàn thiện lowerer + VM).
2. **VM bench gate**: ROADMAP § v0.3 đặt 3× — nếu sau cleanup vẫn không đạt, **không bypass**. Hai option hợp lệ:
   - Hoàn thiện optimization đến khi đạt 3×, **HOẶC**
   - Viết ADR-0010 (revise) hạ gate về số đo được, ghi rõ lý do (VM là development tier per VISION § 4.3, không phải production runtime).
3. **Cargo version** bump lên `0.3.0` đồng bộ với SPEC v0.3.
4. **README** ghi đúng v0.3 status.
5. **Clippy sạch** với `-D warnings` workspace-wide.
6. **0 TODO(v0.3.x)** trong source code.

Nếu một item không đạt được trong thời gian hợp lý, phải có ADR ghi quyết định defer (như ADR-0010 ví dụ trên), không phải im lặng skip.

### Mapping cụ thể cho v0.3.x.cleanup phase

| Sub-task | Gate item |
|---|---|
| v0.3.x.cleanup.1 (ADR-0009 này) | Lock policy |
| v0.3.x.cleanup.2 (Cargo version bump) | Gate C |
| v0.3.x.cleanup.3 (README sync) | Gate C |
| v0.3.x.cleanup.4 (Clippy fix) | Gate B |
| v0.3.x.cleanup.5–8 (Lowerer: enum, while, iterator, Long) | Gate A (gỡ 8 `#[ignore]`) |
| v0.3.x.cleanup.9 (Verify) | All gates pass simultaneously |

## Hệ quả

- **Pace chậm hơn**: Đóng v0.3 thật sự = 6–12 tháng (như ROADMAP ước tính) chứ không phải "ship đại v0.3 với 3/11 differential, fix sau". Nhưng đây chính là cam kết của VISION § 6.
- **v0.4 ABI thiết kế trên IR đã ổn định**: 11/11 differential pass = bằng chứng IR + lowerer + VM consistent cho mọi feature v0.2 features. ABI metadata (v0.4) sẽ encode IR shape; nếu IR/lowerer còn gap, ABI sẽ encode gap đó → buộc redo ABI sau.
- **Không có "v0.3.5", "v0.3.6" tích lũy mãi mãi**: gate giữ phase đóng được thật. Sub-task `v0.3.x.cleanup` là exception duy nhất — hợp lệ vì nó *retroactively* đóng gate v0.3 trước khi mở v0.4, không phải thêm feature mới.
- **AI-as-collaborator**: gate policy này dễ check bằng grep + cargo. Một AI assistant có thể self-verify "phase đã đóng chưa" mà không cần phán đoán mơ hồ.

## Không làm

- **Không yêu cầu** 100% code coverage (chỉ feature coverage).
- **Không yêu cầu** zero clippy lint *suggestions* (chỉ zero `warn`-level).
- **Không yêu cầu** binary backward compat (chưa đến v1.0).
- **Không** áp dụng retroactively cho v0.1, v0.2 — chỉ từ v0.3 → v0.4 trở đi.

## Prior art

- **Rust release process** — feature freeze + beta + stable, mỗi cửa có gate rõ.
- **TC39 stage process** (JavaScript) — stage 4 yêu cầu 2 implementation + spec tests pass.
- **Linux kernel merge windows** — Linus enforce rule: regression test phải pass trước khi merge feature window đóng.

## Tham chiếu

- [VISION § 6 — Stability over speed](../../VISION.md)
- [ROADMAP § v0.3 — Sub-task changelog](../../ROADMAP.md)
- [ADR-0007 — IR design](0007-ir-design.md)
- [ADR-0008 — `.triv` binary format](0008-triv-binary-format.md)
