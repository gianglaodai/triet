---
name: campaign_adr0088_lane_a_nested_nullable
description: "✅ ĐÓNG 2026-07-27 — ADR-0088 Lane A: hàng rào E1055 nested-nullable T?? 2 tầng / 2 lớp nguồn + 9 răng cưa (511-519). Giết ICE E1190 + message MIR sai hướng. Phát hiện lõi: refuse cũ sống ký sinh vào allow-list với 0 răng canh = SPOF câm chờ Lane B. Lane B hoãn vô thời hạn. d9b659a, gate 0·clean·0·511·0."
metadata:
  node_type: memory
  type: project
---

## ✅ ĐÓNG — `d9b659a` (D) + docs (O co-sign). O✅/G✅/Giang✅ 2026-07-27

Gate `0·clean·0·511·0 CLEAN`. Fixtures 502 → **511**.

## 🎯 Recon lật khung — thân bài ADR-0088 CHỈ phủ `get`-family

Giang chốt mở ADR-0088. O recon **20 probe trên binary release** (rebuild trước,
luật 12) → **BÁC khung "double-nullable = vấn đề của `get`"**. `T??` viết TRỰC
TIẾP chưa ai đo:

- **KHÔNG có UB** — mọi đường fail-closed (exit 2/3, 0 ca exit-0-sai, 0 signal 132/134/139).
- **Nhưng chẩn đoán VỠ: 5 mã lỗi cho CÙNG một khái niệm.**
  - 🔴 `struct S{v:Integer??}`+match → **E1190 = mã ICE** "please report compiler bug"
    cho input user hợp lệ cú pháp ⇒ vi phạm taxonomy ADR-0086.
  - ⚠️ local/param/return/pop/pop_front/remove/`!!` → message MIR nói *"heap-nullable…
    ADR-0065 §4 B8 Struct?/Enum? Copy-only… [Fix] Remove the heap field"* — `Integer??`
    KHÔNG heap, KHÔNG struct ⇒ sai hướng hoàn toàn, vi phạm ADR-0027 machine-fixable.
  - ⚠️ enum payload → E1141 (sai nguyên nhân) · `Integer??~E` → E0001 parse.

## 🎯 PHÁT HIỆN LỚN NHẤT — 0 RĂNG CANH, SPOF câm chờ Lane B

Refuse cũ sống ký sinh vào **allow-list** `is_lowerable_nullable_payload`
(`triet-mir:1796`: scalar/heap/Enum/Struct/Reference). `Nullable(_)` **không có tên
trong list** ⇒ rơi ra ngoài ⇒ refuse **by default** = **may mắn cấu trúc**, không
phải hàng rào chủ động. Và grep `??` trong **dòng code** của toàn corpus = **0 hit**.

⇒ Việc ĐẦU TIÊN Lane B sẽ làm là **thêm arm `Nullable` vào chính allow-list đó** —
lúc ấy 7 đường lọt JIT cùng lúc, **câm, gate vẫn xanh**. Cùng hình dạng SPOF-một-lớp
đã bịt ở WO-SPOF-1. **Đây mới là lý do Lane A đáng làm, không phải "dọn message".**

## HAI LỚP NGUỒN (lý do WO không thể "một điểm chạm")

| Lớp | Nơi sinh `Nullable(Nullable(_))` | Phủ |
|---|---|---|
| **A khai báo** | `resolve_type` **2 BẢN SAO**: `check.rs:1365` + `check_resolved.rs:597` | local·param·return·struct field·enum payload·`!!` |
| **B suy diễn** | `check_call` sau `return_type.substitute(&sub_map)` (`check/exprs.rs`) | `pop`/`pop_front`/`remove` trên `<T?>` |

`let x = pop(v)` **KHÔNG có annotation** — shape chỉ tồn tại SAU substitution.
🦷 Lớp A có **2 bản sao** = cùng hình dạng `is_fat_ret` 3-bản-sao (ADR-0065 §14.7):
đụng một bản PHẢI grep bản kia.

## ⚖ HAI LẦN BÁC LỆNH — cả hai đúng

**D bác vị trí WO của O:** O gợi ý `env.rs:374/394/506`; D chỉ ra nơi đó `T` còn là
`TypeParameter` **trừu tượng**, không biết bind ra gì tại call-site ⇒ chuyển
`check_call`. O poison đặc hiệu xác nhận D đặt đúng chỗ. Guard **không name-gate**.

**O bác giao thức verify của G — G rút:** G lệnh *"poison allow-list → 7 fixture đỏ"*.
O đo TRƯỚC và chứng minh sẽ **KHÔNG đỏ** (typecheck chặn trên bọc kín tầng MIR) ⇒
giao thức 1-mũi đẩy người verify vào bẫy "poison không đỏ" rồi buộc **bịa mũi giả cho
nổ để qua cửa**. G phê chuẩn **2 mũi độc lập**. 🔑 Đây đúng vết O từng tự ăn (luật 16
"tiêu chí nghiệm thu cũng là giả định") — lần này bắt được **trước khi giao việc**.

## 🩸 O VERIFY MÁU — 3 mũi + đặc hiệu HAI CHIỀU

| Mũi | Poison | Đo được |
|---|---|---|
| 1a | tắt Lớp A `check.rs:1374` | **6 đỏ** 511·512·513·517·518·519 · 514-516 **xanh** |
| 1b | tắt Lớp B `exprs.rs:1054` | **3 đỏ** 514·515·516 · 6 kia **xanh** |
| 2 | nới allow-list MIR | **chỉ unit test đỏ · 0 fixture** |

**6+3=9 ⇒ hai lớp TÁCH BẠCH**, không lớp nào đội lốt lớp kia. Dưới mũi 1a,
**517/518 lộ lại ĐÚNG ICE cũ** (`unsupported match pattern` / `requires an expected
type`) ⇒ guard mới chính là thứ giết E1190/E1141, không phải trùng hợp.
**Răng ở TẦNG HARNESS** (luật 15): đổi `// ERROR:` 514→`E9999` → ra dòng
`FAIL expected 'E9999', got E1055`. Restore `cp`+md5 khớp 4 file, **0 git checkout**;
`git diff` vs commit D = **rỗng**.

## ⚔ BẤT BIẾN MỚI — message runtime CẤM chứa mã lỗi tầng khác

D tự phát hiện + tự sửa: message MIR nháp chứa chuỗi `"(E1055)"` → harness so bằng
`.contains(code)` nên poison gỡ guard typecheck vẫn **xanh giả** (tầng MIR nổ, message
tình cờ chứa "E1055"). D bỏ chuỗi mã khỏi message runtime, chạy lại bằng harness thật.
🦷 **Nếu message một tầng chứa mã lỗi của tầng khác, mọi poison xuyên tầng bị vô hiệu
hoá CÂM.** D không chọn đường bịa cho qua cửa — 0 vết bịa.

## Control chống over-refuse (giữ mãi)

struct `Integer?` MỘT tầng → **16** · `HashMap<K,Integer?>` insert-store → **5** ·
flatMap 175/212/213 xanh (`exprs.rs:361-364` giữ body nullable, **không bao giờ sinh
`U??`**) · 465/466/467 **giữ E1051** không bị E1055 cướp · 468 control dương.

**Ranh giới E-code:** `E1051` = `get`/`get_ref` (không đụng) · **`E1055`
`NestedNullableUnsupported`** = nested `T??` mọi vị trí khác.

## 🩸 Vết O

Sót **hạng mục ④ Documentation Integrity** của G khi soạn WO (không giao D) → O tự
thi hành docs, không gọi D lại. Bài học: WO phải soát ngược từng điều kiện G ban ra,
không soạn từ trí nhớ.

## ⏸️ LANE B — HOÃN VÔ THỜI HẠN

Thiết kế `T??` thật = repr 3-trạng-thái (sentinel `i64::MIN` hiện chỉ có **1 bit
null**, không đủ chỗ cho 2 tầng độc lập) + ABI + match ergonomics + parser `??~`.
G chốt: chưa có use-case đòi phân biệt *"key không tồn tại"* vs *"giá trị lưu là
null"* thì là **xây cầu khi chưa có sông**. Workaround: sentinel value / wrapper
Struct có cờ `present`. **9 răng cưa 511-519 sẽ nổ đỏ nếu Lane B mở allow-list mà
chưa làm đủ — đó chính là mục đích chúng tồn tại.**

## §AMEND-1 §88A.4 — đính chính thân bài ADR-0088

Câu *"Guard KHÔNG chặn `contains`"* mô tả **hành vi không tồn tại**. Đo thật:
`contains(m,1)` với `V=Integer?` → **E1041 NoMatchingOverload** (overload table không
khai `V` generic) — không phải được cho qua. Ai tưởng `contains` là workaround hợp lệ
cho `HashMap<K,V?>` sẽ trượt. **Nhãn tài liệu sai, không phải lỗ.**

[[campaign_typed_collections]] [[campaign_forgot_nullable_sweep]] [[mentor_o_persona]] [[colleague_d_persona]] [[feedback_poison_must_be_red]]
