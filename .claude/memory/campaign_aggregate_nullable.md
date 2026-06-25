---
name: campaign_aggregate_nullable
description: ✅✅ TRỌN BỘ ĐÓNG — Chiến dịch ADR-0065 Nullable Aggregate (Enum?/Struct?). Lát 1 Enum? (e71f396) + Lát 2 Struct? (f83a8f7) đều push origin. Chuỗi Nullable hoàn tất. ĐỌC nếu đụng lại aggregate-nullable hoặc nợ defer (heap-in-aggregate).
metadata:
  node_type: memory
  type: project
  originSessionId: aggregate-nullable-campaign
---

**Chiến dịch ADR-0065 Nullable Aggregate** — `Enum?`/`Struct?` (nullable stack-slot). ADR `docs/decisions/0065-aggregate-nullable.md` 🔒 LOCKED (O+G ký, Giang chốt 2026-06-20). Trả nợ defer ADR-0062 §6 (Struct?/Enum? "không có ô ptr tự nhiên").

## Bất biến hợp nhất (mở rộng ADR-0062 §2)
`tag_cell == NULL_SENTINEL (i64::MIN) ⟺ null`. `tag_cell` = ô ptr (heap, ADR-0062) **HOẶC** ô **disc@0** (Enum? niche) **HOẶC** ô **tag@0** (Struct? disc-word prepend). Null-check = 1 load + 1 `icmp eq i64::MIN`, CẤM `==0` (0=uninit/dead).

## ⛔ RÀO B8 (ADR §4, in đậm bôi đỏ) — KHẮC ĐÁ
Aggregate-nullable CHỈ chứa **Copy field/payload**. KHÔNG drop-glue, KHÔNG alloc/free, KHÔNG đụng allocator. Heap field/payload (String/Vector/HashMap) GIỮ refuse. Value-model i64 KHÔNG đụng (leaf I64; chỉ mở rộng slot-layout, cùng họ Outcome/nested-aggregate).

## Bất đối xứng cốt lõi (O recon đo file:line)
- **Enum = DỄ:** `EnumLayout` đã có disc@0 (i64 full, giá trị ∈ {0,1,2,…}) → niche khổng lồ. disc@0==i64::MIN=null. Widening no-op. 0 byte.
- **Struct = KHÓ:** `StructLayout` = N field inline, KHÔNG ô disc/ptr. Phải đẻ tag word@0 (+8B). Widening KHÔNG no-op.
- Loại **B (box)** = đụng allocator/Move/drop-glue, phá B8. Loại **C (niche-fill)** = type-dependent, Rust mất nhiều năm ổn định (G chốt "trò khốn nạn compiler non trẻ dây vào"). **Chọn A (disc-word) cho Struct?.**

## ✅ Lát 1 — Enum? ĐÓNG + PUSH (origin `e71f396`, 2026-06-20)
4 commit: `015061c` ADR LOCKED · `1748510` feat Enum? · `e9bd3e0` ADR §9.1 · `e71f396` TODO. Gate `0·0·225·0`.

**5 production delta (D xóa delta-D dead-code theo Rule #7 — `ty_total_size` Nullable-arm unreachable vì caller qua walk_projections đã unwrap):**
- **A gate** `triet-mir:1399` `is_lowerable_nullable_payload += matches!(MirType::Enum(_))`. Field/payload gate (1500/1513) GIỮ scalar-only = B8.
- **B slot-alloc** `triet-jit mir_lower.rs:955-972` loop: cấp StackSlot cho mọi Enum/Nullable(Enum) local dẫn xuất (match-bind/~0/result) chưa qua EnumAlloc (else resolve_addr fallback use_var = con trỏ rác). Unwrap-tại-site `nullable_payload().unwrap_or` (KHÔNG đẻ predicate, mẫu Lát 4.8).
- **C walk_projections** `mir_lower.rs:256` unwrap Nullable → projection resolve trên inner Enum.
- **E result-retype** `triet-lower lib.rs` 2 site (null-arm `lower_arm_no_bind` ~3471 + present ~3531): `result.ty = body_val.ty` (idiom ADR-0056). payload_ty-pin SAI tiềm ẩn cho MỌI type, chỉ lộ khi payload >8B (aggregate-copy coi scalar làm con trỏ → SIGSEGV). GIỮ ở code dùng chung (O ruling Q1: CẤM tách nhánh Enum-riêng = cố giữ đường biết-sai cho scalar/heap).
- **F `~0` materialize** `mir_lower.rs:1229-1232`: store i64::MIN vào enum slot disc@0 (KHÔNG iconst single-i64 như scalar — điểm khác cốt lõi).

**Fixtures 225-230:** 225 present payload-less (8B, extra) · 226 present payload Box{Full(7)}→7 (CORE multi-word) · 227 ~0 null→99 · 228 Elvis→7 · 229 widening Box→Box?→5 · 230 B8 `Has(String?)` refuse `HeapNullableNotLowered`.

**O verify máu (poison độc lập, RED→GREEN):** E poison cả-2-site→226 SIGSEGV139 · B poison slot-loop→226 SIGSEGV139 · F poison ~0-store→227 Trap132 (226 vô can). D-removal verify dead-code an toàn.

**Bài học gác cổng:**
- **Teeth dối phơi bằng poison:** vòng 1 D nộp CHỈ fixture 225 payload-less (8B) → poison E vẫn XANH (8B đi single-word-copy, không chạm multi-word) = teeth giăng chỗ không cá. O dựng enum CÓ payload (Box{Full(Int)}, >8B) → poison E → SIGSEGV → E load-bearing THẬT. Mẫu HP.3 blind-spot + #14 vacuous-teeth.
- **O tự ăn 2 lỗi đo của chính mình:** (1) tưởng E vacuous vì poison-trên-225 không đỏ — sai, do 225 8B; (2) `exit=$?` bắt nhầm exit của `tail` → redirect file. Verify-don't-trust áp cả thao tác đo của O.
- **ADR §9.1 amendment (rule #5):** B8 refuse qua 2 cổng khác mã lỗi — `Has(String?)` nullable-heap → `HeapNullableNotLowered` (guard lát này, fixture 230); plain `Has(String)` → is_copy construction gate ADR-0040 (orthogonal). Teeth nhắm đúng cổng String?.
- **D tiến bộ:** tự xóa dead-code (Rule #7), khai thật blind-spot E2/E3 mutual-redundant (mỗi site đủ một mình; cơ chế retype mới load-bearing) thay vì claim độc lập.

## ✅ Lát 2 — Struct? (tag-word prepend, Phương án A, β) ĐÓNG + PUSH (origin `f83a8f7`, 2026-06-20)
4 commit: `d8c3567` ADR §9.2 · `4b6899f` feat (3 src) · `8d82c64` fixtures 231-237 · `f83a8f7` TODO. Gate `0·0·232·0`. Slot `{tag@0:i64, fields@8…}`, total = struct.total_size+8. tag@0==i64::MIN=null | +1=present.

**6 delta:**
- **Delta 0 (LOWERER — recon-miss của O, vá in-scope, ADR §9.2):** `let x: Struct? = y` ở `triet-lower lib.rs:1207` MẶC ĐỊNH retype-in-place + alias → đó CHÍNH là lý do Enum? Lát 1 no-op (niche cùng slot). Struct? phá vì +8B: in-place giữ slot 16B cũ → walk+8 OOB → 231 trả 6. Sửa: `init==Struct(_) && ann==Nullable(Struct(_))` → fresh local + `Assign{new←v}` (M2 pattern, TODO `1200-1206` đã tiên tri). Khoanh CHẶT Struct→Struct?; Enum?/scalar/String? giữ in-place (229 xanh).
- **1 gate** `triet-mir:1402` `is_lowerable += matches!(Struct(_))`. Field/payload gate (1507/...) GIỮ `is_scalar` = B8.
- **2 slot-alloc** `triet-jit`: loop Struct/Struct? — `Nullable(Struct)→total_size+8`, plain→+0; skip sret/param (pointer-based, reserved_locals) + "String".
- **3 walk_projections** `+8` cho `Nullable(Struct)` base qua helper `nullable_struct_base_offset` (downcast payload-extract).
- **4a widening** store tag=1 + copy N fields src+0→dest+8 (explicit, KHÔNG nhúng scalar path dù N=8).
- **4b β whole-slot** `T?→T?`: copy N+8 **tag-first** (propagate null/present verbatim — G ÉP β, refuse=tự thiến value-model). Kích qua reassignment (`let mutable b; b=a`), KHÔNG qua let (let=alias, đúng vì Copy).

**Lệch-lệnh chuẩn thuận (O verify):** `is_aggregate` + slot-loop skip `Struct("String")` — borrowck builder (`lib.rs:~187`) build MỌI named type thành `MirType::Struct(name)`, String-local là `Struct("String")` slot-less → force aggregate = deref param-ptr SIGSEGV. Khớp precedent is_string_repr. KHÔNG nới B8.

**Fixtures 231-237:** 231 widening present→7 · 232 ~0→99 · 233 Elvis→7 · 234 β T?→T? present (reassign)→5 · 235 ⚔β T?→T? NULL→7 · 236 ⚔B8 Bad{String?} refuse · 237 ⚔ tag-store P3 (reassign-widen-over-null, slot tái-dùng MIN).

**O verify máu (poison độc lập P1-P5, RED, khôi phục byte-identical mỗi phát):** P1 walk+8→231:7→4,234:5→1 · P2 4a-1word→SIGILL(y rác→tràn ADR-0044) · P4 4b-tag→234/235→-1 · P5 B8 gate→236+180. **P3 tag-store VACUOUS trên 231-236** (slot tươi uninit≠MIN) → **O bắt, dựng probe 237 reassign-widen-over-null** → REJECT 1 vòng → D thêm 237 → P3-final 237→-1 (231 vẫn 7) = răng duy nhất.

**Bài học gác cổng:**
- **O tự ăn recon-miss:** giả định "widening sinh Assign" không verify → Delta 0 thiếu trong WO gốc. Vá in-scope, β/B8 không đổi. Bài học: verify CƠ CHẾ lowerer (in-place vs Assign) TRƯỚC khi viết WO JIT.
- **Bắt vacuous-teeth của D (P3):** D self-poison chỉ P4, bỏ sót P3; fixture slot-tươi không bắt tag-store. Mẫu #14 vacuous-teeth — teeth widening-tag PHẢI dùng slot tái-dùng-null. O dựng probe độc lập chứng minh trước khi reject.
- **D tiến bộ:** dừng đúng Luật 4 khi vấp lowerer (không tự sửa, hỏi O); tự khai 2 bug + 1 lệch-lệnh kèm data.

## ✅ Lát 3' (RE-SCOPE) — Nested Nullable Aggregate Copy (Trục A) ĐÓNG + PUSH (origin `04beac8`, 2026-06-20)
5 commit: `f4af620` ADR §12.7 · `5a52b13` JIT (+mir gate) · `75a6aa2` lowerer · `e6f0418` fixtures 245-250 · `04beac8` TODO. Gate `0·0·245·0`. **Trục A TRỌN BỘ HOÀN TẤT.**

**WO gốc "Trục A" (Ca1 `Holder{p:Point?}`) under-scope — O recon-miss lần 2 (cùng họ Delta 0):** viết "tái dùng widening 4a" SAI — 4a/4b gate `projection.is_empty()` HAI bên = top-level only; field-position construction (dest projected) + readback (source projected) CHƯA TỪNG implement. G ép re-scope, KHÔNG bàn lùi.

**3 bug O trace (dump MIR, D báo THIẾU — chỉ thấy bug A):**
- **A (JIT):** `nullable_struct_base_offset` (+8) bake mù trong `walk_projections:297`. load_place/store_place empty-proj đọc slot@0 thẳng (KHÔNG walk → top-level 231-237 đúng). NHƯNG Assign-copy (1477/1478) gọi walk 2 side → bare Nullable(Struct) bị +8 trong whole-move → tag MIN nuốt (null→rác, readback lệch).
- **B (LOWERER):** `~+ Point` → `Expr::OutcomeConstructor` dùng `c.sig.return_type` (=Integer main) → `OutcomeAlloc non-Outcome Integer`. D claim "Lát 5 compile sạch" — SAI (chỉ compile-Rust, MIR nôn rác).
- **C (LOWERER):** implicit `Point{}` field → plain Assign KHÔNG set-tag → present **pass-by-luck** (tag rác≠MIN). Delta 0 `is_struct_widening` chỉ ở let-path, không field.

**Giải pháp (G ký, option a — giết "chắp vá", KHÔNG đắp thêm): Taxonomy 4-case.** Bỏ base-downcast → `walk_projections` faithful (total_offset=0, `nested_nullable_shift` mid-walk Struct+8/Enum+0). XÓA Delta 4a/4b → `nullable_struct_taxonomy` dispatch (src_ty,dest_ty) giữ Nullable wrapper:
- **WholeCopy** N+8 tag-first (Nullable←Nullable; = 4b + construction + readback)
- **Widen** tag=1+fields→+8 (Nullable←plain Struct; = 4a + field implicit)
- **Downcast** fields src+8→dest (plain Struct←Nullable; = match-bind, +8 NAY tường minh)
- Enum? KHÔNG match taxonomy (niche 0-byte → general-copy đúng).

**O verify máu (4 poison ĐỘC LẬP, observable, restore byte-identical):** case1 WholeCopy→+8: 245 null→rác + LOCKED 234/235 β FAILED · case2 Widen tag=MIN: 246/247→-1, 248→999 · case3 Downcast bỏ+8: 246→1, 248→1199 + LOCKED 231 FAILED 7→4 · lowerer ~+ vô hiệu: 247→OutcomeAlloc, 246 vô can. **3 taxonomy poison phá đúng LOCKED 231-237 = chứng minh subsume thật.** **⚔ field-kế-cận 248** `H2{a@0,p:Point?@8(24B),z@32}` byte-exact (poison đổi 1399→999/1199, z không suy chuyển). **Nếp gấp soundness B8** (O tự đòi trong WO): gate body-aware `is_copy` → `H{b:Bad?}` (Bad chứa String) refuse `HeapNullable T=Bad`. B8 NGUYÊN.

**Fixtures 245-250:** 245 Struct? null→99 · 246 present implicit→3 · 247 present explicit ~+→3 · 248 ⚔field-kế-cận→1399 · 249 Enum? present→5 · 250 Enum? null→77.

**Bài học:** O recon-miss lần 2 (verify CƠ CHẾ construction/materialization TRƯỚC viết WO — 4a/4b chỉ top-level). D mẫu "báo đẹp hơn thực" tái diễn (claim compile-sạch, đo "3 bằng may" thiếu bug B) — O bắt bằng dump MIR + RUN giá trị. D tiến bộ: bảng poison khớp đo O, khai lệch-WO minh bạch, KHÔNG chữ ký giả (học cảnh cáo G).

## ✅ §12.8 — `~+` nullable-present UNIFY ĐÓNG + PUSH (origin `badf50d`, 2026-06-21)
5 commit: `98d0a5c` ADR §12.8 · `ab577ed` feat (2 fix lib.rs) · `b6dd822` fixtures 251-255 · `f64789f` TODO · `badf50d` ADR ký O+G. Gate `0·0·250·0`. **Trả nợ defer "`~+` top-level" (campaign line 89).**

**Bug:** `~+ v` (Positive) lower thẳng `OutcomeConstructor` → `outcome_ty = c.sig.return_type` (Integer main, non-Outcome) → `OutcomeAlloc on non-Outcome 'T?'` rác. O probe RAW: chết CẢ scalar/Struct/Enum top-level (`Integer?`/`Point?`/`Color?`) **+ field-scalar** (`Holder{f:~+5}` với `f:Integer?`). Field Struct?/Enum? ĐÃ chạy §12.7 (247/249). Typecheck KHÔNG cản (`exprs.rs:458-460` `~+`+Nullable → `Type::Unknown` matches) → bug thuần LOWERER.

**2 fix LOWERER-ONLY (tái dùng 100% widening Trục A, 0 dòng JIT/typecheck/value-model/borrowck):**
- **Fix 1** (`lib.rs` ~1210 đầu nhánh else Let): redirect — `init==OutcomeConstructor{Positive,Some(inner)}` ∧ annotation lower ra `Nullable(_)` → lower `*inner` plain THAY `*init`. Khối widening sẵn có (Lát 2 Delta 0) gánh: Struct→`is_struct_widening` Assign-fresh→taxonomy Widen / Enum→retype niche disc@0 / scalar→retype PA-3c no-op. KHÔNG nhánh-hóa type. `lower_type_simple(&Ctx)` pure→gọi 2 lần an toàn.
- **Fix 2** (`lib.rs` ~2940 StructLiteral gate): `field_is_nullable_agg`(Struct|Enum) → `field_is_nullable = matches!(_, Some(Nullable(_)))`. Scalar `~+5`→store i64 (value IS repr). **B8 NGUYÊN** — is_copy check (2999) chạy SAU mọi nhánh → `String?` set `~+"hi"` refuse.

**O verify máu (3 răng đỏ ĐỘC LẬP, mỗi ngã rẽ một răng, restore byte-identical md5):** P1 tắt redirect→251+252+253 `OutcomeAlloc 'Integer?'/'Point?'/'Color?'` (254/255 sống) · P2 gate→_agg→254 `OutcomeAlloc 'Integer'` (251-253 sống) · P3 nới is_copy→255 đỏ (message pin "heap types…" biến mất, rơi lớp-2 verifier "heap-nullable T? not yet lowered"). **B8 defense-in-depth 2 LỚP** (is_copy pin message + verifier). Fixtures value-discriminating (252 pt.x=3≠pt.y=4, 253 Green=5≠Red=1).

**Fixtures 251-255:** 251 top-let scalar→5 · 252 top-let Struct→3 · 253 top-let Enum→5 · 254 field-scalar (đọc qua typed-let `let y:Integer?=h.f`)→5 · 255 ⚔B8 field String? refuse.

**⛔ Nợ phái sinh ghim ADR §12.8 (G xác nhận Sổ Tử Thần, CẤM mở WO-2):** direct `match h.f` trên scalar-nullable FIELD chết `unsupported match pattern (expected enum variant)` — gap **READ-side** (field-read temp Unknown-typed `lib.rs:2904-2911`, cố ý giữ scalar-leaf-as-i64 cho số học), KHÁC bug GHI. Fix = nới field-read typing 2904, blast-radius chưa đo → defer. 254 đọc qua typed-let làm cầu nghiệm thu luồng GHI.

**Bài học:** O recon-trước-WO ĐÚNG nhịp lần này (probe phát hiện phạm vi rộng hơn nhãn + gap read-side TRƯỚC khi viết WO — không lặp recon-miss). D code sạch 1 vòng, không nhánh-hóa, KHÔNG giả chữ ký (học cảnh cáo G). Verify-don't-trust: O tự cắm 3 poison độc lập khớp đúng bảng D.

## Nợ defer (ghim minh bạch)
- ⚰️ **SỔ TỬ THẦN — Trục B:** heap-in-aggregate (String/Vector field) + recursive drop-glue = campaign VISION RIÊNG, **ADR trắng chưa viết**, đụng object-model/ownership/lifetime. B8 §4 khóa chặt mọi heap-in-aggregate field-offset (nullable hay không). CA2 chứng minh plain `String`-trong-struct cũng chưa chạy (chưa có recursive struct drop-glue) → Trục B chặn bởi tiền đề SÂU HƠN nullable. Probe O: `struct Person{name:String}` → lowerer refuse "Only bare local variables may hold heap values in Bậc A".
- ~~`~+` top-level~~ ✅ **ĐÓNG §12.8** (`badf50d`, 2026-06-21) — xem mục trên.
- **READ-side: direct `match h.f` trên scalar-nullable FIELD** (mới ghi §12.8) — field-read temp Unknown-typed `lib.rs:2904-2911`, fix=nới field-read typing 2904 (blast-radius chưa đo). G xác nhận Sổ Tử Thần, CẤM mở WO lúc này.
- `?+>` map/flatMap trên aggregate-nullable · `T?~E` (Outcome aggregate) — defer ADR §8.

## Ghi chú heap-allocation / Box-tam-phân (Giang hỏi 2026-06-20, defer)
Giang ghét `Box<>`, hỏi cú pháp tam phân thay thế. O recon: **ADR-0022 §2 đã map `&+ T`≈`Box<T>`** — `&{+,0,-}` (owner/borrow/weak) gom Box/&/Weak vào 1 trục cân bằng, ĐÃ nuốt Box. Nhưng làm rõ: Box giải nhiều việc (ownership + heap-placement + **recursive types** + indirection); `&+` giải ownership. Câu hỏi kiến trúc THẬT = "heap placement + recursive type biểu diễn ra sao" — giao điểm với Trục B sổ tử thần. `&+` mới design-locked, chưa implement backend (phong ấn YAGNI Mũi C/ADR-0059). Khi mở: ADR trắng (recursive type repr + allocator cấp `&+` + drop-glue đệ quy), KHÔNG vẽ cú pháp mới. Giang nói "bàn lại sau".

[[mentor_o_persona]] [[colleague_d_persona]] [[campaign_heap_nullable]]
