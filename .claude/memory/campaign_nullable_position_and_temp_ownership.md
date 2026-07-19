---
name: campaign_nullable_position_and_temp_ownership
description: "✅ ĐÓNG 2026-07-19 — 5 WO liên tiếp: họ 'match exact, quên Nullable' ở 3 VỊ TRÍ (Enum? param · Struct? param · Struct? return) + INV-HeapNullable probe (SOUND, đập doc-comment nói dối) + ShimTempOwnership (rỉ câm CẢ MẢNG shim-mượn). origin/main aa9e584, gate 0·0·439·0. Phát hiện lớn nhất: SPOF arg_consumes."
metadata:
  node_type: memory
  type: project
  originSessionId: 9f55d317-ec4f-4bce-bded-eac47a8223a5
  modified: 2026-07-19T16:22:40.715Z
---

## ✅ ĐÓNG — 5 WO, tất cả O ✅ + G ✅, đã PUSH

```
aa9e584  docs(todo): refresh handoff header + SPOF debt
72a0bd6  WO-ShimTempOwnership ĐÓNG (8 commit 04b6174…72a0bd6)
c88832a  WO-INV-HeapNullable-Probe (a) — đập doc comment nói dối
645ae61  WO-StructReturnRefuse (e7aab8c fix + fixtures 437-445)
ec7ecd8  WO-StructParamABI  (7d59b7c fix + fixtures 428-436)
ccb8db3  WO-NullableEnumParamABI (fixtures 419-427)
```
Gate cuối `0·0·439·0 CLEAN`. Fixture 419→445 + 4 file counting mới.

## 🧬 SỢI CHỈ: họ **"match exact, QUÊN `Nullable`"** — 4 thành viên

| # | Site | Triệu chứng | Trạng thái |
|---|---|---|---|
| ① | `Enum?` param copy-in (`mir_lower.rs` match `MirType::Enum` exact) | **rác câm**, nhánh `~0` chết trên mọi biên gọi | vá `ccb8db3` |
| ② | `Struct?` param bare-read (`load_place:1248-58` không slot → `use_var` = con trỏ) | **rác câm** | vá `7d59b7c` |
| ③ | `Enum?` return-shape | — | đã có refuse từ trước |
| ④ | **`Struct?` return-shape** (`is_struct_return = matches!(ret, MirType::Struct(_))`) | 4 hố: câm · rác địa chỉ · SIGILL 132 · **SIGABRT 134** | POLICY GATE refuse `e7aab8c` |

⚠️ **④ nằm cách một comment do CHÍNH O viết phiên trước đúng 10 dòng** — comment ấy gọi tên hiện tượng là *"P0-sibling gap"*, vá ① anh em rồi bỏ sót anh em còn lại.

## 🔑 CƠ CHẾ CHUNG
- **param**: Variable giữ **con trỏ** tới slot caller → sentinel-compare so địa chỉ với `i64::MIN` → **luôn "present"** ⇒ nhánh null chết. Field-read vẫn ĐÚNG (đọc xuyên con trỏ) → chỉ tag-read hỏng.
- **return**: hai nhánh sinh repr **KHÔNG tương thích** cho cùng kiểu — null → `const NULL_SENTINEL` (scalar), present → `struct P{..}` **TRẦN, không tag**. Cả tag-read LẪN field-read hỏng.
- **Tiền lệ chạy đúng:** `Integer?`→`Scalar` (đúng, sentinel vừa i64) · `String?`→`Struct` fat/sret qua `is_string_repr()` — predicate này **cố ý bao cả wrapper `Nullable`**. Đó là mẫu đúng mà `is_struct_return` thiếu.

## 🩸 RỈ CÂM CẢ MẢNG SHIM-MƯỢN (WO-ShimTempOwnership)

Lộ ra **nhờ** hạ tầng counting của WO trước, không nhờ đọc code.

```
length(h.name)            FREE=0 RI     userfn f(h.name)     FREE=1 OK  <- bac gia thuyet "temp-lifetime chung vo"
length(o.inner.name)      FREE=0 RI     length(s) local      FREE=1 OK
length("hello")           FREE=0 RI  <- KHONG co field-access -> giet ten "InlineFieldTempLeak"
concat 3->1 · contains 2->0 · eq 2->0   (moi ca that thoat dung 2 temp)
push/insert (TIEU THU)    FREE=1 ca inline lan let-bound -> LANH, la CONTROL
```
**Đặc tả đúng:** temp **vô danh** (field-access HOẶC literal) làm arg cho builtin **mượn** không bao giờ `push_owned` → không ai drop. `let` thì lành (đăng ký qua let), user-fn thì lành (chuyển sở hữu qua `Deinit`, ADR-0042 Q1).

**Fix:** chokepoint `emit_shim_call` tra `arg_consumes` (mượn/thiếu-entry → `push_owned`; tiêu thụ → cấm) + fast-path `length()` vá riêng. **Bán kính rộng hơn phạm vi ký** (quét cả `remove`/`get` key) — G duyệt giữ rộng: *"thu hẹp = viết `if name=="remove" { tiếp_tục_rỉ_nhé() }`, đó là NGU XUẨN, cố tình sinh sibling gap"*.

**⚠️ Oracle `hashmap_string_key_struct_value_remove_frees_key_and_value` 2→3:** giá trị cũ **ghim trên baseline ĐANG RỈ**. O verify **pointer-identity `frees=3 distinct=3 dup=0`** ⇒ KHÔNG double-free. Ai lùi về 2 là tái mở leak.

## 🔴 NỢ LỚN NHẤT ĐỂ LẠI — **SPOF `arg_consumes`**

`builtin_shim_meta().arg_consumes` được đọc bởi **CẢ HAI** tầng: `push_owned` (lowerer `emit_shim_call`) + **M3 zero-on-consume** (JIT `mir_lower.rs:4717`).
⇒ **KHÔNG phải defense-in-depth — là MỘT quyết định áp hai tầng.** Một entry khai láo thủng cả hai:
- khai **mượn** mà thực **tiêu thụ** → leak
- khai **tiêu thụ** mà thực **mượn** → double-free
- **cả hai CÂM** ở tầng giá trị. `contains` không có entry → rơi mặc định ngầm.

**Chưa có răng nào canh bảng này.** Hướng: unit test quét toàn bảng đối chiếu chữ ký shim thật.

## ⚖ O SAI 11 LẦN — CÙNG MỘT GỐC: **hành động trước khi đo**

1-9. Khái quát từ MỘT biến quan sát được: exit-code làm oracle (6 control gắn ✅ nhờ may) · `Struct?` param "lành" từ một ô · `T7 refuse ✅` từ một dạng khởi tạo (`~+`, sót `~0`) · đặt tên bug theo field-access (P6 `length("hello")` giết tên đó) · "hố nhỏ ở `length`" (số đo D bác: cả mảng).

**10. Dán nhãn failure-mode SAI:** ghi "SIGILL 132" cho `Struct?` param; đo lại 5/5 → đọc **HAI** field = SIGILL (rác+rác vượt ngưỡng → trap **ADR-0044**, **THỨ CẤP**), đọc **MỘT** field = rác câm. **Rác câm là gốc; SIGILL là tiếng sấm.**

**11. NẶNG NHẤT — thiết kế TIÊU CHÍ NGHIỆM THU theo cơ chế GIẢ ĐỊNH.** O tuyên *"poison ngược không nổ ⇒ reject"*. Ép đo hai chiều:
- M3 **bật** + bỏ phân biệt → FREE=1 **không nổ** (D đúng)
- M3 **tắt** + phân biệt đúng → **SIGABRT double-free**
⇒ **M3 mới là lớp chịu lực**; nhánh `!consumed` bị che. **O rút tiêu chí.** Nếu giữ nguyên, O đã **bác một fix ĐÚNG** và ép D sửa cái không hỏng.
🔑 **Chính mũi poison ĐẶT SAI CHỖ đó lôi ra SPOF** — thất bại có kỷ luật sinh ra phát hiện.

## 🦷 LUẬT RĂNG MỚI (khắc vào persona)
- **Oracle cũng là giả định — phải verify.** exit-code không đo giá trị; **giá trị không đo leak**; **FREE-count không phân biệt 3-object với double-free** (phải dedup con trỏ).
- **Răng phải chứng minh ở TẦNG HARNESS.** `integration_test_corpus()` là MỘT test chạy vòng lặp ⇒ một fixture crash giết cả tiến trình, mọi fixture sau **không bao giờ chạy**. "Suite đỏ" KHÔNG chứng minh răng của mình. Cách chứng minh: đổi `EXPECT`/`ERROR` sang giá trị bịa → phải ra `FAIL <tên>: expected …, got …`.
- **Test xanh có thể đang canh giữ hiện trạng SAI** (oracle ghim trên baseline rỉ). Fix đúng làm nó đỏ — **cấm sửa oracle cho xanh mà không có bằng chứng độc lập**.
- **"Poison không đỏ" phải ép tới cùng**: gỡ lớp che (tắt M3) rồi đo lại, đừng kết luận từ happy-path.

## Ghi chú vai — D (Sonnet 5)
**Bác O 8/8 lần, đúng cả 8.** Vết kỹ thuật: **0 vết bịa**. Điểm sáng lớn nhất: **dừng khi test đỏ, KHÔNG tự sửa oracle 2→3 cho xanh** — hành vi ngược lại sẽ ship double-free dưới vỏ "đã cập nhật kỳ vọng".
Vết còn lại = **kỷ luật vòng lặp**: 4 lần vi phạm luật foreground (một lần **bị O trả WO**), 3 lần để việc treo không commit. 🔑 **Luật phải cấm HÀNH VI, không cấm CÔNG CỤ** — cấm `run_in_background` thì D lách bằng `Monitor`; cấm *"kết thúc lượt khi chưa cầm output"* thì không lách được.

## Nợ còn treo
🚩 **SPOF `arg_consumes`** (trên) · 🚩 **ADR "Full SRET cho Nullable Aggregate"** (gỡ CẢ hai policy gate `Struct?`+`Enum?` return; phải vá widen-path — present arm không ghi tag — LẪN sret sizing `{tag@0,fields@8+}`) · `key_marshal` >8B param (**over-refuse to tiếng, KHÔNG phải UB** — O đo hạ ưu tiên) · lỗ N1 `~0` bypass (policy-hole) · `is_empty` · `HashMap<String,V>` key-position.

[[campaign_nullable_enum_aggregate_pa_a]] [[campaign_borrowck_nll_foundation]] [[feedback_failure_mode_precision]] [[feedback_poison_must_be_red]] [[mentor_o_persona]] [[colleague_d_persona]]
