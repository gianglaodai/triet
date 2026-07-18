---
name: campaign_nullable_enum_aggregate_pa_a
description: "✅ ĐÓNG 2026-07-18 — WO-NullableEnumAggregate-Refuse (PA-A): vá silent-miss SỐNG `Nullable(Enum)` tra nhầm struct_map trong co-fixpoint sizing. G phán ĐẢO VAI: N3=soundness fix, N1=policy gate. D bác WO của O lần 5/5 — đúng. Nợ mới: `Enum?` param ABI SIGILL 132. origin/main 5c713c4, gate 0·0·412·0."
metadata:
  node_type: memory
  type: project
  originSessionId: 78c71263-44c7-40b2-be94-2fae740e93dd
---

## ✅ ĐÓNG — 2 commit PUSHED (O+G ký, 2026-07-18)

```
5c713c4  docs(todo): close PA-A, record G ruling, log Enum? param ABI debt
186bd1c  fix(track-c): refuse payload-bearing nullable enum in aggregates (PA-A)   gate 0·0·412·0
```

## 🔑 BUG — silent-miss SỐNG, KHÔNG phải mìn hẹn giờ

⚠️ **Sổ cũ ghi SAI hai chỗ, đã đính chính:** tọa độ **`triet-lower/src/lib.rs:567`** (không phải `:503` — số dòng trôi), và nó **KHÔNG "bất-khả-observable"** — là bug **SỐNG**, chạm được bằng 5 dòng Triết hợp lệ:

```triet
enum E { V(Integer), N }
struct Mid { e: E?, m: Integer }
Mid { m: 5, e: E::V(42) }   //  mid.m đọc ra 42, KHÔNG phải 5.  exit 0.
```

**Cơ chế:** `:632` seed field = 8B; co-fixpoint tồn tại CHỈ để sửa seed. Nhánh `Nullable(Enum)` tra **`struct_map`** — nơi enum **không bao giờ** được đăng ký (comment của chính hàm đó nói vậy, cách 2 dòng ở nhánh `MirType::Enum`) → **luôn MISS → fallback → fixpoint là NO-OP**. Enum payload-bearing 16B (disc@0+payload@8) nhét slot 8B ⇒ tràn đè field kế.

**Vì sao sống sót:** không phải bị nhốt — **ZERO COVERAGE**. Gate CLEAN 407 cả khi cắm fix. Refuse cũ ở `Expr::OutcomeConstructor` chỉ canh `~+`/`~0`; **gán enum trần vào field `E?` (widening ngầm, `Expr::EnumVariant`) đi vòng qua lồng.**

⚰️ **Nợ 13/07 "Enum-Payload-Aggregate Sizing Fix" KHÔNG chung số phận** — đã đóng bởi `9a1799c` (ADR-0067 §AMEND co-fixpoint, 16/07). Cái vừa vá là **bug SIBLING** trong cùng hàm. Gạch khỏi sổ.

## ⚖ G PHÁN QUYẾT — ĐẢO VAI N1/N3

| | Vai | Bằng chứng |
|---|---|---|
| **N3** (`:567` `struct_map`→`enum_map`) | **SOUNDNESS FIX** — fix THẬT | gỡ N1 giữ N3 → **không shape nào corrupt** |
| **N1** (quét tầng khai báo, `lower_program` sau fixpoint) | **POLICY GATE** — chặn chờ ADR-0065 bless repr | **KHÔNG** quan sát được failure-mode nào để poison đỏ |

**🚫 CẤM về sau viện N1 như bằng chứng đã đóng một đường UB.** G giữ N1 tuyệt đối vì bề mặt `Enum?` còn vỡ chỗ khác (SIGILL 132 dưới) — "mở bề mặt khi chưa chuẩn bị kỹ là tự sát".

**O verify độc lập 6 shape:** aggregate payload 24B · đọc ngược chính field nullable · enum-payload-nullable lồng · 3 shape heap (heap bị `heap_type_not_supported` chặn, **không phải** N1). Tất cả đúng.

## 🦷 N3 CÓ RĂNG DUY NHẤT — và vì sao phải có

N1 chặn MỌI đường fixture-level chạm N3 ⇒ N3 sẽ thành **code ma**: ngày mở ADR-0065 gỡ N1, ai lùi `enum_map`→`struct_map` thì bug sống lại **câm, gate vẫn xanh**. G duyệt đòi hỏi này của O.

Răng = unit test `resolve_aggregate_size_nullable_enum_reads_enum_map_not_struct_map` (`lib.rs` mod tests, gọi được private fn cùng module). **O verify độc lập:**
- đảo token → **ĐỎ** (`left: 8 / right: 16`)
- **đặc hiệu**: poison site sinh đôi bare-`Enum` `:570` → test **VẪN XANH**
- site sinh đôi `:570` cũng **có răng riêng** → `enum_field_moveout_frees_once_with_cap` đỏ khi poison. **Cả hai site đều được canh.**

## 🔴 NỢ MỚI — `Enum?` PARAMETER ABI VỠ (SIGILL 132)

D khui, O verify. `function pick(u: U?)` với `U` **unit-only**: `pick(a)` → exit **132** cả arm present (`~+ U::A`) LẪN arm null (`~0`); `SwitchInt` rơi default-arm Trap. **Pre-existing** — tái hiện trên pristine `564f0f7`. Nghiêm trọng vì unit-only `Enum?` là thứ **đang được cho phép** (417/418 canh field/local, **KHÔNG canh param**); 412 fixture không cái nào chạm `Enum?` ở vị trí parameter. **Cần WO riêng.**

## 🩸 BÀI HỌC O TỰ ĂN (khắc)

**★ GIAO THỨC POISON PHẢI ĐỐI CHIẾU NGƯỢC VỚI SỐ ĐO CỦA CHÍNH MÌNH.** O đo control-biến đầu phiên (`:567` đổi một token → p1/p2/p3 ra **7** ⇒ N3 một mình đã fix), rồi vài giờ sau soạn WO viết teeth *"gỡ N1 → thấy 42"* — **hai mệnh đề không thể cùng đúng**. Không thiếu dữ liệu, **không nhảy số**. D bắt được cái O không bắt.
⇒ Verify-don't-trust trước nay áp cho *kết luận*; nay áp cả cho **thiết kế test**.

**★ D BÁC O LẦN 5/5 — đúng cả 5.** Xem [[colleague_d_persona]].

**★ Deadlock subagent:** D chạy `gate.sh` ở **background** rồi dừng lượt chờ notification → subagent dừng lượt = kết thúc, notification KHÔNG BAO GIỜ tới. Kẹt 2 lượt (~40 phút). ⇒ **Subagent phải chạy lệnh cần kết quả ở FOREGROUND + timeout dài.** Ghi công D: kẹt mà **không bịa `(all pass)`** để thoát.

**★ O KHÔNG chạy gate hộ khi D chưa nộp raw gate** — giữ được luật, khác vụ 2026-06-11 bị G chém vì nhượng bộ thủ tục 3 lần.

## Ghi chú vai
Model D = Sonnet 5. Vết phiên này: **không có vết bịa** (failure-mode SIGILL 132 mô tả CHÍNH XÁC — ngược vết cũ "bịa SIGSEGV khi thực tế là leak"). Vết duy nhất: bế tắc không hỏi (LUẬT 4).

[[campaign_borrowck_nll_foundation]] [[campaign_aggregate_nullable]] [[feedback_poison_must_be_red]] [[feedback_failure_mode_precision]] [[mentor_o_persona]] [[colleague_d_persona]]
