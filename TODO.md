# TODO — Triết Backend (Track C)

Backlog sống cho chiến dịch kế. **Chỉ chứa việc CHƯA xong / phong-ấn.**
Ledger các phần ĐÃ đóng (per-step + commit-hash) → [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + `git log` + `docs/decisions/`.

Mốc hiện tại: Gate `0·0·439·0`, origin/main = `5a61e74`, synced sạch. **🏁 WO-1 `is_empty` TEMP LEAK ĐÓNG — O+G ký 2026-07-20, `5a61e74`.** Thành viên CUỐI của họ fast-path-bypass-`emit_shim_call`: `is_empty` owned-String đọc field `len` trực tiếp, KHÔNG qua chokepoint đã vá ở WO-ShimTempOwnership — sót cách `length` đúng 95 dòng. Đo: `is_empty("hello")`/`is_empty(h.name)` **FREE=0 rỉ câm**, control let-bound=1, anh em `length`=1; mọi `value` ĐÚNG ⇒ 439 fixture value-based mù tuyệt đối. Vá 1 dòng `c.push_owned(arg)`. Răng `is_empty_temp_leak_counting.rs` 7 test, 5 shape assert **`free==1` VÀ `dup==0`** (dedup con trỏ — FREE-count đơn thuần mù trước double-free) + 2 test chứng minh BỘ ĐẾM CÒN SỐNG. **🩸 O poison ĐỘC LẬP:** gỡ đúng dòng của `is_empty` → IE-A/IE-B ĐỎ `FREE=0`, LEN-A/LEN-B/IE-A-ctrl XANH (đặc hiệu), md5 khôi phục khớp. Biến thể Vector qua chokepoint = FREE=1 ⇒ KHÔNG cắm test thường trực. **KHÔNG thêm fixture corpus** (giá trị luôn đúng ⇒ fixture là đồ trang trí — G chuẩn thuận). 🦷 **Bài học: "cùng code path" KHÔNG suy ra "cùng trạng thái đã-vá".** **🏁 WO-ShimTempOwnership ĐÓNG — O+G ký 2026-07-19, `04b6174`…`72a0bd6` (8 commit).** Rỉ câm **CẢ MẢNG shim-mượn** (KHÔNG cục bộ ở `length()` như chẩn đoán gốc của O — số đo của D bác): temp vô danh (field-access HOẶC literal) làm argument cho builtin **mượn** không bao giờ được `push_owned` → không ai drop. `concat` 3→1, `contains` 2→0, `eq` 2→0, `length` 1→0 — mỗi ca thất thoát đúng 2 temp; **cả 7 shape in ra GIÁ TRỊ ĐÚNG** ⇒ 439 fixture value-based mù hoàn toàn. Fix tại **chokepoint `emit_shim_call`** tra `arg_consumes` (mượn/thiếu-entry → `push_owned`; tiêu thụ → cấm) + fast-path `length()` vá riêng. **Bán kính rộng hơn phạm vi ký** (quét cả `remove`/`get` key) — G duyệt giữ rộng: *"thu hẹp nghĩa là viết `if name=="remove" { tiếp_tục_rỉ_nhé() }` — đó là NGU XUẨN, cố tình sinh sibling gap"*. **⚠️ Oracle `hashmap_string_key_struct_value_remove_frees_key_and_value` 2→3**: giá trị cũ ghim trên baseline ĐANG RỈ — **O verify pointer-identity `frees=3 distinct=3 dup=0`**, KHÔNG double-free; ai lùi về 2 là tái mở leak. **🩸 O ép poison tới cùng, TIÊU CHÍ CỦA O SAI:** O tuyên "poison ngược không nổ ⇒ reject"; đo hai chiều → M3 bật + bỏ phân biệt = FREE=1 **không nổ** (D đúng), M3 **tắt** + phân biệt đúng = **SIGABRT double-free** ⇒ **M3 (JIT `mir_lower.rs:4717`) mới là lớp chịu lực**, nhánh `!consumed` bị nó che. O rút tiêu chí. **🔴 NỢ SPOF MỚI (phát hiện nhờ mũi poison đặt nhầm chỗ): `builtin_shim_meta().arg_consumes` là ĐIỂM CHẾT** — cả hai tầng (`push_owned` lowerer + M3 zero JIT) đọc CÙNG một bảng ⇒ **KHÔNG phải defense-in-depth**; một entry khai láo thủng cả hai (khai mượn-thực-tiêu-thụ→leak · khai tiêu-thụ-thực-mượn→double-free, **cả hai câm**). Chưa có răng nào canh bảng này. **🏁 WO-StructReturnRefuse ĐÓNG — O+G ký 2026-07-19, `e7aab8c`.** POLICY GATE refuse `Nullable(Struct)` ở RETURN position — anh em thứ hai của `nullable_enum_return_unsupported` (mirror `Ctx::new`), chặn miscompile câm đã đo ở mặt trận trước (rác câm exit 0 / rác địa chỉ / SIGILL 132 / **SIGABRT 134 mới đo cho struct heap-bearing**). 9 fixture 437-445 (4 refuse + 5 control). Nợ mới ghi sổ: `is_lowerable_nullable_payload` (MIR verifier) cho qua `Nullable(Struct)` VÔ ĐIỀU KIỆN ở return-type position kể cả heap-bearing — đo được SIGABRT 134 thật, KHÔNG suy luận; doc-comment tại đó tự khẳng định sai ("heap fields stay refused via the scalar-only field/payload gate" — số đo bác). Chi tiết ở mục riêng ngay dưới header. **🏁 WO-StructParamABI ĐÓNG — O+G ký 2026-07-19, `7d59b7c`+`ec7ecd8`.** Rác câm ở `Struct?` PARAM (khác cơ chế `Enum?`: pass-by-pointer đọc-xuyên-con-trỏ, KHÔNG copy-in) — fix 1 nhánh `load_place` bare-local trong `mir_lower.rs`, 9 fixture 428-436, nợ mới ghi sổ: `Struct?` RETURN position rác câm+SIGILL không hàng rào (pre-existing, ngoài phạm vi) + `key_marshal` chưa đo. Chi tiết ở mục riêng ngay dưới header. **🏁 WO-NullableEnumParamABI ĐÓNG — O+G ký 2026-07-19, `ccb8db3`.** Chi tiết + nợ lỗ-N1 ở mục riêng ngay dưới header. **🏁 WO-NullableEnumAggregate-Refuse (PA-A) ĐÓNG — O+G ký 2026-07-18, `186bd1c`.** Vá silent-miss SỐNG (`struct Mid{e:E?,m:Integer}` → `mid.m` đọc ra 42 thay 5, exit 0) — chi tiết + phán quyết đảo vai N1/N3 ở mục ADR-0083 dưới. **🏁 ADR-0082 §AMEND-3 GET-BY-VALUE COPY-AGGREGATE KHÓA SỔ — O+G ký 2026-07-15, `28f7c6f`(feat)+`12adc0b`(docs).** `get(container, k)` trả **by-value** aggregate **thuần Copy** (Struct/Enum không heap leaf) từ `Vector<Agg>` + `HashMap<scalar-K, Agg>` — non-destructive (element ở lại, KHÔNG tombstone/free, bitwise-copy). **Heap-bearing aggregate → REFUSE E1049** (mã mới, trỏ `get_ref`); aggregate-key×aggregate-value → defer (refuse cả typecheck E1041 lẫn JIT). Scope + E1049-mới + Vector-HashMap-cùng-slice = G rule 2026-07-14. **Predicate `Type::is_copy_aggregate` mirror `MirType::is_copy` (single-source-of-truth) — KHÔNG `!aggregate_needs_drop`** (over-approx enum-field via `collect_heap_leaves`; **D bác giả định WO gốc của O đúng LUẬT 5, D đúng O sai**, O đính chính §AMEND-3.2). Shim mới `__triet_{vector,hashmap}_get_copy` (`returns_borrow_of:None`) tách khỏi get_ref → borrowck KHÔNG synth loan (§AMEND-3.5); reuse thân get_ref dưới symbol thứ 2. JIT copy-out: thin (stride≤8) ghi thẳng deref'd return, fat (>8) load-loop; shared defensive `!is_copy` guard (Rule#7, latent). **🩸 O verify máu (cp-snapshot restore md5 `a753366b`):** poison `is_copy_aggregate` heap→Copy → `Vector<Tagged{String}>` **double-free 134** (E1049 gate load-bearing, non-vacuous) · **8B-heap-struct T9-masking** (`Wrapper{v:Vector<Integer>}` total_size=8 — leak-câm Slice A) D KHÔNG test → **O probe sống bắt → E1049**, fixture 367 canh vĩnh viễn · positive 361/362/363/366 + counting FREE-count route-lower xanh · E1049 harness assert code-string. **⚠️ D tự bắt+vá SIGSEGV thin-return-deref (363) khai thật; F3 gom guard đối xứng 1 chỗ + bỏ guard hashmap-value trùng. D claim "fmt ran" nhưng edits cuối chưa fmt (LUẬT 2 slip — pre-commit hook bắt, O fmt lại).** Model D=Sonnet 5 (đủ lõi; vết: thiếu fixture masking + fmt-slip, O bù). **Nợ đóng-gói-campaign-riêng:** 🚩 **Deep-Clone heap-bearing aggregate** (campaign LỚN riêng — ADR `.clone()` tường minh + carve-out ADR-0042 move-only + codegen clone đệ quy) · 🚩 **get_ref value-aggregate** (Cụm D/ADR-0081 FROZEN) · 🚩 **drain** (ADR Iteration). ⚰️ ADR-0068 Box/recursive VẪN CẤM CỬA (G nhắc lại). **🏁 ADR-0083 §AMEND-1 SLICE 2 ENUM KEYS KHÓA SỔ — O+G ký 2026-07-13, `91c273a`.** `HashMap<Enum,V>` enum-key (payload unit/scalar/String + enum-as-struct-leaf) sound: insert/get/get_ref/contains/remove/drop. Walker model flat `KeyLeaf` → **recursive emission** `emit_key_hash_value`/`emit_key_eq_value` (disc-mix + `brif`-chain **ACTIVE variant only**, mirror `emit_enum_drop_glue_at` — KHÔNG đọc inactive/padding garbage); key free-loop REUSE `emit_enum_drop_glue_at`; §6 fnptr-first shield unchanged. **🩸 O verify máu (cp-snapshot restore md5 `80fd7ce7`):** DP-E2 reassign-garbage tooth **NON-VACUOUS** (poison tail@+16 → 358 MISS -1; tail rác sau reassign THẬT) · §6-reverse → 354 crash 134 (enum key-class mới rides shield) · struct Slice-1 KHÔNG hồi quy (352/353 green sau walker viết-lại). **⚖ DESCOPE (G rút lệnh "nested-enum MỞ"):** enum variant ôm **aggregate payload (Struct/Enum)** → **REFUSE E1048** — O probe lift-refuse → `HashMap<Shape,_>` roundtrip **MISS -1** chứng minh lowerer fix-8B enum-payload → marshal truncate → silent-MISS. Refuse-over-guess G khen "cứu rỗi bộ nhớ". **⚠️ D bịa 1 chứng phụ** ("enum-in-enum fails MIR verifier in plain match" — O P1 probe bác, plain match chạy=7) → O bắt, D/O sửa doc-comment types.rs (giữ sự thật truncate→MISS, xóa claim MIR-verifier). ~~**🔴 NỢ MỚI G MỞ: "Enum-Payload-Aggregate Sizing Fix"** — lowerer `triet-lower/src/lib.rs` fix-8B enum payload size (fixup pass chỉ struct field, bỏ enum payload) → aggregate payload >8B truncate. Mở khóa nested-enum/enum-struct-payload key khi đóng.~~ **ĐÓNG 2026-07-18 (WO-NullableEnumAggregate-Refuse PA-A).** Sizing đã đúng từ `9a1799c` (ADR-0067 §AMEND co-fixpoint); lỗ CÒN LẠI vừa vá là một bug SIBLING riêng trong `resolve_aggregate_size`'s `Nullable(Enum)` arm (tra nhầm `struct_map` thay vì `enum_map` → luôn MISS → field `E?` payload-bearing giữ seed 8B, tràn sang field kế — silent-miss, không phải sizing-fixpoint-bỏ-sót như chẩn đoán gốc). Refuse tại tầng khai báo (N1, `lower_program`) + fix 1 dòng (N3, `struct_map`→`enum_map`) đã đóng. **⚠️ D probe hậu-vá: N3 alone (KHÔNG N1) đã đủ tự sửa hoàn toàn 3 kịch bản đo được** (struct-field sibling-corruption, nested-field, enum-variant-payload construct+read, cả present lẫn `~0` null arm) — **⚖ G PHÁN QUYẾT 2026-07-18 — ĐẢO VAI N1/N3:** **N3 = SOUNDNESS FIX** (chống tràn size gây corruption — fix THẬT); **N1 = POLICY GATE** (chặn chờ ADR-0065 cấp phép thiết kế repr — **KHÔNG** phải soundness-gate quan sát được qua poison). **O verify ĐỘC LẬP 6 shape** (aggregate payload 24B · đọc ngược chính field nullable · enum-payload-nullable lồng · 3 shape heap — heap bị `heap_type_not_supported` chặn, KHÔNG phải N1): gỡ N1 giữ N3 → **không shape nào corrupt**. G GIỮ N1 tuyệt đối vì bề mặt `Enum?` còn vỡ chỗ khác (nợ SIGILL 132 dưới). **CẤM về sau viện N1 như bằng chứng đã đóng một đường UB.** **🦷 N3 CÓ RĂNG DUY NHẤT** = unit test `resolve_aggregate_size_nullable_enum_reads_enum_map_not_struct_map` (`triet-lower/src/lib.rs` mod tests) — vì N1 chặn MỌI đường fixture-level chạm N3; O verify đỏ độc lập (đảo token → `left: 8 / right: 16`) **+ đặc hiệu** (poison site sinh đôi bare-`Enum` `:570` → test VẪN XANH; site đó do `enum_field_moveout_frees_once_with_cap` canh — cả hai site đều có răng). **⚖ D BÁC WO CỦA O — ĐÚNG (lần 5/5):** O viết teeth protocol "gỡ N1 → thấy 42" **mâu thuẫn với chính số control-biến O đã đo đầu phiên** (`:567` đổi một token → ra 7). **Bài học O khắc: giao thức poison phải đối chiếu ngược với số đo đã có của chính mình — dữ liệu bác WO nằm sẵn trong tay mà không nhảy số.** ~~**🔴 NỢ MỚI (D khui, O verify): `Enum?` PARAMETER ABI VỠ — SIGILL 132.**~~ **ĐÓNG 2026-07-19 (WO-NullableEnumParamABI, `ccb8db3`).** ⚠️ **Chẩn đoán gốc "SIGILL 132" MÔ TẢ THIẾU — failure-mode thật là RÁC CÂM:** `Nullable(Enum)` param không bao giờ được cấp `enum_slots` entry (copy-in `mir_lower.rs` match `MirType::Enum` **exact**, trong khi vòng derived-locals **có** unwrap `Nullable`) → `load_place` bare-local + `GetDiscriminant` rơi fallback `use_var(param_val)` = đọc **bit-pattern của CON TRỎ caller** làm giá trị. Địa chỉ stack không bao giờ bằng `i64::MIN` ⇒ sentinel-compare **luôn phán "present"** ⇒ **nhánh `~0` CHẾT trên mọi biên gọi hàm, trả sai câm, exit 0**. SIGILL 132 chỉ là **ca con**: khi nhánh present bị nhận nhầm đi đọc discriminant thì `SwitchInt` mới rơi default-arm `Trap`. Fix = unwrap `Nullable` tại copy-in (mirror idiom `nullable_payload().unwrap_or`), **chỉ vị trí parameter**; guard `~+` (ADR-0065) + guard return-position không đụng. **9 fixture 419-427 chấm bằng GIÁ TRỊ, không bằng exit code** — oracle-exit-code XANH trên cây hỏng ở ca câm (422). **🩸 O verify máu độc lập (cp-snapshot restore md5 `8dde9fc9`, KHÔNG `git checkout`):** poison-1 gỡ fix → 419/420/421/424/425 SIGILL 132 + **422 ĐỎ bằng giá trị (1≠0)**, 423/426 control xanh · poison-2 chặn nhánh `Enum` **trần** giữ `Nullable` → **426 ĐỎ, 419-425 XANH tuyệt đối** (đặc hiệu) · **O tự đào lỗ verify D bỏ sót: harness là MỘT test đơn `integration_test_corpus()` chạy vòng lặp ⇒ SIGILL ở 419 giết cả tiến trình, 422 KHÔNG BAO GIỜ CHẠY dưới poison-1** → O dựng phép thử riêng đổi `EXPECT: 0`→`777` → `FAIL 422…: expected 777, got 0` chứng minh **T3 có răng ở tầng harness**, không phải file trang trí. **⚖ D BÁC O 2/2 lần, đúng cả hai** (xem mặt trận `Struct?` + nợ lỗ-N1 dưới). **🩸 Bài học O khắc (lần 7): dán nhãn failure-mode SAI** — O ghi "SIGILL 132" cho `Struct?` param, D đo ra rác câm; đo lại 5/5 mỗi bên: đọc **HAI** field (`v.x+v.y`) → SIGILL 132 xác định (rác cộng rác vượt ngưỡng ±(3²⁷−1)/2 → trap range-check **ADR-0044**, hiệu ứng **THỨ CẤP**), đọc **MỘT** field → rác câm exit 0 đổi theo ASLR. **Rác câm là failure-mode GỐC; SIGILL là tiếng sấm đi kèm.** [[feedback_failure_mode_precision]] **🏁 ADR-0083 KEY-AGGREGATE HashMap SLICE 1 (Struct keys) KHÓA SỔ — O+G ký 2026-07-13, `0ebd763`+`1c08a67`.** `HashMap<Struct,V>` struct-key (leaves scalar/String/nested-struct) sound end-to-end: insert/get/get_ref/contains/remove/drop. **ABI G-mandate:** header 24B fixed `[refcount@0][packed@4][hash_fn@8][eq_fn@16]` + fnptr-in-header null-sentinel (Integer/String→NULL, Struct→walker addr) — **§6 dispatch fnptr-TRƯỚC-stride = lá chắn Size-Collision-Trap** (struct 24B trùng String `key_stride` 24; đảo thứ tự → O verify 352 **SIGSEGV 139**, máu G đòi). JIT walker `build_key_hash_walker`/`build_key_eq_walker` đệ quy `collect_key_leaves` (scalar→FNV-mix · String→`__triet_string_hash` · nested→đệ quy; eq short-circuit); key free-loop đệ quy §4; typecheck `is_hashable_leaf` E1048 (Enum-key/collection-leaf/Nullable-leaf REFUSE). Spike `func_addr` fail-fast TRƯỚC walker (G mandate). **🩸 O verify máu độc lập (cp-snapshot restore md5 `0fd4b450`):** §6-reverse→352 SIGSEGV 139 · eq-content-String→ptr-identity→353=-1 (răng thật) · baseline walker JIT THẬT 352=42007/353=42 (MIR dump = `get(struct_key)`, KHÔNG stand-in) · diff BASE-vs-committed = **chỉ doc-comment** (logic byte-identical). **🩸 O bắt 2 Blocker (D vá):** (A) get/get_ref/contains CHƯA wire ở source (E1041, chỉ insert/remove reachable) → D wire overload `exprs.rs:1190` + 2 fixture roundtrip THẬT 352/353; (B) walker correctness chỉ stand-in/compile-only → thêm test runtime; **comment fixture 353 claim hash-poison→RED GIẢ (vacuous do allocator 16-align mask bucket `cap≤16`)** → D sửa thành eq-poison + note trung thực. **Bài học: hash-content KHÔNG tooth được bằng functional-roundtrip cap-nhỏ (alignment mask); cap-LỚN + assert-hash-trực-tiếp mới bắt (ADR-0080 tooth #5 `cap=1_000_003`) — correctness rides on EQ.** **Nợ Slice 2 defer (🚩 cắm cờ):** Enum-key (discriminant + padding-bits + variant-size) · Nullable-leaf key · hash-caching. **⚠️ BOM FIX-2 zero-@8 (Slice B) GIỮ NGUYÊN, chưa đụng.** **🏁 `Vector::pop_front` (O(n) shift, tombstone `len--`) ĐÓNG — ADR-0082 B-α continuation, O+G ký 2026-07-12, `5462c5b`.** Move-out phần tử ĐẦU by-value (`T?`) tái dùng ABI D-1 (`tag@0/fields@+8` marshal + `len--` tombstone); shift xuống O(n) giữ INV-B-α (KHÔNG ring-buffer, KHÔNG hứa O(1)). get-by-value/drain/HashMap-pop_front trục xuất (campaign-nền riêng). 1 commit gộp code+counting-tooth. 7 site (grep-verified 6 ABI-site `__triet_vector_pop` mirror đủ 0 sót); shim B1(rút[0] trước)/B2(`ptr::copy` memmove — overlap len≥3)/B3(`len--` no-zero). **O poison máu độc lập (restore md5 `d90caa4f` mọi vòng):** T-G1 order=132 · T-G2a bỏ`len--`→SIGABRT 134 · T-O1 bỏ pop_front khỏi fat-gate→JIT compile-refuse · **leak counting-tooth thường trực** (`vector_string_pop_front_then_drop_no_double_free` FREE==3; poison→4 RED, pop-back tooth xanh=cô lập) · **T-G2b báo trung thực NON-manifest** (front-pop shift xuống dst<src=memcpy-safe; `ptr::copy` GIỮ vì UB-hygiene, KHÔNG giả đỏ). **Nợ campaign-nền đóng băng (G chốt lôi ra Phase sau):** 🚩 get-by-value (ADR Copy/Clone) · 🚩 drain (ADR Iteration) · 🚩 BOM FIX-2 zero-@8 Slice B. **🏁 CỤM B SLICE C (`HashMap<K,aggregate>` VALUE) KHÓA SỔ — G ký 2026-07-10, `36ba45f`.** insert+drop+alloc value-aggregate (Struct/Enum) SOUND — mirror Slice A/B element push+drop; get/get_ref/contains/remove + key-aggregate REFUSE (get-family chết ở **typecheck** E1041/E1002/E1048, remove refuse ở **JIT** — O probe `.tri` source verify = defense-in-depth có tri thức). 2 commit code: `6ec2630`(F1–F4 fix + T4 unit) · `36ba45f`(teeth). **F1** value-free-loop guard `is_any_heap()`→`aggregate_needs_drop` (mirror Vector 1186); **F2** Enum-arm đệ quy (defense-in-depth latent — frontend chặn enum-payload-aggregate; T4 unit pin); **F3** marshal HAI ĐẦU S3-gap (ĐẦU-A `enum_slots` fat >8 · ĐẦU-B 8B `stack_load` KHÔNG `use_var` — C5/T9 tái sinh); **F4** refuse tách `_key`(key-only)@alloc+insert / giữ `_kv`(K+V)@remove×2+get-family. **🩸 O tự bắt lỗ G bỏ sót ở WO: MÌN-3 ĐẦU-B** (8B-aggregate value ôm 1 handle → `use_var` đọc garbage → LEAK CÂM, 331 fixture không thấy) — tooth **T3** cắm riêng, poison→FREE 0. O verify 4+1 poison→RED độc lập (F1→T1/T2/T3 FREE 0 · F2→T4 false · F3-A→T2 compile-fail · F3-B→T3 FREE 0 · neuter 2 helper→6 refuse "SUCCEEDED"), cp-snapshot restore md5 `62ab04…` khớp, failure-mode = FREE-count-wrong (leak, KHÔNG SIGSEGV). Repurpose `hashmap_struct_value_refused_at_jit`→`..._remove_refused_at_jit` (Luật 3; coverage dương insert-Struct-value chuyển sang T1). **⚠️ Bom hẹn giờ cũ (FIX-2 zero-@8) giữ nguyên, không đụng slice này.** **Nợ mở Slice C (defer):** value move-out (get/remove by-value — nấm mồ chung Vector pop, recursive tombstone) · get_ref borrow value-aggregate (Cụm D/ADR-0081) · contains-allow value-aggregate (follow-up nhỏ) · key-aggregate hash+eq đệ quy. **🏁 CỤM B SLICE B (`Vector<Enum>`) KHÓA SỔ — G ký 2026-07-09, `638b455`.** push+drop sound (heap-payload variants), **pop/by-value move-out REFUSE** (deferred, đòi recursive move-out-tombstone). 8 commit: `c8b8aa6`(S1+S2) · `3bede0c`(S3) · `98a3be2`(AM1 refuse pop) · `a665e96`(AM2) · `a6a41c2`(FIX-1+FIX-2) · `638b455`(11 teeth) + 2 docs. **🩸 O tự bắt: (1) pop fat-aggregate UB pre-existing SLICE A** (verify binary `1e49058`; Slice A teeth chưa từng test pop; deferred-KHÔNG-refuse = UB câm shape P0) → AM1 REFUSE bịt cả A lẫn B. **(2) push+drop UNSOUND — HAI bug che nhau** (BUG-1 `aggregate_needs_drop:1663` thiếu Enum→leak · BUG-2 enum-local không tombstone→double-free; named-case triệt tiêu thành "2 giả sound"). **poison-must-be-red cứu mạng:** first-draft named-tooth đếm nhầm→10/10 xanh giả, poison S2 KHÔNG đỏ mới đào ra. FIX-1(Enum arm đối xứng Struct)+FIX-2(zero payload ptr @8). Teeth làm lại INLINE-anchor non-masking: FIX-1→inline 0 · FIX-2→named 4 · AM1→struct-pop compiles(lỗ A phơi). **⚠️ Bom hẹn giờ ghi sổ:** FIX-2 zero-@8 đúng CHỈ VÌ frontend refuse enum-payload multi-heap-leaf (struct/tuple); nếu gỡ → phải walk mọi leaf. **Nợ mở:** `Vector<aggregate>` pop/get-by-value move-out (recursive tombstone) · scalar-enum disc chưa observe source. **Mặt trận kế = Slice C ĐÃ ĐÓNG 2026-07-10 (xem đầu Mốc).** **🏁 CHIẾN DỊCH READ-SIDE (CỤM A) KHÓA SỔ — G ký 2026-07-04, `37a0723`.** ✅ **A1 get-borrow generic-V**: 6 overload `get` V=Vector/HashMap (Int-key + String-key) → `(&0 V)?`; §AMEND-1 stride-conditional deref giữ invariant `&0 V` bit-for-bit dù local hay get_ref (thin→body_ptr, fat String→cell). ✅ **P0 BÁO ĐỘNG ĐỎ**: pre-existing String-key read SIGSEGV (latent từ ADR-0080 — get/get_ref/contains nhận `&0 HashMap` Reference-wrapped, key_stride chỉ bóc Nullable → default 8 → String-key 24B marshal by-value → hash rác → SIGSEGV) VÁ = unwrap `MirType::Reference` trước match HashMap; cắm cờ 335. ❄️ **A2 get-borrow-mutable** (ADR-0081): FROZEN → Cụm D (functional push/insert ⇒ `&0 mutable V` vacuous khi chưa có deref-assign). 🚫 **V=Nullable**: REFUSE/defer (lowerer chưa match `&0 Nullable`). O verify máu poison→RED độc lập (POISON-1 stride-deref→garbage · POISON-P0 Reference-unwrap→SIGSEGV 139 · overload-break 336/337→E1041); 5 fixture 333-337; restore md5 khớp. **⚠️ Kỷ luật D**: cảnh cáo thép "lần cuối dung túng ném-API-không-test" (D bẻ lệnh giữ String-key overload + thiếu fixture heap-value → G ép bổ sung 336/337). **🔑 ADR-0080 KM-P1a (key-typed `HashMap<String,V>` BACKEND) LANDED — Author+O+G ký 2026-07-03, `c003a5f`. Mechanism sound & sleeping (hand-built MIR + counting): slot `key_stride` 24B fat · `__triet_string_hash` FNV-1a + `hashmap_key_hash/eq` dispatch theo key_stride · key drop-obligation §AMEND-1 out-param ABI (D.1 map-drop-loop / D.2 insert-dup `is_update_out` / D.5 remove-resident `key_out_ptr` — free ở JIT call-site registry-routed → counting-testable, KHÔNG free trong thân shim). O verify 5 teeth poison→RED độc lập (map-drop-leak 1→0 · update-leak 2→1 · remove-leak 1→0 · content-hash alloc-indep cap=1_000_003 · rehash key-stride→NULL_SENTINEL). Chi tiết mục "🔨 ĐANG MỞ" dưới.** **🩹 BUG-E (Outcome-param ABI mis-tag + `~->` early-return heap double-free) ĐÓNG — O+G ký 2026-07-03, 2 WO liên tiếp (`ddb7841` param-ABI copy-in gap + `818602c` early-return heap-payload double-free, 3 site). Chi tiết đầy đủ ở mục "✅ ĐÓNG — Bug-E" bên dưới.** **🩸 GET-BORROW HEAP VALUE (ADR-0079) IMPLEMENTED/CLOSED — G ký 2026-07-01.** Read-side container khép: `get(&0 container,k)→(&0 V)?` zero-copy borrow (P1 V=String). Borrowck whole-container loan (U2 PropagatedLoan builtin + U3 mutate-while-borrowed E2440 cho consume insert/push + in-place remove/pop); JIT shim trả con-trỏ-slot (0 alloc), not-found→NULL_SENTINEL. O verify máu: content-read `length`→2/5 · source-level E2440 · 5 borrowck teeth poison. Slice A `a970540`. **🏁 TYPED HEAP-CONTAINER P1 ĐÓNG TRỌN — ADR-0077 (Vector) + ADR-0078 (HashMap) SEALED, G ký 2026-07-01.** `Vector<T>` + `HashMap<Integer,V>` (T/V = built-in heap: String/Vector/HashMap/Nullable) chạy sound end-to-end source qua JIT real-allocator: construct + push/insert(Move) + pop/remove(move-out `T?`/`V?`) + drop — không rỉ một byte. Element/value-SIZE = hằng compile-time (tách-tầng khỏi native-layout). **KEY-typed (`HashMap<String,V>`) + UserStruct value + get-clone/borrow heap value = defer Tầng 2/P2.** HM-P1b 3 vòng O-reject ép chân lý: garbage `lower_type` bỏ value-arg → vacuous-tooth (literal-temp no-drop-obligation) → named-local poison `arg_consumes[2]=false`→SIGABRT 134 ĐỎ. **🏁 KỶ NGUYÊN NULLABLE KHÉP HOÀN TOÀN — ADR-0076 SEALED (heap-`T?` trong aggregate field/payload, giao điểm B8 cuối).** Lát đơn atomic `6327890`: 5 mũi (gate-lift + field-layout sentinel + drop-arm `collect_heap_leaves` + construct/widen + borrowck). Cổ tức PA-3c: conditional-drop = sentinel-no-op, KHÔNG `brif`. O vồ double-free CASE B (match-present-bind-move → SIGABRT 134, borrowck im) → D đóng STATIC tag-niche-tombstone (KHÔNG dynamic-flag). O verify máu 3 tooth (Deinit-after-bind 134 · sinh-tử `is_copy(Nullable(heap))==false` 7-leak · drop-arm). `let s=b.s`→E2423 (Nợ defer giữ). **🔒🏁 CAPABILITY Ł3 (ADR-0069) NIÊM PHONG — COHERENCE VISION §8 HOÀN TẤT.** Đại số Ł3 khép kín ba chân: null(PA-3c) / logic(Trilean) / **capability**. ZST-token ngậm Ł3-Trit: Grant(+)/Ambient(0)/Deny(−) tĩnh zero-cost + Defer(Unknown) runtime trap `user(2)` fail-closed. Lát 0 `8b06a28` (ZST & cấm copy, 2-classifier defense-in-depth) · §amend-A `47eb283` (M1 receive-only) · Lát 2 `ca8272e` (possession E2212) · §5 `d84cd24` (mint-site lock) · Lát 3 `2dd4d5f` (Defer hook — O verify 4 răng, R-fail-closed boundary `≤` là tử huyệt) · Lát 4 demo fixture `278` (end-to-end →30). Mã mới: E2211 (mint non-grant) · E2212 (deny possession). **Mặt trận mandate ternary-first (G+Giang 2026-06-22) ĐÓNG.** **🏁 Heap-aggregate cluster ĐÓNG TRỌN** — ADR-0070 partial-move + ADR-0071 import `::` + WO-0073/74/75 (heap-nullable-return drop-glue · enum-field move-out · multi-level `h.inner.x` projection-path), origin/main = `0947482`.

### ✅ ĐÓNG — **`WO-INV-HeapNullable-Probe`** — LOCAL SOUND, doc comment sửa (O duyệt + G ký nhánh B, 2026-07-19, `5f65dee`+giai đoạn (a))

**Câu hỏi:** `Nullable(Struct)` ở LOCAL-binding position với struct mang PLAIN heap field
(`struct H { name: String }`, khác `String?` field — đã có ADR-0076 riêng) không bị
refuse ở tầng nào (predicate `is_lowerable_nullable_payload`,
`crates/triet-mir/src/lib.rs:1637`, cho qua `MirType::Struct(_)` vô điều kiện) và chạy
ra GIÁ TRỊ đúng trên 4 shape lịch sử rủi ro nhất (O đo trước WO). Giá trị đúng KHÔNG
chứng minh sound — WO này đo FREE-COUNT, không đo giá trị.

**Tooth:** `crates/triet-driver/tests/heap_nullable_struct_local_counting.rs` (7 test,
`AtomicUsize` process-global qua `__nls_str_free` shim, `TEST_LOCK` Mutex serialize
trong binary).

**4 con số đo được (khớp bảng kỳ vọng 4/4):**
| Shape | FREE-count kỳ vọng | Đo được | |
|---|---|---|---|
| S1 `let a: H? = ~0;` (null) | 0 | **0** | ✓ |
| S2 present, drop tự nhiên cuối scope | 1 | **1** | ✓ |
| S3 present + `match a { ~+ v => …, ~0 => 0 }` bind-move | 1 (không phải 2) | **1** | ✓ |
| S4 `while` 3 vòng, alloc mới mỗi vòng (ép tái dùng StackSlot qua back-edge) | 3 | **3** | ✓ |

**Bằng chứng non-vacuous (poison → số đổi, dán raw):**
- S2 poison-leak (`__nls_str_free_poison_leak`, no-op không đếm) → 0 (kỳ vọng 1 nếu healthy) — `s2_poison_leak_proves_tooth_is_live` xanh vì assert đúng 0.
- S3 poison-double (`__nls_str_free_poison_double`, đếm gấp đôi mỗi free thật) → 2 (không phải 1) — `s3_poison_double_proves_tooth_is_live` xanh vì assert đúng 2.
- S4 poison-leak → 0 (không phải 3) — `s4_poison_leak_proves_tooth_is_live` xanh vì assert đúng 0.
- Cả 3 poison đều làm số ĐỔI đúng theo cơ chế poison — bộ đếm quan sát được free thật, không phải hằng số cứng.

**Kết luận T0 (thuần dựa trên số):** **SOUND** trên 4 shape LOCAL đã đo. O verify độc
lập (tự chạy 7 tooth 7/7 xanh + tự đọc S3 xác nhận D không né góc chết + tự cắm probe
riêng cô lập bug orthogonal, restore md5 khớp) → duyệt. G ký nhánh B.

**Giai đoạn (a) đã thi hành:**
1. **Đập doc comment lừa đảo** tại `crates/triet-mir/src/lib.rs:1627` (predicate
   `is_lowerable_nullable_payload`) — xóa câu sai *"Both are Copy-only (rào B8): heap
   fields/payloads inside the aggregate stay refused via the scalar-only field/payload
   gate below."* Viết lại đúng sự thật đo được: predicate này CHỈ gác return-type +
   local (không gác field/payload — đó là `is_field_payload_lowerable` riêng), **cố ý
   permissive** ở vị trí local — cổng heap thật nằm ở BA nơi khác, dẫn tọa độ chính xác
   tự verify (không chép của O):
   - **field** → `crates/triet-lower/src/lib.rs:3762-3772` (guard `is_heap_nullable_leaf`/
     `ctx_is_copy` trong struct-literal field lowering) → `LowerError::heap_type_not_supported`,
     driver **exit 3**.
   - **param** → `crates/triet-jit/src/mir_lower.rs:3329-3334` (`struct_slots.get(local)`
     thất bại trong tag-guarded Drop-glue, param không có slot vì ABI pass-by-pointer) →
     `JitError::Unsupported("Struct? Drop without slot")`, driver **exit 4**.
   - **return** → `crates/triet-lower/src/lib.rs:158` (`nullable_struct_return_unsupported`,
     `WO-StructReturnRefuse`, fixture 440), driver **exit 3** — guard này chạy TRƯỚC khi
     predicate kịp thấy heap-bearing `Nullable(Struct)` ở return.
2. **Lưới kiểm soát 5 mục — D tự đo lại trên binary rebuild, raw dán trong báo cáo kèm:**
   `P?` local → exit 0 giá trị 0 ✓ · `H?` param → exit 4 ✓ · `H?` field → exit 3 ✓ ·
   `H?` return → exit 3 ✓ · `Enum?` payload-bearing N1 → exit 3 ✓. Không over-refuse.
3. **7 tooth `5f65dee` re-run trên cây cuối (rebuild `cargo build --release` +
   `cargo test`)** — 7/7 xanh, 3 poison (`s2_poison_leak`→0, `s3_poison_double`→2,
   `s4_poison_leak`→0) vẫn đỏ-đúng-hướng (assert pin giá trị chính xác, không phải
   "test chạy được"). Cọc không mục.

**KHÔNG đụng bug leak orthogonal** (xem nợ `WO-InlineFieldTempLeak` ngay dưới) — G rào
mặt trận riêng, chạm vào ở WO này = REJECT.

**Gate cuối WO này:** build 0 · test-fail 0 · fixtures 439 · clippy 0 · CLEAN.

### ✅ ĐÓNG — `WO-ShimTempOwnership` (O+G ký 2026-07-19; lịch sử tên: `WO-InlineFieldTempLeak` → `WO-LengthFastPathTempLeak` → **`WO-ShimTempOwnership`**)

**Câu hỏi quyết định:** RỈ CÂM đo được ở `length(h.name)` (FREE=0, giá trị đúng, exit 0)
có **cục bộ tại fast-path `length()`** (`crates/triet-lower/src/lib.rs:2472-2479`), hay
**rỉ cả mảng shim-mượn** (mọi builtin nhận `String` sở hữu qua `emit_shim_call`)?

**Tooth:** `crates/triet-driver/tests/heap_shim_temp_leak_counting.rs` (10 test — đăng
ký thêm shim `__triet_string_concat`/`__triet_string_contains`/`__triet_string_eq` vào
harness counting, thứ mà tooth trước KHÔNG có → trước đây đo `concat` báo lỗi
`Unsupported("shim '__triet_string_concat' not registered")`).

**Ma trận đo được (raw, `cargo test -p triet-driver --test heap_shim_temp_leak_counting -- --test-threads=1 --nocapture`):**
```
running 10 tests
test poison_double_on_shb_control_proves_tooth_is_live ... SH-B-ctrl POISON(double): FREE=6 ok
test poison_leak_on_shb_control_proves_tooth_is_live ... SH-B-ctrl POISON(leak): FREE=0 ok
test sha_concat_two_inline_literals ... SH-A (concat inline-literal x2): FREE=1 ok
test sha_control_concat_two_let_bound ... SH-A-ctrl (concat let-bound x2): FREE=3 ok
test shb_concat_field_plus_inline_literal ... SH-B (concat field+inline-literal): FREE=1 ok
test shb_control_concat_both_let_bound ... SH-B-ctrl (concat let-bound both): FREE=3 ok
test shc_contains_field_plus_inline_literal ... SH-C (contains field+inline-literal): FREE=0 ok
test shc_control_contains_both_let_bound ... SH-C-ctrl (contains let-bound both): FREE=2 ok
test shd_control_eq_both_let_bound ... SH-D-ctrl (eq let-bound both): FREE=2 ok
test shd_eq_field_plus_inline_literal ... SH-D (eq field+inline-literal): FREE=0 ok
test result: ok. 10 passed; 0 failed
```

| Shape | Inline (rvalue temp) | Let-bound control | Sound-baseline |
|---|---|---|---|
| `concat("ab","cd")` — 2 LITERAL | FREE=1 | FREE=3 (`sha_control`) | 3 |
| `concat(h.name,"cd")` — FIELD+LITERAL | FREE=1 | FREE=3 (`shb_control`) | 3 |
| `contains(h.name,"ell")` — FIELD+LITERAL, shim KHÔNG có `builtin_shim_meta` entry | FREE=0 | FREE=2 (`shc_control`) | 2 |
| `eq(h.name,"world")` — FIELD+LITERAL | FREE=0 | FREE=2 (`shd_control`) | 2 |

**Bằng chứng non-vacuous:** poison trên control `SH-B-ctrl` (giá trị baseline sound = 3
free thật) — poison-leak (`__hstl_str_free_poison_leak`, no-op không đếm) → **0**;
poison-double (`__hstl_str_free_poison_double`, đếm gấp đôi mỗi free thật) → **6**
(= 2×3). Cả hai đổi đúng hướng theo cơ chế poison → bộ đếm quan sát free thật.

**⚖ KẾT LUẬN DỨT KHOÁT, THUẦN DỰA TRÊN SỐ: RỈ CẢ MẢNG SHIM-MƯỢN, KHÔNG CỤC BỘ TẠI `length()`.**
3 shim độc lập (`concat`, `contains`, `eq`) — 2 shim CÓ `builtin_shim_meta` entry
(`arg_consumes: [false,false,false,false]`), 1 shim (`contains`) HOÀN TOÀN KHÔNG có
entry — đều rỉ Y HỆT khi arg là temp vô danh (field-access HOẶC literal, không phân
biệt), và đều lành khi arg qua `let`. Sự có/không của `builtin_shim_meta` không ảnh
hưởng gì tới leak này — cơ chế thật nằm ở TẦNG TRƯỚC shim: `Ctx::push_owned` không bao
giờ được gọi cho một temp sinh ra bởi `lower_expr` trên một biểu thức field-access
hoặc literal khi biểu thức đó được dùng TRỰC TIẾP làm argument (không qua `Stmt::Let`).
`length()`'s fast-path (`:2472-2479`) là MỘT ca của lỗ này, không phải NGUYÊN NHÂN của
nó — vá riêng `:2472-2479` sẽ KHÔNG đóng `concat`/`contains`/`eq`/bất kỳ shim-mượn nào
khác.

**Vì sao user-function call KHÔNG rỉ (đối chứng P3/P4 của O):** `Expr::Call` cho hàm
người dùng (CẢ 3 nhánh return: fat-sret/outcome/scalar,
`crates/triet-lower/src/lib.rs:3016-3182`) đều tự Deinit-tombstone mọi arg Move-type
SAU lời gọi (`to_zero` + `Statement::Deinit`, ADR-0042 Q1) — đây là CHUYỂN QUYỀN SỞ HỮU
thật sự sang callee (tham số của callee ĐƯỢC `push_owned` ở function-entry, dòng
~1101-1105), nên callee tự free nó ở scope-end của chính nó, bất kể caller có
`push_owned` temp đó hay không. `emit_shim_call` (dòng 1477-1503, dùng cho MỌI builtin
shim-dispatch: concat/eq/contains/is_empty/…) **KHÔNG có bước tương đương** — nó chỉ
emit `CallDispatch` rồi trả `dest`, không đụng gì tới ownership của `args`. Với shim
mà `arg_consumes[i]=false` (borrow — hoặc hoàn toàn không có `meta` entry, tương
đương), KHÔNG ai từng free/transfer temp đó cả.

**Bảng builtin nhận `String` sở hữu — tiêu thụ (consume) vs mượn (borrow):**
| Builtin | Owned `String` arg? | `arg_consumes` | Phân loại | Đo trong T0 này? |
|---|---|---|---|---|
| `length`/`len(s)` | arg0 | N/A — bỏ qua shim hoàn toàn (fast-path đọc field `len` trực tiếp) | **MƯỢN** | Đo (WO-INV-HeapNullable-Probe, FREE=0) |
| `is_empty(s)` | arg0 | **RỈ FREE=0 → vá FREE=1** (WO-1, `5a61e74`) | **MƯỢN** (ĐÃ ĐO) | ✅ ĐÓNG — `is_empty_temp_leak_counting.rs`, 5 shape assert `free==1 && dup==0` |
| `concat(a,b)` | arg0,arg1 | `[false,false,false,false]` | **MƯỢN** | Đo (SH-A/B, FREE=1 thay vì 3) |
| `eq(a,b)` | arg0,arg1 | `[false,false,false,false]` | **MƯỢN** | Đo (SH-D, FREE=0 thay vì 2) |
| `contains(h,n)` | arg0,arg1 | KHÔNG CÓ entry trong `builtin_shim_meta` | **MƯỢN** (theo hành vi Rust — chỉ đọc ptr+len) | Đo (SH-C, FREE=0 thay vì 2) |
| `clear(&0 mutable s)` | KHÔNG — nhận Reference, không phải `String` sở hữu | N/A | N/A (khác cơ chế — mutate qua pointer) | Ngoài phạm vi |
| `append(&0 mutable s, byte)` | KHÔNG — nhận Reference | N/A | N/A | Ngoài phạm vi |
| `__triet_string_free` (nội bộ, KHÔNG gọi được từ user syntax — chỉ Drop-glue) | arg0 | `[true]` | TIÊU THỤ | Đó chính là Drop-glue, không phải leak surface |
| `insert(map: HashMap<Integer,String>, key, value: String)` value arg | value | `[true,true,true]` (container+key+value) | **TIÊU THỤ** | **Đo (T0 bổ sung, PC — LÀNH, xem dưới)** |
| `push(vec: Vector<String>, elem: String)` elem arg | elem | `[true,true]` (container+elem) | **TIÊU THỤ** | **Đo (T0 bổ sung, PA/PB — LÀNH, xem dưới)** |

**✅ NỢ NÀY ĐÃ ĐÓNG (WO-1, 2026-07-20, `5a61e74`):** `is_empty(s)` ĐÃ đo riêng —
và suy luận cũ SAI NGƯỢC. Ghi chú cũ nói "code path y hệt `length`, độ tin cậy cao";
thực tế `length` ĐÃ được vá ở WO-ShimTempOwnership còn `is_empty` thì KHÔNG — cách nhau
95 dòng. Đo: `is_empty("hello")` và `is_empty(h.name)` đều **FREE=0 (rỉ câm)**, control
let-bound = 1, anh em `length` = 1. Vá bằng `c.push_owned(arg)` tại `triet-lower/src/
lib.rs:2639`. **🦷 Bài học: "cùng code path" KHÔNG suy ra "cùng trạng thái đã-vá" —
phải đo từng anh em, vì fix trước đó có thể chỉ chạm một đứa.**
Nullable/aggregate drop-glue), khác tầng so với `WO-INV-HeapNullable-Probe`. Tooth S3
của WO đó dùng idiom `let n = v.name; length(n)` để CÔ LẬP câu hỏi Nullable(Struct)
khỏi bug này — không né tránh, chỉ tách biến số.

---

#### T0 BỔ SUNG — nhóm TIÊU THỤ (`push`/`insert`), O+G chặn cứng trước khi cho code

**Câu hỏi:** temp vô danh (field-access hoặc literal) đưa thẳng vào một shim **TIÊU
THỤ** (`arg_consumes: true` — `push`/`insert`) có LÀNH (container tự free đúng 1 lần),
RỈ (không bao giờ vào container), hay DOUBLE-FREE?

**Tooth:** `crates/triet-driver/tests/heap_shim_consuming_temp_counting.rs` (8 test).

**Raw đo được** (`cargo test -p triet-driver --test heap_shim_consuming_temp_counting -- --test-threads=1 --nocapture`):
```
running 8 tests
test pa_control_push_field_let_bound ... PA-ctrl (push field let-bound): FREE=1 ok
test pa_push_field_access_inline ... PA (push field-access inline): FREE=1 ok
test pb_control_push_literal_let_bound ... PB-ctrl (push literal let-bound): FREE=1 ok
test pb_push_literal_inline ... PB (push literal inline): FREE=1 ok
test pc_control_insert_field_let_bound ... PC-ctrl (insert field let-bound): FREE=1 ok
test pc_insert_field_access_inline ... PC (insert field-access inline): FREE=1 ok
test poison_double_on_pa_control_proves_tooth_is_live ... PA-ctrl POISON(double): FREE=2 ok
test poison_leak_on_pa_control_proves_tooth_is_live ... PA-ctrl POISON(leak): FREE=0 ok
test result: ok. 8 passed; 0 failed
```

| Shape | Inline (temp vô danh) | Let-bound control |
|---|---|---|
| `push(v, h.name)` — field-access element | FREE=1 | FREE=1 (`pa_control`) |
| `push(v, "hello")` — literal element | FREE=1 | FREE=1 (`pb_control`) |
| `insert(m, 1, h.name)` — field-access value | FREE=1 | FREE=1 (`pc_control`) |

**Non-vacuous:** poison trên `PA-ctrl` (baseline sound = 1 free thật) — poison-leak → **0**;
poison-double → **2**. Cả hai đổi đúng hướng.

**⚖ KẾT LUẬN DỨT KHOÁT, THUẦN DỰA TRÊN SỐ: LÀNH.** Cả 3 shape × 2 biến thể (inline vs
let-bound) = 6 con số, TẤT CẢ bằng nhau (1). Không có khác biệt nào giữa temp vô danh
và local có tên — trái ngược hẳn với nhóm MƯỢN (nơi inline luôn thấp hơn let-bound
đúng đủ số args). Cơ chế: cho shim `arg_consumes:true`, M3 zero-on-consume
(`crates/triet-jit/src/mir_lower.rs:4717`) chạy dựa trên `args` của chính
`CallDispatch` đó — không phụ thuộc `Ctx::push_owned` (không cần MIR `Drop` nào tồn tại
để zero mới có tác dụng). Bản thân giá trị được COPY/MOVE vào bên trong container bởi
shim Rust (`__triet_vector_push`/`__triet_hashmap_insert`), nên quyền sở hữu chuyển
thật sự — bất kể ai đứng tên caller-side. **Nhóm tiêu thụ là CONTROL đúng nghĩa của WO
lõi: fix chỉ cần chạm nhóm MƯỢN, không đụng nhóm này.**

**Nợ còn treo (không đo trong T0 bổ sung này):** `HashMap<String,V>` với key TIÊU THỤ
(khác value vừa đo) — cùng lớp `arg_consumes:true` nhưng khác vị trí, độ tin cậy cao
theo cùng cơ chế (M3-zero không phân biệt vị trí arg) nhưng CHƯA đo riêng bằng số.

**CẤM đã sửa gì ở T0 này** — `triet-lower/src/lib.rs:2472-2479`, `emit_shim_call`, và
`Ctx::push_owned` GIỮ NGUYÊN, đúng lệnh "chỉ đo". "Poison ngược" (ép `push_owned` chạy
cho shim tiêu thụ để chứng minh double-free reproducible) là hạng mục của WO LÕI
(`WO-ShimTempOwnership`, chưa thi hành), không phải T0 đo này.

---

#### THI HÀNH — code đã vá (O+G duyệt phạm vi rộng, `emit_shim_call` chokepoint)

**Diff cốt lõi** (`crates/triet-lower/src/lib.rs`):
1. `emit_shim_call` — TRƯỚC khi emit `CallDispatch`, tra
   `triet_mir::builtin_shim_meta(shim_name)`; với mỗi arg: nếu
   `arg_consumes[i] == false` **hoặc KHÔNG có entry** (coi như mượn) →
   `c.push_owned(args[i])`. Nếu `arg_consumes[i] == true` → bỏ qua tuyệt đối.
2. Fast-path `length()` (`:2489-2517`, bypass hoàn toàn `emit_shim_call`) —
   thêm `c.push_owned(arg)` trong nhánh `MirType::String` (luôn là vị trí MƯỢN).

**Logic phân biệt mượn/tiêu thụ:** dựa 100% vào
`builtin_shim_meta().arg_consumes[i]` — `false`/thiếu entry = mượn = đăng ký
(`push_owned`, idempotent, an toàn khi gọi lại trên local đã có tên qua
`Stmt::Let`); `true` = tiêu thụ = cấm đăng ký (đã chuyển sở hữu sang
shim/container, đăng ký thêm ở đây là double-free tiềm tàng THEO LÝ THUYẾT —
xem phát hiện poison-ngược bên dưới, thực tế có một lớp phòng thủ RIÊNG).

**Bảng FREE-count đầy đủ (mọi shape, raw đã chạy):**

| Nhóm | Shape | Inline | Let-bound control |
|---|---|---|---|
| Fast-path | `length(h.name)` | 1 | 1 |
| Fast-path (đối chứng, xem WO-INV-HeapNullable-Probe S3) | `let n=v.name; length(n)` | — | 1 |
| Shim mượn | `concat("ab","cd")` | 3 | 3 |
| Shim mượn | `concat(h.name,"cd")` | 3 | 3 |
| Shim mượn | `contains(h.name,"ell")` (no meta entry) | 2 | 2 |
| Shim mượn | `eq(h.name,"world")` | 2 | 2 |
| Shim mượn (RĂNG MỚI) | `remove(m,"k")` key vô danh, map rỗng | 1 | 1 |
| Shim mượn (RĂNG MỚI) | `get(m,"k")` key vô danh, map rỗng | 1 | 1 |
| Control tiêu thụ | `push(v,h.name)` | 1 | 1 |
| Control tiêu thụ | `push(v,"hello")` | 1 | 1 |
| Control tiêu thụ | `insert(m,1,h.name)` | 1 | 1 |

Cả 8 shape mượn: inline == let-bound (đã vá xong gap). Cả 6 phép đo tiêu thụ
(3 shape × 2 biến thể): giữ nguyên 1, không đổi so với trước fix.

**RAW poison CHIỀU 1 — gỡ fix hoàn toàn** (`cp` snapshot md5 `1ce93a2ae7445ea85e64a7166a528a83` trước/sau, khôi phục khớp):
```
sha_concat_two_inline_literals ... FREE=1 (kỳ vọng post-fix 3) — FAILED, đúng con số rỉ cũ
shb_concat_field_plus_inline_literal ... FREE=1 (kỳ vọng 3) — FAILED
shc_contains_field_plus_inline_literal ... FREE=0 (kỳ vọng 2) — FAILED
shd_eq_field_plus_inline_literal ... FREE=0 (kỳ vọng 2) — FAILED
get_key_literal_inline ... FREE=0 (kỳ vọng 1) — FAILED
remove_key_literal_inline ... FREE=0 (kỳ vọng 1) — FAILED
-- mọi *_control (let-bound) VẪN XANH, không đổi (push_owned qua Stmt::Let
   không phụ thuộc fix này) --
```
Đặc hiệu tuyệt đối: gỡ fix → CHỈ inline-shape rơi lại đúng số rỉ đã đo ở T0, control không suy suyển.

**RAW poison CHIỀU 2 — POISON NGƯỢC (ép `push_owned` cho CẢ nhóm tiêu thụ)** —
KẾT QUẢ KHÔNG NHƯ DỰ ĐOÁN, xem mục 🔴 phán quyết chờ ngay dưới:
```
pa_control_push_field_let_bound ... FREE=1 (kỳ vọng double-free -> 2, ĐO ĐƯỢC 1)
pb_control_push_literal_let_bound ... FREE=1 (kỳ vọng 2, ĐO ĐƯỢC 1)
pc_control_insert_field_let_bound ... FREE=1 (kỳ vọng 2, ĐO ĐƯỢC 1)
-- cả 6 control tiêu thụ: KHÔNG double-free, giá trị + FREE-count như healthy --
```
Poison áp dụng: xóa nhánh `if !consumed { push_owned }` trong `emit_shim_call`,
thay bằng `for &arg in args.iter() { c.push_owned(arg); }` VÔ ĐIỀU KIỆN (bất
kể `arg_consumes`). `cp` snapshot trước (md5 `1ce93a2ae7445ea85e64a7166a528a83`) →
poison (md5 `07548a170e39418c5b09ce8a40b148db`) → khôi phục (md5 khớp lại
`1ce93a2ae7445ea85e64a7166a528a83`).

**⚖ CHỐT (O tự verify cả hai chiều, 2026-07-19) — poison ngược KHÔNG NỔ khi
M3 còn bật, nhưng M3 mới là lớp chịu lực thật:**

| M3 | phân biệt `!consumed` trong `emit_shim_call` | Kết quả đo |
|---|---|---|
| BẬT | ĐÚNG (như đã vá) | FREE=1, sound |
| BẬT | BỎ (poison ngược của D) | FREE=1 — **không nổ** |
| **TẮT** | ĐÚNG (giữ nguyên fix) | **`free(): double free detected in tcache 2`, SIGABRT** |

**M3 zero-on-consume** (`crates/triet-jit/src/mir_lower.rs:4717-4718`) mới là
lớp chịu lực thật chống double-free cho nhóm tiêu thụ — tắt nó đi, double-free
xảy ra NGAY CẢ KHI nhánh `!consumed` của `emit_shim_call` hoàn toàn đúng.
Nhánh đó **không phải khóa an toàn độc lập** — nó đúng về ngữ nghĩa (giữ
MIR-level ownership khớp thực tế) và sẽ TRỞ THÀNH lớp chịu lực nếu M3 từng bị
refactor/gỡ, nhưng hiện tại nó bị M3 CHE KHUẤT.

**Đáy vấn đề (đọc mã hai tầng, không phải hai cơ chế độc lập):** cả
`emit_shim_call`'s `push_owned` decision LẪN JIT's M3 zero decision đều đọc
**CÙNG MỘT** bảng `builtin_shim_meta().arg_consumes` — không phải
defense-in-depth (hai khóa độc lập, một cái đủ), mà là **MỘT quyết định áp
hai tầng**. Một entry khai láo phá **CẢ HAI** lớp cùng lúc — SPOF, xem mục
nợ riêng ngay dưới. Comment trung thực đã ghi tại `emit_shim_call`
(`crates/triet-lower/src/lib.rs`).

**Oracle cũ bị fix làm lộ leak khác — đã sửa, có bằng chứng pointer-identity:**
`hashmap_string_key_struct_value_remove_frees_key_and_value`
(`typed_hashmap_counting.rs:1124`) oracle `2 → 3`. O verify bằng probe
nhận-dạng-con-trỏ (dedup): `frees=3 | distinct=3 | dup=0` — KHÔNG double-free,
là leak thật của `remove`'s search-key literal, cùng lớp bug với
concat/contains/eq, giờ đã đóng theo chính cơ chế chokepoint. Comment mới tại
test giữ lịch sử + cảnh báo chống lùi.

**Răng mới cắm cho bán kính mở rộng:**
`crates/triet-driver/tests/heap_shim_hashmap_key_borrow_counting.rs` (8 test)
— `remove`-key và `get`-key trên `HashMap<String,Integer>` RỖNG (cô lập khỏi
resident-key), mỗi nhóm có inline + control let-bound + 2 poison. **`get`-key
đo ra ĐÚNG dự đoán (1/1), KHÔNG có bất ngờ** — không cần dừng lại báo cáo cho
riêng phần này.

**Full `cargo test --workspace` sau khi vá + oracle fix + răng mới:** 0 FAILED
(toàn bộ xanh). RAW GATE: `0·0·439·0 CLEAN`.

**✅ Nợ này ĐÃ ĐÓNG (WO-1, 2026-07-20, `5a61e74`):** `is_empty(s)` đã có con số
riêng — RỈ FREE=0, đã vá. Suy luận "độ tin cậy cao" ở đây SAI. Xem mục WO-1 trên.
`HashMap<String,V>` KEY vị trí trong `insert` (tiêu thụ, khác VALUE đã đo ở
`push`/`insert` value) chưa đo riêng bằng số — theo cùng lý luận M3-zero
(không phân biệt vị trí arg) dự đoán LÀNH nhưng CHƯA kiểm chứng.

**🔴 NỢ MỚI — `builtin_shim_meta().arg_consumes` là SPOF cho HAI TẦNG (G đặt
tên, 2026-07-19):** bảng `crates/triet-mir/src/lib.rs:1076` được đọc bởi CẢ
`emit_shim_call`'s `push_owned` decision (lowerer) LẪN M3 zero-on-consume
(`crates/triet-jit/src/mir_lower.rs:4717-4718`, JIT) — không phải hai lớp
độc lập canh cùng một bất biến, mà là MỘT quyết định (đúng/sai của một
entry) áp lên cả hai tầng cùng lúc. **Không có răng nào canh tính đúng đắn
của chính bảng này** — mọi tooth hiện có canh HÀNH VI (FREE-count) cho các
shim CỤ THỂ đã đo (concat/eq/contains/length/push/insert/remove-key/get-key),
không canh METADATA tổng quát.

Bãi mìn cụ thể nếu một entry tương lai khai láo:
- Khai **mượn** (`false`) nhưng shim thực **tiêu thụ** → cả `push_owned`
  (thiếu đăng ký sai hướng — thực ra đây là hướng ĐÚNG vì shim tự lo) VÀ M3
  (không zero) đều bỏ sót cùng kiểu — có thể LEAK hoặc miscompile tùy chi
  tiết shim, câm ở tầng giá trị.
- Khai **tiêu thụ** (`true`) nhưng shim thực **mượn** → `push_owned` bị bỏ
  qua sai (arg cần Drop lại không được đăng ký) VÀ M3 zero nhầm một giá trị
  caller còn cần dùng → LEAK (không ai free) hoặc corrupt use-after-zero,
  câm ở tầng giá trị, không crash ngay.
- `contains` (và bất kỳ shim tương lai nào) **không có entry** → rơi vào mặc
  định "mượn" ở CẢ HAI TẦNG (`emit_shim_call`'s `is_some_and` + M3's
  `if let Some(meta)`) — đúng cho `contains` hiện tại (đã đo, LÀNH), nhưng
  là mặc định NGẦM, không phải khai báo tường minh; một shim tiêu thụ tương
  lai quên đăng ký entry sẽ ÂM THẦM rơi vào "mượn" sai.

**Hướng tương lai (G nêu, chưa làm):** unit test quét TOÀN BỘ
`builtin_shim_meta` (mọi tên shim đã đăng ký trong cả `crates/triet-lower`
lẫn `crates/triet-jit`), đối chiếu từng `arg_consumes[i]` với chữ ký Rust
thật của shim (arg đó có thực sự được giữ lại/nhân bản bên trong container
hay không) — chứng minh không entry nào khai láo, thay vì tin tưởng từng
entry được viết đúng tay.

### ✅ ĐÓNG — **`WO-StructReturnRefuse` — POLICY GATE cho `Struct?` ở RETURN position** (O+G ký 2026-07-19, `e7aab8c`)
Anh em thứ HAI của guard `nullable_enum_return_unsupported` — cùng lỗ **"match exact, quên `Nullable`"** ở `Ctx::new` (`crates/triet-lower/src/lib.rs`, quyết định `ReturnShape`): `let is_struct_return = matches!(ret, MirType::Struct(_))` không khớp `Nullable(Struct(_))` → trượt xuống `_ => ReturnShape::Scalar`. Thành viên thứ TƯ của họ bug "match exact, quên `Nullable`": ① `Enum?` param copy-in (`ccb8db3`) · ② `Struct?` param bare-read (`7d59b7c`) · ③ `Enum?` return-shape → refuse (WO trước) · ④ **`Struct?` return-shape → refuse (WO này)**.

**T0 xác nhận (đo lại độc lập trên `768fc8e`, khớp ma trận O 100%):** không tầng nào khác refuse ca này — MIR verifier `is_lowerable_nullable_payload` (INV-HeapNullable) cho `Nullable(Struct(_))` qua **vô điều kiện** ở return-type position (không lọc Copy-ness), `INV-Enum-shape` chỉ khớp `MirType::Enum` trần, `find_refused_nullable_field` chỉ chạy trên struct-field/enum-payload chứ không phải return type. `triet-driver check` (parse→typecheck→lower→borrowck) lọt thẳng exit 0 trên cây pre-guard.

**Ma trận đo (`768fc8e`, chấm GIÁ TRỊ + exit, D đo lại từ đầu — khớp O 100%):**
| shape | exp | pre-guard đo được | post-guard |
|---|---|---|---|
| `return ~0`, arm không đọc field | 0 | **1** — câm, exit 0 | refuse exit 3 |
| `return ~+ P{40,2}`, arm không đọc field | 1 | 1 — *đúng ngẫu nhiên* | refuse exit 3 |
| `return ~+ P{40,2}`, đọc **một** field `v.x` | 40 | rác dạng địa chỉ (`94087559834896`), exit 0 | refuse exit 3 |
| `return ~+ P{40,2}`, đọc **hai** field `v.x+v.y` | 42 | SIGILL 132 (rác+rác vượt ngưỡng ADR-0044, hiệu ứng thứ cấp) | refuse exit 3 |
| struct **lồng** (`Outer{inner:Inner,b}`), null | 0 | câm/rác exit 0 | refuse exit 3 |
| struct **heap-bearing field** (`struct H{name:String}`), null | refuse | `check` lọt exit 0 → `run` **SIGABRT 134** `free(): invalid pointer` (mới đo, O đòi sau T0) | refuse exit 3 |
| control: `Struct` **trần** return (sret) | 42 | 42 ✅ | 42 ✅ giữ nguyên |
| control: `String?` return null/present | 0/1 | 0/1 ✅ | 0/1 ✅ giữ nguyên (`MirType::String` ≠ `MirType::Struct("String")`, guard tự nhiên không chạm) |
| control: `Integer?` return null | 0 | 0 ✅ | 0 ✅ giữ nguyên |
| control: `Struct?` param (428-436, hồi quy) | — | ✅ | ✅ giữ nguyên |
| control: `Struct?` local (mới, 445) | 0 | 0 ✅ | 0 ✅ giữ nguyên |

**Guard:** POLICY GATE (không phải soundness fix) tại `Ctx::new`, refuse **vô điều kiện** mọi `Nullable(Struct(_))` ở return — khác guard `Enum?` (chỉ unit-only, vì payload-bearing đã refuse sẵn ở nơi khác); không có refuse nào khác cho `Struct?` return nên không thu hẹp phạm vi. Comment tại chỗ khóa lại: khi ADR "Full SRET cho Nullable Aggregate" mở (gộp giải quyết luôn `Enum?` sibling), guard PHẢI bị gỡ và đường return phải probe lại từ đầu.

**9 fixture 437-445** (4 refuse chấm bằng exit 3 + message, 5 control chấm bằng GIÁ TRỊ).

**🩸 Poison đủ 3 loại (cp-snapshot lib.rs + 4 fixture, md5 mọi vòng, KHÔNG `git checkout`):**
1. **Gỡ guard hoàn toàn** → 437/438/439/440 rơi lại ĐÚNG hiện trạng đo ở T0 (437: exit 0/`1` câm · 438: exit 132 SIGILL · 439: exit 0/rác (`93832130708912`, khác con số T0 do layout bộ nhớ khác nhưng ĐÚNG failure-mode: câm/rác exit 0) · 440: exit 134 `free(): invalid pointer`, y hệt) — không chỉ "khác exit code", đúng hố cũ.
2. **Nới guard sang `Nullable(String)`** → control 442/443 (`String?` null/present) chuyển ĐỎ (refuse exit 3 thay vì 0/1) — chứng minh guard hiện tại căn đúng milimet, không lọt/không thừa.
3. **Harness-teeth per-fixture** — đổi `// ERROR: nullable struct return` → chuỗi bịa `BOGUS_POISON_STRING_43{7,8,9}`/`440` cho CẢ 4 fixture cùng lúc, chạy `cargo test -p triet-driver --test integration_tests` → cả 4 dòng `FAIL <tên>: expected error containing 'BOGUS_...', got: ...` xuất hiện (không có ca nào bị crash che khuất — guard refuse sạch bằng `Err`, không panic, nên chạy song song 4 poison không giết tiến trình) → khôi phục md5 khớp.

**🔴 Nợ mới ghi sổ (KHÔNG sửa, ngoài phạm vi WO — O yêu cầu đo lại sau khi D ghi nợ mô tả nhẹ hơn thực tế):**
1. **`is_lowerable_nullable_payload` (MIR verifier `INV-HeapNullable`, `crates/triet-mir/src/lib.rs:1636-1644`) cho qua `Nullable(Struct(_))` VÔ ĐIỀU KIỆN** ở return-type/local position — không lọc theo `is_copy`/heap content. Đo được: `struct H { name: String }`, `return ~0` cho `H?` → `check` lọt exit 0, `run` **SIGABRT 134** (`free(): invalid pointer`). Doc-comment ngay trên predicate tự khẳng định *"heap fields/payloads inside the aggregate stay refused via the scalar-only field/payload gate below"* — **số đo bác câu này cho vị trí return** (field/payload gate chỉ chạy trên struct-field/enum-payload, không chạy trên return type). Guard `WO-StructReturnRefuse` vừa đóng CHE ca này (refuse mọi `Nullable(Struct)` return, kể cả heap-bearing) nên KHÔNG còn đường sống chạm predicate sai này ở return position — nhưng bản thân predicate + doc-comment sai vẫn còn đó, treo cho ai chạm lại `INV-HeapNullable`.
2. **`key_marshal`** (`mir_lower.rs:1129`) — `Nullable(Struct)` dùng trực tiếp làm HashMap key chưa đo (kế thừa từ WO trước, chưa động tới).
Cùng LỚP triệu chứng với `Enum?` param (WO trước) nhưng **KHÁC CƠ CHẾ THẬT** — xác nhận đúng giả thuyết D dựng: `Struct` param **không có copy-in** (đọc-xuyên-con-trỏ cố ý, KHÔNG được đụng — G cấm), khác `Enum?` (có copy-in, chỉ thiếu unwrap). Root cause **KHÔNG** ở sentinel-compare `triet-lower` (MIR đúng, `_3 = _0 == NULL_SENTINEL` chuẩn) mà ở **JIT** `crates/triet-jit/src/mir_lower.rs` `load_place`'s bare-local branch (~1248-1283): local không có `struct_slots`/`enum_slots` entry (mọi param bị Lát-2 loop loại trừ tường minh `i < reserved_locals`, vì ABI pass-by-pointer) rơi `use_var(place.local)` = đọc **bit-pattern CON TRỎ** làm giá trị thay vì tag@0 — sentinel-compare không bao giờ đúng ⇒ nhánh `~0` chết trên MỌI `Struct?` param, bất kể null/present. Field-projection path (`walk_projections` + pointer-based branch) đã đúng sẵn — KHÔNG đụng.

**Fix (surgical, không đổi ABI):** tại bare-local read, nếu local không slot-backed và type là `Nullable(Struct(_))` (trừ `String`), `load(I64, ptr, 0)` thay vì `use_var` — dereference con trỏ để lấy tag, mirror 2 nhánh slot-backed phía trên.

**9 fixture 428-436** (chấm GIÁ TRỊ, không exit code — oracle-exit-code XANH trên cây hỏng ở 428/433/434):
| Fixture | Shape | EXPECT | Unfixed (đo riêng, không qua suite) | Fixed |
|---|---|---|---|---|
| 428 | null, arm không đọc field | 0 | exit 0, in `1` — **câm** | 0 ✓ |
| 429 | null, arm đọc field | -1 | **exit 132 SIGILL** (rác+rác vượt ngưỡng ADR-0044, hiệu ứng thứ cấp) | -1 ✓ |
| 430 | present, arm không đọc field (control chống lật dấu) | 1 | exit 0, `1` ✓ (trùng may) | 1 ✓ |
| 431 | present, đọc field 0+1 (control) | 42 | exit 0, `42` ✓ (trùng may) | 42 ✓ |
| 432 | nested struct, null | 0 | exit 132 SIGILL | 0 ✓ |
| 433 | 2 tầng gọi hàm (`main→outer→inner`), null | 0 | exit 0, `1` — câm | 0 ✓ |
| 434 | present+null cùng chương trình (`f(7)+f(3)`) | 10 | exit 0, `14` — câm | 10 ✓ |
| 435 | `Struct?` param field String (heap) | refuse | exit 4 "Struct? Drop without slot" (KHÔNG đổi — Drop-glue đọc `struct_slots` trực tiếp, không qua `load_place`) | refuse ✓ |
| 436 | control: `Struct` **trần** param (không hồi quy) | 42 | exit 0, `42` (không chạm code path) | 42 ✓ |

**🩸 Poison đủ 3 loại (cp-snapshot + md5 mọi vòng, KHÔNG `git checkout`):**
1. **Gỡ fix hoàn toàn** → nhóm null (428/429/432/433/434) hỏng (câm hoặc SIGILL — bảng trên); nhóm present+control (430/431/435/436) không đổi. Chạy full suite (một tiến trình) qua poison này SIGILL ngay ở 428 — **giết cả tiến trình, mọi fixture sau KHÔNG BAO GIỜ CHẠY** (đúng cảnh báo G) ⇒ phải đo riêng lẻ từng file, không dựa "suite đỏ".
2. **Twin-site mở rộng bắt `Nullable(Enum)`** → **VÔ HIỆU, không phải răng yếu**: `enum_slots.get()` (nhánh đứng TRƯỚC code mới trong cùng if-chain) đã có entry cho MỌI `Enum?` param nhờ copy-in của WO trước (`mir_lower.rs:2605`, chạy ở function-entry, trước khi `load_place` được gọi trong block body) ⇒ code mới **KHÔNG BAO GIỜ TỚI ĐƯỢC** với Enum — chứng minh bằng THỨ TỰ CODE, không chỉ thực nghiệm. Test literal theo đúng chữ WO cho kết quả rỗng (419-427 vẫn xanh nguyên) vì lý do cấu trúc, không phải vì thiếu đặc hiệu.
3. **Twin-site thay thế: mở rộng bắt bare `Struct` (không nullable)** → cũng VÔ HIỆU trên fixture 436, cùng lý do cấu trúc: `p.x + p.y` chỉ đọc field PROJECTED (qua `walk_projections`), không bao giờ bare-read `p`; và một bare-Struct Assign LUÔN bị ép qua nhánh aggregate-memcpy (`is_aggregate` match tên Struct non-String vô điều kiện, bất kể size) trước khi có thể chạm `load_place`. Khớp đúng bảng quét 4-call-site của T0: bare non-nullable Struct **không có đường nào** chạm nhánh bare-local qua pipeline thật.
4. **Harness-teeth BẮT BUỘC per-fixture** (EXPECT-flip → chạy full suite → `FAIL <tên>: expected X, got Y` → khôi phục, md5 khớp): làm đủ cho **cả 9 fixture** (8 EXPECT + 1 ERROR), tất cả ra đúng dòng FAIL, tất cả khôi phục md5 khớp. Xác nhận răng nằm ở tầng harness, không phải trang trí.

**Nghĩa vụ đo bắt buộc — RETURN position không đổi:** đo 4 repro (null/present × arm không-đọc/đọc-field) TRÊN CẢ 2 CÂY (fixed và unfixed) — kết quả **giống hệt cả 4 cặp** (`1`/`1`/`132`/`132`) ⇒ fix hoàn toàn tự chứa ở PARAM, không đụng RETURN.

**🔴 Nợ mới ghi sổ (KHÔNG sửa, ngoài phạm vi WO):**
1. **`Struct?` ở RETURN position — rác câm + SIGILL, KHÔNG có hàng rào nào**, pre-existing (O verify độc lập trên `de715ff` sạch, D verify lại trên cả fixed/unfixed — byte-identical). Repro: `function make() -> P? { return ~0; } / return ~+ P{x:40,y:2};` — arm không đọc field: cả null lẫn present đều in `1` (câm, đáp đúng lần lượt 0/1); arm đọc field (`v.x+v.y`): cả null lẫn present đều SIGILL 132 (khác PARAM case — ở RETURN, field-read hỏng cho **cả hai** null và present, không riêng null). Đối chiếu `Enum?` return đã refuse tường minh (`nullable_enum_return_unsupported`, fixture 427/403). Ứng viên mặt trận kế tiếp — CẦN đo sâu hơn trước khi lên WO (cơ chế RETURN chưa được xác định, chỉ mới đo hành vi bề mặt).
2. **`key_marshal`** (`mir_lower.rs:1129`) — `Nullable(Struct)` param dùng trực tiếp làm HashMap key chưa đo. Cơ chế `by_ptr` giống `copy_base_addr` (khả năng cao ĐÚNG theo cùng lý luận: param Variable vốn đã là con trỏ) nhưng CHƯA ĐO — treo nợ, không claim đóng.

**Gate cuối:** build 0 · test-fail 0 · fixtures 430 · clippy 0 · CLEAN.

### 🟠 NỢ TREO (ưu tiên THẤP — sau khi đóng `Struct?`) — **Lỗ hàng rào N1: `~0` bypass**
**D khui, O verify độc lập, G phân loại 2026-07-19 = POLICY-HOLE (KHÔNG phải UB).** Em sinh đôi của PA-A: dựng cửa chính, quên khóa cửa sau.

`let x: E? = ~0` với `E` **payload-bearing** **lọt hoàn toàn** hàng rào N1 (`check exit 0`, chạy được) — trong khi `let x: E? = ~+ E::V(42)` **refuse đúng** (exit 3, ADR-0065). Hai đường khởi tạo đi **code path khác nhau**; refuse guard chỉ gác một đường. Nghi can: `Stmt::Let` fast-path `is_null_expr` bypass thẳng sang `Statement::Const`, không qua chokepoint cắm guard; đường thứ hai là **implicit widening `E → E?`** (`let x: E? = plain;`) cũng không qua `~+`/`~0` construct. Lọt ở **cả local lẫn param**.

**Phân loại POLICY-HOLE dựa trên số đo:** D đo 8/8 round-trip đúng (payload `Integer`, dương/âm, nhiều variant, local+param), O kiểm chéo b1/b3 đúng — **0 crash, 0 giá trị sai**. Cơ chế: local/param luôn có slot đủ `layout.total_size` (16B disc+payload) bất kể unit-only hay payload-bearing, khác **struct FIELD** (nơi PA-A vá seed 8B).
**⚠️ KHÔNG tuyên "sound":** chưa đo payload **heap** (String/Vector — guard riêng ADR-0067), chưa đo đường qua struct/HashMap, chưa adversarial quanh `i64::MIN`.
**⚠️ Ghi nhận tác dụng phụ của `ccb8db3`:** fix copy-in **type-agnostic** ⇒ payload-bearing `E?` param init `~0` **đổi hành vi**: trước in `1` (sai), sau in `0` (đúng). Fix **không tạo UB, không refuse thêm, không phá gì** — nó xóa một câu trả lời sai trên shape lẽ ra không compile được. Nhưng hệ quả: **đường param payload-bearing giờ trông "chạy đúng" mà chưa qua audit ADR-0065** — quả bom nổ chậm về THIẾT KẾ. G lệnh ném xuống sổ nợ, vá sau `Struct?`.
**🚫 Nhắc lại phán quyết G (PA-A):** CẤM viện N1 như bằng chứng đã đóng một đường UB.

### ✅ ĐÓNG — **Heap-Nullable trong aggregate field/payload** (ADR-0076 SEALED, G ký 2026-06-29)
~~`struct S{x:String?}` / `enum Bag{Has(String?)}` refused (rào B8)~~ → **ĐÓNG SẬP — KỶ NGUYÊN NULLABLE KHÉP.** [ADR-0076](docs/decisions/0076-heap-nullable-aggregate-field.md) SEALED: heap-`T?` (String?/Vector?/HashMap?) ở field/payload nay construct + whole-move + drop sound. Lát đơn atomic `6327890`. **5 mũi:** ① gate `is_field_payload_lowerable` +`is_any_heap()` · ② field-layout sentinel @offset (String?=24B fat, Vector?/HashMap?=8B handle, null=NULL_SENTINEL) · ③ `collect_heap_leaves` arm `Nullable(heap)`→drop vô điều kiện reuse shim · ④ construct/widen store fat-ptr/sentinel @offset · ⑤ borrowck Move-classify + E2423 move-out refuse. **Cổ tức PA-3c:** conditional-drop = sentinel-no-op (ptr@offset ∈ {ptr→free, sentinel→no-op, 0→no-op}), 0 `brif` Cranelift. **O vồ double-free CASE B** (match-present-bind-move heap-aggregate → SIGABRT 134, borrowck im, MỚI do gate-lift) → D đóng STATIC tag-niche-tombstone (KHÔNG dynamic-drop-flag). O verify máu 3 tooth đỏ độc lập (Deinit-after-bind→134 ·3 biến thể · sinh-tử `is_copy(Nullable(heap))==false`→7 counting LEAK · drop-arm→leak). Fixtures FLIP 180/230/236/255→run + 311/312 present-bind + 310→E2423 + counting `heap_nullable_field_counting` 9/9. **Nợ defer giữ (ADR-0070):** partial-heap-field-move-out `let s=b.s` (đòi dynamic-drop-flag).

### ✅ ĐÓNG — **Path `.` → `::` + `use` + enum-variant ::** (ADR-0071 SEALED, G ký 2026-06-26)
~~Import `.`→`::`, dot-variant `Color.Red`, bare-variant~~ → **ĐÓNG SẬP TRỌN BỘ.** [ADR-0071](docs/decisions/0071-path-separator-and-module-import.md) SEALED, supersede ADR-0005 dot-path+Python-import. **AST pha lê: `::` tĩnh (path/type/enum-variant) · `.` động (field/method).**
- **Lát 1** (`4a7da96`): lexer `::`+`use`/giết import-from · `Item::Use{path,group}` (schema-first, brace-group) · resolver route 2-đường-cũ. 4 teeth (P-colon-token/longest-match/old-keyword/resolver).
- **Lát 2** (commit kế): `Color::Red`(+payload) via EnumLiteral + Pattern::EnumVariant{name:Some}. **Giết 3 cơ chế ngầm** (① pattern guess-hack ② expr in-scope-scan ③ 3 dot-hacks). **E1018 khai tử** (nguyên nhân chết). §2.A Variable=catch-all (đối xứng scalar). Bare un-qualified→E1002 mọi nơi; import-bound `use` chừa (env.lookup). Dọn dead `expr_resolutions` (rule #4, 21 caller). 5 teeth (O bóc tooth-vacuous P-catch-all + sharpen 293 scrutinee≠arm). Reading A (G phán giết-không-tha).

### ✅ ĐÓNG — **Partial-move & Struct-ZST** (ADR-0070 SEALED, G ký 2026-06-25)
~~`let v = hw.vga` field-level move-state~~ → **ĐÓNG SẬP.** [ADR-0070](docs/decisions/0070-partial-move-field-level-move-state.md) SEALED: borrow-checker mọc per-Place move-state (`partial_moves: BTreeMap<Local, BTreeSet<String>>`, union-merge monotone), capability ZST field sống đàng hoàng trong struct. **3 file:** `checker.rs` (Δ3 cho capability single-field, refuse heap E2423 giữ nguyên, 6 use-site invalidate), `lib.rs` (gate C allow-list capability + sizing 0B true-ZST per ADR-0069), `mir_lower.rs` (leaf-less non-copy struct → Drop no-op). **6 fixtures 279-284** (cap run→17 · use-after E2420 · whole-base E2420 · cfg-branch E2420 · heap-refused E2423 · mixed-struct offset run→105). O verify máu **5 teeth đỏ độc lập** (P-field-key · P-merge union→intersection · P-Δ3-heap no-panic · P-reread · Step3-JIT) + restore byte-identical. Schema §10 `kernel_main(hw)` destructure-move = canonical proof Hardware-Token nay CHẠY THẬT. **Nợ defer (No-Box):** partial-move field HEAP (đòi JIT dynamic drop-flag) · multi-level `hw.a.b` (conservative whole-base).

### ✅ ĐÓNG — **Capability Ł3** (ADR-0069 SEALED, G ký 2026-06-25)
~~Capability runtime Ł3 4-state~~ → **NIÊM PHONG.** Capability Ł3 — đã NIÊM PHONG bởi [ADR-0069](docs/decisions/0069-zst-capability-token-luk3.md) (synthesis ZST-token ngậm Ł3-Trit, CHÔN ADR-0016/0017/0018). Chi tiết: header trên.

**🏁 TRỤC B LÁT 2 NO-BOX (ADR-0067) ĐÓNG SẬP TRỌN BỘ — 2a+2b+2b+ hàn kín, không rỉ một byte (O verify 4 răng đỏ độc lập + G co-sign).** **⚰️ nhát 2b+ ENUM-IN-STRUCT FIELD ĐÓNG** — cầu nối `collect_heap_leaves`↔`emit_enum_drop_glue`: `struct Wrapper{msg:Msg, tag:Integer}` (Msg.Text(String)) construct+move+drop sound, FREE_COUNT==1, bịt lỗ enum-kẹt-giữa-struct. **2b+-A** `LeafKind{Heap,Enum}` — collect push enum-leaf KHÔNG đệ quy (payload tag-dependent) · **2b+-B** tách `emit_enum_drop_glue_at(base_addr)` address-based, slot-based cũ→wrapper mỏng (2b byte-identical) · **2b+-C** Drop dispatch `Enum→emit_enum_drop_glue_at(copy_base_addr)` + Deinit zero payload@abs+8 KHÔNG disc@abs+0 · **2b+-D** gate `is_nested_enum` song song `is_nested_struct`. **death-line #2 (lỗ THẬT D đào):** lib.rs merged-arm `Struct|Enum→struct_map` rơi `_=>8` → enum field under-size 8B (đáng 32B) → slot under-size + offset sai → SIGSEGV; vá = dời `enum_layouts` lên TRƯỚC struct-fixpoint + tách arm `Enum→enum_map` (enum-sizing độc lập struct → ordering sound). 4 răng O cắm poison độc lập đỏ: death-line#2→SIGABRT134 · R-leak→Drop-Unsupported · ⚔R-wrong-variant(cross-wire)→2 fail · R-double-free-move→count≠1. Fixtures 269/270 + counting `enum_in_struct_counting`. Nợ latent (G ký để nguyên, surgical): `Nullable(Enum)` sizing arm dùng struct_map→8 (correct-now vì gate refuse Nullable(heap); đồng bộ khi mở ADR-0062 §6 heap-nullable-enum). **⚰️ Trục B Lát 2 nhát 2b TOP-LEVEL ENUM-HEAP ĐÓNG.** ADR-0067 §2b — `enum Msg{Text(String),Code(Integer),Empty}` top-level: construct + move + drop sound, free CHỈ payload variant active (tag-switch runtime). **D recon-trước bắt gap payload-layout** (analog 1a STEP-4, O verify+rule IN-SCOPE): **2b-0a** enum payload size heap-aware (String→24, lib.rs:603 — M-1 struct-fixup không chạm enum) · **2b-0b** fat-store String payload vào enum_slot (analog STEP-4). **2b-1** gate gỡ refuse leaf (EnumLiteral+EnumVariant, refuse struct-transitive/Nullable-heap) · **2b-2** `emit_enum_drop_glue` N-arm brif tag-switch (free active variant only) · **2b-3** Deinit tombstone CHỈ ptr@8 KHÔNG disc@0 (disc=0 variant hợp lệ, khác Outcome). 4 răng đỏ: R-enum-leak(0)·R-enum-double-free-move(2)·⚔R-enum-wrong-variant(cross-wire Text/Buf→shim sai)·R-enum-cap(rác≠5). Fixtures 266/267/268 + counting `enum_heap_payload_counting`. Vector/HashMap payload sound sẵn (thin 8B). **⚰️ Trục B Lát 2 nhát 2a NESTED-FLAT ĐÓNG (chờ ký).** ADR-0067 §2a — mở khóa `struct Outer{inner:Inner}` (Inner chứa heap, non-recursive): construct + move + drop sound mọi tầng lồng. **2a-1** M-2 nới `is_nested_struct` (bare Struct layout-resolve ALLOW; **CHỈ Struct KHÔNG Enum** — enum-payload=2b; Nullable-heap/box GIỮ refuse) · **2a-2** `collect_heap_leaves` đệ quy compile-time (depth-64→JitError, DÙNG CHUNG Drop+Deinit đối xứng Sinh-Tử, trả flat abs-offset) · **2a-3** move tái dùng 1b/1c (0 dòng). 3 răng đỏ: R-leak-nested (Unsupported refuse) · R-double-free-nested (FREE==2) · R-recursive-creep (stack-overflow SIGABRT). Fixtures 263/264/265 + **257 FLIP** (negative→positive, LUẬT 3 O-signoff) + counting `struct_nested_heap_counting` + unit `collect_heap_leaves_recursive`. **🏁 Trục B Lát 1 (heap-in-struct FLAT) HOÀN TẤT — 1a+1b+1c+1d, B8 thủng cho heap-leaf field.** Nhát 1d LOCK & SEAL (chờ ký): niêm phong Vector/HashMap field + struct use-after-move — **0 dòng compiler** (mechanism type-generic 1a/1b/1c đã phủ), thuần fixtures 260/261/262 + counting teeth. 3 răng đỏ độc lập: R-leak-vec (cut is_vec → vec leak 0) · R-leak-hmap · **ISOLATION SCALPEL** (poison riêng is_vec → Mixed{Vector,String}: vec=0 leak, str=1 sống — dispatch per-field-type) · R-e2420. Counting test serialize Mutex (3 test chung counter, gate chạy song song). **Lát 1 đủ:** heap-leaf field (String/Vector/HashMap) construct + whole-move (arg 1b + assign 1c) + inline drop-glue (1a KCN-1) + tombstone + use-after-move E2420 = sound + locked. **Nhát 1c:** assign-move `let q=p` true-move (D1 ctx_is_copy + D2 Deinit ATOMIC), LOWER-ONLY. **Nhát 1b:** arg-move (A copy_base_addr unify + B to_zero ctx_is_copy + C Deinit walk). **Nhát 1a:** M-1 sizing + M-2 B8-relax + KCN-1 + STEP 4 fat-store. **Trước đó:** ADR-0065 §12.8/§12.7 Nullable Aggregate Trục A. ADR-0068 Box/recursive `&+` **G HOÃN** (chưa có allocator + iterative-drop — xem Nợ defer). **Nợ defer No-Box (chưa use-case):** payload-struct-chứa-heap (`enum{Rec(Wrapper)}` — collect đệ quy TRONG arm) · 2c true-recursive + box (ADR-0068, cần allocator + iterative-drop). **Nợ Lát 1.x:** partial-move `let s=p.name` / match-arm bind heap payload (blocked read-side gap String→Unknown) · field-reassign.

### 🟡 Sổ nợ Tech-Debt Hạ tầng — counting-test parallel isolation
Các test free-count (`nullable_map_heap_output_counting`, `vector_nullable_drop_counting`, …) dùng process-global `AtomicUsize` + no-mangle shim → flake hiếm dưới `cargo test --workspace` tải nặng (đo: `map_vector_output_freed_once` đỏ 1 lần, xanh 6+ lần isolation/release/re-run). Cần `--test-threads=1` hoặc subprocess isolation (hạ tầng N7 đã có cho một số). KHÔNG chặn nhát 1a (code orthogonal). Ghi nợ theo lệnh G.

---

## 🟢 BACKLOG MỞ

### 🔨 ĐANG MỞ — Field auto-deref qua `&0` reference (ADR-0084 DRAFT, chờ O verify + G ký)
`e.f` với `e : &0 T` / `&0 mutable T` (T=UserStruct) → auto-deref 1 tầng project field. Semantic đầy đủ khóa ở ADR-0084 §CỐT LÕI.
- [x] **Slice 1a — scalar field** (D code, chờ O verify): typecheck auto-deref scalar-only (`check_field_access` `is_scalar()` gate) + lowerer `Projection::Deref` (`place_result_type`+`lower_place`) + JIT `walk_projections` Deref + **Blocker-B vá** (`Statement::Borrow` slot-addr mọi struct/enum local, không chỉ String). Fixtures 381 (param) / 382 (block-local), EXPECT 30. Poison-2-tầng: gỡ typecheck→E1015 · revert Blocker-B→SIGSEGV 139.
- [x] **Slice 1b — sub-borrow aggregate/heap field qua `&0`** (D code, chờ O verify + G ký): `f` là Struct/String/Vector/HashMap → `&0 F` sub-borrow zero-copy (chainable, `(&0 Ngoai).trong.x`). 4 tầng — typecheck arm (`is_scalar`→value giữ 1a; `UserStruct`/`is_heap`→`Reference(BorrowReadOnly, field_ty)`) · lowerer `Expr::FieldAccess` rvalue (source có `Deref` + Struct/heap → emit `Statement::Borrow` thay `Assign`) · JIT `Statement::Borrow` projected-source (`walk_projections` offset + base-addr, số học địa chỉ KHÔNG copy) · borrowck WHOLE-OBJECT FALLBACK + REBORROW CHASE (Deref-source → anchor loan lên whole object; combo `(&0 h).name` chase qua temp về owner). Gate `0·0·381·0`. Fixtures 383 (heap-leaf param, EXPECT 5) / 384 (nested scalar, 7) / 385 (nested heap, 4) / 386 (dangling ERROR E2450) / 387 (move-while-borrowed ERROR E2440). Poison-đỏ: JIT revert projected-addr → 383/385 silent-wrong · borrowck bỏ chase → E2450/E2440 biến mất · typecheck gỡ arm → E1015. §AMEND ADR-0084 (DRAFT). Nghi ngờ báo O: whole-object false-conflict (2 field khác qua cùng ref) · Vector/HashMap-field cùng đường addr nhưng chưa có builtin đọc để fixture riêng (chỉ String-field test end-to-end).
- [ ] 🚩 **Borrowck lexical wart (NLL defer VÔ THỜI HẠN)** — borrow local còn sống tới return của owner = E2450 giả (ADR-0046 Case-D, fixtures 21/24). KHÔNG unsound, chỉ cồng kềnh (dùng block-scope/param để lách). NLL = hố đen, KHÔNG đụng `flush_all_for_return`.

### 🔨 ĐANG MỞ — Key-typed `HashMap<String,V>` (ADR-0080 + §AMEND-1, Author+O+G ký 2026-07-03)
Key mang content hash/eq + drop-obligation. ADR mới (BÁC amend ADR-0038 Comparable: `Ord ≠ Hash`; BÁC `Hashable` trait). Key ∈ {Integer, String} đóng băng. Slot `[key@key_stride | value@value_stride | state]`.
- [x] **KM-P1a BACKEND** (`c003a5f`) — Mũi A slot key_stride 24B fat · B `__triet_string_hash` FNV-1a + dispatch · D.1/D.2/D.5 key drop-obligation (§AMEND-1 out-param `is_update_out`/`key_out_ptr`, free registry-routed) · rehash key-stride. Hand-built MIR + counting, sleeping. O verify 5 teeth (#1 map-drop / #2 update / #3 remove / #5 content-hash / #7 rehash).
- [ ] **KM-P1b TYPECHECK+BORROWCK** (WO G-duyệt 2026-07-03, D đang code) — mở source `.tri` end-to-end:
  - **C1** typecheck generic-ize K∈{Integer,String} (`env.rs:342-391`): `hashmap_new<K,V>`/`insert<K,V>`/`remove<K,V>` + String-key overload get/contains/len. Seed K từ expected_type_stack.
  - **C2** REFUSE K∉{Integer,String} → mã lỗi MỚI `E10xx UnsupportedHashMapKey` (G: lấy E-code cao nhất rảnh, đập ở cửa typecheck, không defer-mềm).
  - **D3** borrowck insert-key = **Move** (String consume, Integer no-op) — mirror value-consume type-aware (`checker.rs:1224-1227`).
  - **D4** borrowck lookup-key = **borrow `&0 String`** (get/remove/contains KHÔNG consume) — tái dùng mô hình `&0` ADR-0079. **Lằn ranh sinh tử** (G): gõ lộn `consume=true` cho get = user double-free.
  - **Teeth (G ưu tiên #1):** ★SS `HashMap<String,String>` construct→insert→**remove→drop** — key-loop ∥ value-loop miễn nhiễm tombstone (state=2): gut key-loop→key leak · gut value-loop→value leak · double-free probe drop-lại-nấm-mồ→SIGABRT 134. + #4 insert-Move double-free · #6 lookup-borrow · #8 REFUSE E10xx · #9 Integer-key source compat.
- Defer Tầng-2+: `HashMap<_, UserStruct>` (P2 native) · get-clone/borrow heap value · get-borrow-mutable key · hash caching.

### ✅ ĐÓNG — Get-Borrow Heap Value from Container (ADR-0079 IMPLEMENTED/CLOSED, G ký 2026-07-01)
Đọc giá trị heap trong container **không đập hộp, không clone ngầm** — `get(&0 container, k) → (&0 V)?` zero-copy borrow (P1: V=String). Lấp lỗ read-side sau ADR-0077/0078. Mô hình loan = **whole-container** (G ký). Not-found → `(&0 V)?` nullable-borrow (PA-3c, NULL_SENTINEL). Slice A `a970540` + Slice B (gộp commit đóng).
- [x] **U1 typecheck/env** — `get(&0 Vector<String>,k)` + `get(&0 HashMap<Integer,String>,k) → (&0 String)?` overload (`env.rs`); value-position E1047 GIỮ.
- [x] **U2 borrowck builtin-return-borrow** — `returns_borrow_of: Some(0)` trên get_ref; PropagatedLoan source = whole container (source-tracing qua intermediate borrow `_tmp=&0 m`).
- [x] **U3 borrowck mutate-while-borrowed** — `mutates_arg: Some(0)` (remove/pop) + consume (insert/push); active loan → **E2440** (conflicting.dest thật).
- [x] **U4 lower/JIT** — `__triet_{hashmap,vector}_get_ref` trả con-trỏ-slot zero-copy (0 memcpy/alloc); not-found → NULL_SENTINEL `~0`. +F-d Copy-source skip-conflict.
- [x] **Teeth O (verify máu, poison cp-snapshot):** 5 borrowck (E2450·E2440 insert/remove/pop·negative) + source-level (content-read `length`→2/5·not-found `~0`·E2440 insert/remove). Fixtures 325/326 (route) + 327 (content-read guard).
- Defer: ~~generic V-overload~~ **ĐÓNG (Cụm A `37a0723`, V=Vector/HashMap Int+String-key)** · get-borrow-mutable (`&0 mutable`) **→ ADR-0081 FROZEN, đày Cụm D**. **key-typed: BACKEND `c003a5f` + source KM-P1b `381979e` ĐÓNG.**

### ✅ ĐÓNG — Read-side Cụm A: get-borrow generic-V + P0 String-key SIGSEGV (ADR-0079 §AMEND-1, G ký 2026-07-04, `37a0723`)
- [x] **A1 get-borrow generic-V** — env.rs 6 overload `get` V∈{Vector<Integer>,HashMap<Integer,Integer>} qua Vector<V>/HashMap<Integer,V>/HashMap<String,V> → `(&0 V)?`. JIT §AMEND-1 stride-conditional deref (get_ref): thin V (stride≤8)→`*cell`=body_ptr, fat V (String)→cell. Invariant `&0 V` bit-for-bit local≡get_ref.
- [x] **P0 String-key read SIGSEGV** (BÁO ĐỘNG ĐỎ, pre-existing từ ADR-0080) — get/get_ref/contains nhận `&0 HashMap` (Reference-wrapped) ≠ insert (owned); key_stride extraction chỉ bóc Nullable → default 8 → String-key 24B marshal by-value → SIGSEGV. VÁ: unwrap `MirType::Reference` trước match HashMap. Fixture 335 (contains+get+not-found →142).
- [x] **Fixtures 333-337** — 333 Int-key content-read(3) · 334 borrowck-track(E2440) · 335 P0 scalar String-key(142) · 336 String-key get_ref Vector(2) · 337 String-key get_ref HashMap(1). O verify poison→RED: POISON-1/POISON-P0/overload-break→E1041.
- Defer: get-borrow-mutable (A2 FROZEN, Cụm D) · V=Nullable get_ref (lowerer chưa match `&0 Nullable`) · V=UserStruct (P2 native-layout).

### 🏛️ Facade pattern (`public use` re-export) — Amend ADR-0005 §76 (G chốt 2026-06-29, ghi sổ)
Tách Logical Tree (API surface) khỏi Physical Tree (file layout) — lấy cái ngon của Rust
(`pub use` facade) mà GIỮ DNA explicit của Triết. **Bối cảnh:** Triết KHÔNG 1-1 Java
(ADR-0005 §17/§96/§155 đã tách bằng `module foo` declaration); auto-discovery (file=module
ngầm) bị BÁC — nghịch ADR-0005 (reject A1/A3/A5) + nghịch refuse-over-guess + phá hermetic build
(file `.bak`/generated đổi API). Việc THẬT còn thiếu = re-export, đã defer ở **ADR-0005 §76**
("Re-exports defer sang v0.3+") + parser đã chừa chỗ (`triet-parser/src/item.rs:78-85` refuse
`public use` với "not yet implemented", refuse-over-guess thay vì drop `public` silently).
- [ ] **Amend ADR-0005 §76**: bật `public use X` re-export — facade-file (`module.tri`/inline) chìa
      mặt tiền phẳng; dev xé file tự do trong bóng tối, chỉ `public use` mới phơi ra. Xây TRÊN
      `public`/`public(package)` hiện có — **MỘT cơ chế visibility, KHÔNG đẻ `export...from` song song** (coherence).
- [ ] **🌱 Ghim — Capability-aware facade (hạt giống đẳng cấp, O đề xuất + G bless):** facade chặn
      không chỉ visibility mà cả **năng lực** — re-export một type KHÔNG đồng nghĩa re-export quyền
      (capability token Ł3) để khởi tạo nó. Module system mang ngữ nghĩa Ł3 ở API boundary → vượt Rust/TS.
- **Timing (G chốt):** TUYỆT ĐỐI KHÔNG code lúc này. Chờ tới khi xây `std` hoặc Package Manager
  (nhu cầu facade mới rõ). Trade-off đã biết: mất zero-indirection navigation → cần tooling go-to-def bù.

### ✅ ĐÓNG — Bug-E: Outcome-param ABI mis-tag + `~->` early-return heap double-free (O+G ký 2026-07-03)
~~Outcome (`T~E`/`T?~E`) làm tham số hàm mis-tag `disc` (sai lặng lẽ) + `~->`
early-return double-free heap payload~~ → **ĐÓNG SẬP, 2 WO liên tiếp, cùng chiến
dịch.**

**WO1 — Outcome-param callee copy-in gap** (`ddb7841`): callee prologue cấp StackSlot
rỗng cho MỌI Outcome-typed local kể cả tham số (`mir_lower.rs:1453`), nhưng vòng bind
tham số chỉ có nhánh copy-in cho String/Enum, thiếu Outcome — con trỏ caller truyền
vào (đã đúng, `:2676`) bị bỏ xó, `_N.disc`/`_N.payload` đọc rác. Fix mirror đúng
khuôn String/Enum tại `:1644-1684`. Fixtures 328/329/330 (scalar/nullable/interleaved-
offset). O verify độc lập cp-snapshot (không stash — D vi phạm `feedback_teeth_never_git_checkout`
lần đầu, G ghi sổ đen cảnh cáo, kết quả tình cờ đúng).

**WO2 — `~->` early-return heap-payload double-free** (`818602c`), phát hiện khi O tự
mở rộng test ngoài phạm vi WO1 (`String~Integer` param) → SIGABRT 134. Cô lập: bug
KHÔNG liên quan tham số hàm, tái hiện chỉ với 1 local. 3 site cùng thiếu pattern
HP.4 (`copy_heap_outcome_payload`/`bind_heap_outcome_payload` + `Deinit`):
Site A (`lib.rs:~5163`, success-arm passthrough unwrap), Site B (`:~5023`, error-arm
bind `e`), root cause chung (`:~1947`, `Expr::OutcomeConstructor` heap-payload branch
— DÙNG CHUNG cho mọi `~+ v`/`~- e` trong ngôn ngữ, vô hại với literal/temp nhưng
double-free khi payload là named-local có drop-obligation — đúng tình huống Site B
tự tạo ra). Fixtures 331/332 (named-local, theo LUẬT `feedback_poison_must_be_red`).
O verify máu độc lập cả 3 site — poison từng site một, xác nhận đỏ đúng biến thể
(331 XOR 332), restore md5 khớp, gate CLEAN 326.

**Bài học:** silent-wrong-answer (WO1) được G xếp NẶNG hơn crash (WO2) — nhưng cả hai
đều bắt nguồn từ cùng một lỗ hổng lớp: thiếu hoàn thiện pattern copy-in/Deinit khi mở
rộng bề mặt Outcome sang ABI tham số + early-return. 0 fixture cũ từng chạm surface
này vì luôn dùng literal/temp (không có drop-obligation) — đúng LUẬT NAMED-LOCAL đã
ghi từ HM-P1b.

### 🔴 Chiến dịch CFG Tail-Expression — ưu tiên 1 (soundness)
Wire nốt ADR-0055: block tail-expr gánh giá trị cuối hàm.
return-scope đã khóa (ADR-0020 §3.8): `return` = early-exit + cọc-tiêu-mode, KHÔNG phải throw.

- [x] **ĐẬP TRƯỚC TIÊN (soundness):** 🔴 expr-body fat-struct return không route sret → **SIGILL 132**. `4d51faa`
      Free fn `f() -> Point = Point{...}` emit `Return(struct)` by-value thay vì ghi sret slot;
      block-body (`{ return ... }`) chạy đúng. Crash/soundness hole có sẵn, độc lập trait/nợ#2.
      *Soundness trước syntax (G 2026-06-17).* — ADR-0055 lát 1: helper SSOT `emit_struct_sret_copy`
      route tail-Return qua sret y hệt Stmt::Return; teeth 182/183/184 poison→SIGILL.
- [x] Wire tail-expr gánh giá trị cuối hàm → giảm `return` cuối thân (happy-path). `a0eff46`
      ADR-0055 lát 2 A-hẹp: phần lớn ĐÃ wire bởi ADR-0055+0056/0057/0058 (probe 20+ dạng:
      literal/expr/if/match/nested/while-tail/struct/heap-if/heap-match/outcome/nullable-widen
      đều chạy). Còn đúng MỘT bất đối xứng tail: `= ~0` báo lỗi trong khi `return ~0` chạy →
      mirror null-~0 special-case sang tail-path. Fixtures 185-188. Gap #2 (`{ ~0 }`/if-arm)
      đẩy Heap-Nullable (fail y hệt ở return/let, không phải tail-asymmetry).

### ✅ ĐÓNG — **Heap-Nullable (KỶ NGUYÊN NULLABLE KHÉP)** — ADR-0062/0065/0076 SEALED
`T?` cho `T` heap (String/Vector/HashMap/Struct/Enum) ĐÓNG TRỌN mọi vị trí:
top-level (ADR-0062 ptr-sentinel) + aggregate `Enum?`/`Struct?` (ADR-0065 niche/tag-prepend)
+ field/payload B8 (ADR-0076 `994afc8`). Gate `MirError::HeapNullableNotLowered` còn SỐNG
nhưng giờ chỉ refuse heap-nullable trong **recursive/Box** (ADR-0068 CẤM CỬA) — mọi vị trí
non-recursive đã lower.

**Ruling β (G ký 2026-06-18, vẫn hiệu lực):** gate ở LOWER (`Body::verify()`), KHÔNG typecheck
— stdlib khai heap-nullable làm API (`env.get`/`fs.read -> String?`); declaration vô hại
(stub `= ~0`), chỉ *compilation* mới gác. ✅ 5 mũi gốc (ptr-sentinel · widening · conditional-drop
· Elvis/match `~+/~0` · `?+>` map) landed qua ADR-0062; aggregate qua ADR-0065; field/payload
qua ADR-0076 (cổ tức PA-3c: conditional-drop = sentinel-no-op, 0 `brif`). Fixtures 189-237 + 311/312.
- [x] **Gap #2 — expected-type propagation (ADR-0072 🔒 SEALED 2026-06-27, 3-slice).**
  - [x] **Slice 1** `c9a46e6` — `lower_expr` thêm param `expected: Option<&MirType>`, 61 site=`None`, byte-identical (O verify MIR-diff rỗng toàn corpus).
  - [x] **Slice 2** `2c900fb` — leaf-consumer (`OutcomeConstructor`/`NullLiteral`) đọc `expected`; wire 4 nguồn (function-body/return/let-init/struct-field); đập 3 Bug-B redirect. **Mở `T?`-return scalar** (303 Integer?, 305 let-Integer?-trong-Outcome-fn). Fallback §2.5 chuyển-tiếp (gỡ ở Slice 3). O verify: gate 0·0·299·0 + byte-identical 297 cũ + 2 poison `OutcomeAlloc on non-Outcome` đỏ + defense-in-depth 2 guard.
  - [x] **Slice 3 (kết liễu)** — transparent forwarding `expected` xuống Block tail + If then/else + **13 arm-body** (8 WO + wc.body×4 + arm_for_present×1) + helper `lower_value_keyed_match`; gỡ sạch fallback §2.5; **nhổ `c.sig.return_type` khỏi input constructor** (chỉ còn 4 nguồn return-position hợp pháp: body-tail/return-non-sret/return-sret/Expr::Return); extract `emit_outcome_zero`. Mở `~+`/`~0` trong if/match/block-arm context≠sig (306/307/308) + **309 negative khóa luật untyped-ctor-bị-từ-chối** + 157 annotated. O verify máu: gate 0·0·303·0 + byte-identical 299/299 (worktree baseline) + 3 poison R-fwd đỏ + grep C sạch. Diagnostic tổng quát hóa (hết nói "~0 null" cho ~+/~-).
- [x] ✅ **WO-0075 — multi-level extraction `let x = h.inner.x` (ĐẠI PHẪU Nợ B — projection-path move-state, ADR-0070 §AMEND Phase 3, G ký 2026-06-29).** ADR `b74e03e` + C1 `3826924` + C2 `bd614f3` (push main). Nâng `partial_moves` từ field-name → **projection-path** (`Set<String>`→`Set<Vec<String>>`), mở móng Capability Ł3 nested. **2 commit tách (G mandate git-sạch):** C1 = vá **lỗ fixpoint-hole CÓ SẴN** (fixpoint check chỉ so var_states+active_loans, KHÔNG so partial_moves → partial-move không set base→Moved → delta âm thầm vứt → UAM lọt qua back-edge trong loop), XANH ĐỘC LẬP trước; C2 = feature projection-path. **Cascade:** `single_field`→`projection_path() -> Option<Vec<String>>`; helper `prefix_conflict(p,m)=p[..min]==m[..min]` (exact/ancestor/descendant/whole-base DEAD, sibling LIVE); allow-arm ghi path đầy đủ; §F sub-path reassign (`h.inner=fresh` sau move `h.inner.x`) KHÓA bằng **E2424 SubPathReassignUnsupported mới** (ADR-0027 format). JIT Site-G nới gate `[Field]`→`all-Field` (`walk_projections` trả abs-offset mọi depth); Lower Site-H **no-op** (`place_result_type` đã loop multi-level — MIR probe pin). **9 tooth, O verify máu độc lập (cp-snapshot, control-biến, KHÔNG git checkout):** G🩸 fixpoint-loop (gỡ partial_moves khỏi fixpoint→UAM lọt `got:[]`, structural-guard back-edge thật); F⚔ merge-union (gỡ union→join quên `got:[]`, structural-guard diamond ≥2 preds); B+D prefix exact-only poison → B(ancestor)+D(whole-base) ĐỎ, **A/C/E/F XANH=control đặc hiệu**; H runtime (revert JIT gate→`left:2 right:1` double-free); E2424-lock (skip diagnostic→`got:[]`). **LUẬT THÉP #3:** `cannot_move_multilevel_field_out` retarget→`cannot_move_non_field_projection_out` (Payload→E2423, ca CÒN refuse) — negative coverage BẢO TỒN dời mục tiêu (hành vi cũ bị ADR G-ký lật); fixtures 298/302 `*_e2423`→`*_run` real-allocator soundness witness (double-free→SIGABRT, clean-run trong corpus 303). Gate `0·0·303·0` (counting binary riêng). Backward-compat 100% (Phase 2/2b regression XANH). **NỢ B ĐÓNG TRỌN. Còn defer: non-Field projection (Index/Deref) · sub-path reassign mở · ADR-0068 recursive CẤM.**
- [x] ✅ **WO-0074 — enum-field move-out `let e = h.msg` (Heap-aggregate Phase 3 — Nợ A, G ký 2026-06-29).** `e0b1ed7` (push main). Single-level move-out của heap-carrying enum field ra khỏi struct base (construct + base-Drop enum-in-struct đã có từ ADR-0067; WO này chỉ mở đường move-out). **3 site đối xứng heap-struct campaign:** Site-1 `lower/lib.rs` thêm `matches!(field_ty, Enum(_))` vào gate type-slot → dest mang Enum → JIT cấp enum-slot (không thì Unknown→no-slot→SIGSEGV); Site-2 `borrowck/checker.rs` thêm `Enum(_)` vào allow-guard — **`partial_moves` key = field-name đơn ("msg"), data-structure KHÔNG đổi** (Nullable/Outcome vẫn refused); Site-3 `jit/mir_lower.rs` arm `Enum(_) => zero payload-ptr@field_off+8` (disc giữ, đối xứng leaf-Enum `abs+8`). **5 tooth, O verify máu độc lập (cp-snapshot, KHÔNG git checkout):** T1 borrowck (gỡ Site-2 → E2423 `obj.msg`/`Msg`); T2 double-free (gỡ Site-3 → FREE `left:2 right:1`); T3 leak (phủ định kép `==1`); T4 cap+count đồng thời (`STR_CAP==5 && STR_FREES==1`, assertion-guard byte-copy); **T5 ⚔ SIGSEGV in-suite** (gỡ Site-1 → child `wait_status(139)`=signal 11, crash CÔ LẬP subprocess, test-runner sống). File mới `enum_field_moveout_counting.rs` + `enum_field_moveout_subprocess.rs` + tooth-1 inline `checker.rs`. Gate `0·0·303·0` (counting/subprocess là test-binary riêng → KHÔNG nhúc nhích corpus 303; WO §1 "≥305" của O là giả định sai, ghi nhận). **NỢ B (multi-level `h.inner.x`) ĐÓNG BĂNG — đụng borrowck core `partial_moves` key, cần ADR-0070 amend riêng.**
- [x] ✅ **WO-0073 — heap-nullable-RETURN drop-glue (cờ đỏ ADR-0072 §6 NHỔ TẬN GỐC, G ký 2026-06-29).** `3738eb5` (push main). File `heap_nullable_return_present_counting.rs` — **7 cell counting-tooth** cho `~+ <heap>`-present return (String?/Vector?/HashMap?), 2 shape: **expr-body** (A/B/C/D: `= ~+ x` + match-consume) + **named-local explicit-return** (E/F/G: `{let s; return ~+ s;}`). O verify máu độc lập (cp-snapshot, KHÔNG git checkout): **leak-tooth** (elide drop-glue) → 7/7 RED FREE→0; **double-free-tooth** (gỡ M4 1982) → E/F/G RED FREE→2, A/B/C/D INERT(1). **Sự thật kiến trúc** (G bắt sửa doc 2 vòng): expr-body = lowerer **escape-by-omission** (callee KHÔNG emit Drop; O đo: bỏ guard ptr==0 + M4 off → tổng free-call==1) → double-free bất khả → M4-tooth INERT; named-local = `flush_all_for_return` emit Drop(s), **M4 load-bearing**. Cả leak lẫn double-free đóng. **Bài học O: verify-don't-trust cắt cả WO của chính O** (spec double-free-tooth ban đầu sai — tưởng M4 gác expr-body; D bắt, nới scope +3 cell named-local).

### 🟣 ADR-0065 Nullable Aggregate (`Enum?` & `Struct?`) — 🔒 LOCKED, 2 lát
Bất biến hợp nhất: `tag_cell == i64::MIN ⟺ null`. Rào **B8 §4**: aggregate-nullable CHỈ
chứa Copy field/payload — KHÔNG drop-glue/alloc/free. Value-model i64 KHÔNG đụng.

- [x] **Lát 1 — `Enum?` (disc-sentinel niche, 0 byte).** `1748510` (feat) + `e9bd3e0` (§9.1).
      disc@0 == i64::MIN = null (discriminant thật ∈ {0,1,2,…}); widening no-op. 5 delta:
      A gate(triet-mir 1399) · B slot-alloc(triet-jit) · C walk_proj unwrap · E result-retype
      (lower, idiom ADR-0056) · F `~0` store disc@0. Fixtures 225-230 (present payload-less/
      payload, ~0 null, Elvis, widening, B8 heap refuse). Poison E/B→SIGSEGV139, F→Trap132.
- [x] **Lát 2 — `Struct?` (tag 8B, Phương án A, β).** slot `{tag@0:i64, fields@8…}`,
      total = struct.total_size + 8. tag@0 == i64::MIN = null / +1 = present. **6 delta:**
      Delta 0 lowerer (Struct→Struct? widening → fresh local + Assign, KHÔNG retype in-place —
      §9.2, recon-miss của O vá in-scope) · 1 gate += `Struct(_)` · 2 slot-alloc +8 (skip
      sret/param/String) · 3 walk +8 (helper `nullable_struct_base_offset`) · 4a widening
      tag=1 + copy→+8 · 4b **β** whole-slot N+8 tag-first (`T?→T?` propagate, KHÔNG refuse).
      Lệch-lệnh chuẩn thuận: `is_aggregate` + slot-loop skip `Struct("String")` (borrowck builder
      type String-local thành `Struct("String")` slot-less → tránh deref SIGSEGV). Fixtures
      231-237; **237 = teeth tag-store** (reassign-widen-over-null, slot tái-dùng MIN). O verify
      máu P1-P5 RED: P1 walk→231/234 sai · P2 4a-1word→SIGILL · P3 tag-store→237 đỏ (231 tươi
      KHÔNG bắt) · P4 4b-tag→234/235 đỏ · P5 B8 gate→236+180. Gate `0·0·232·0`.

### 🟢 Perf — ADR-0044 §iii (không chặn)
- [ ] **D1 Codegen opt range-check 1-instruction:** `(val−MIN) >ᵤ 2M` unsigned-sub trick + fallback `bor` gộp 2 icmp. Cắt nửa instruction mỗi Add/Sub.
- [ ] **D2 Constant folding pass:** toán hạng const in-range → tính compile-time, bỏ trap block.

### Khác
- [ ] **D2 HashMap reject-MIN** (ADR-0043 Q6): `insert` reject `i64::MIN` — GIỮ defense-in-depth.
- [x] **gate.sh giòn — exit 1 giả khi clippy=0** (G ghi sổ 2026-06-18) — VÁ `9263501` (chore(infra) Kỷ-Luật-Gate: exit code trung thực + counting-test Mutex isolation). gate.sh nay exit 0 ⟺ cây sạch; vận hành ổn định qua WO-0073/74/75 (`0·0·303·0`). *(Commit gốc mang nhãn `[PENDING O-VERIFY]` — teeth tự-kiểm gate-bắt-đỏ là việc O, ngoài scope WO này.)*

---

## 🔒 PHONG ẤN Nhóm E — YAGNI (G defer 2026-06-10)

Mở khi có tiền đề (value-model thoát single-i64 / producer thật). KHÔNG build tạm.

- Native struct multi-field layout — cần value-model + ADR byte-size mapping + fixture Trit/Tryte-in-struct. Spike: `spec/plans/phase10-native-struct-layout.md`.
- Packed Outcome ABI — đi kèm Native.
- Multi-value return (>1 value) — cần producer thật (Outcome/tuple). Spike: `spec/plans/phase11-c5-multivalue-return.md`.
- `&+` / `&-` borrow forms — phong ấn (ADR-0059).

---

## ✅ ĐÃ ĐÓNG — tóm tắt (chi tiết: [`docs/TODO-ARCHIVE.md`](docs/TODO-ARCHIVE.md) + git + ADR)

- **Phase 4 Aggregate:** struct/enum/String/Vector/HashMap (ADR-0042 B7-lift · 0043 HashMap · 0060 nested `a.b.c`).
- **Phase 5 Bậc C borrow:** ADR-0044 trap-overflow · 0045/0046 `&0` borrow + return-elision · 0047 read-ops · 0048 mutable-borrow · 0059 `&0` heap.
- **Bậc D Fat-Pointer ABI:** ADR-0049 (param/return fat-String sret, slot = chân lý duy nhất).
- **Outcome `T~E`:** ADR-0050 MirType · 0051 borrowck-merge · 0052 producer · 0055→0058 CFG/heap sret.
- **Trait Tier 1:** ADR-0061 (`594abd9`) — dispatch + Comparable (ADR-0038) + match-on-Trit.
- **Nullable `?+>` scalar:** ADR-0039 (`73532b4`) — map/flatMap, `?->` → E1046.
- **Chiến dịch Trả Nợ** (2026-06-09→10): A (3 bom) · B1 type-system (ADR-0050) · B2 borrowck-merge (ADR-0051) · C1/C2/C6 feature-gaps · OP Outcome-producer.
- **Chiến dịch Cleanup "Đại Hốt Xà Bần"** (2026-06-17→18): LoweringInput refactor · fat-return trait sret · heap-nullable LOWER-gate · return-scope ADR-0020 §3.8.
